use crate::{MailSearchIndex, StorageError};
use cove_core::{
    Account, CalendarEvent, MailFolder, ReminderTask, SearchResult, SyncJob, SyncStatus,
};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone)]
pub struct Storage {
    pool: SqlitePool,
    search: MailSearchIndex,
}

impl Storage {
    pub async fn connect(
        db_path: &Path,
        search_index_dir: &Path,
        sqlcipher_key: Option<&str>,
    ) -> Result<Self, StorageError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db_url = format!("sqlite://{}", db_path.to_string_lossy());
        let options = SqliteConnectOptions::from_str(&db_url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(30))
            .pragma("temp_store", "memory")
            .pragma("mmap_size", "30000000000")
            .pragma("cache_size", "-20000");

        let pool = SqlitePoolOptions::new()
            .max_connections(50)
            .connect_with(options)
            .await?;

        if let Some(key) = sqlcipher_key {
            sqlx::query("PRAGMA key = ?1")
                .bind(key)
                .execute(&pool)
                .await?;
        }

        sqlx::migrate!("./migrations").run(&pool).await?;

        let search = MailSearchIndex::open_or_create(search_index_dir)?;

        Ok(Self { pool, search })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn upsert_account(&self, account: &Account) -> Result<(), StorageError> {
        let provider = serde_json::to_string(&account.provider)?;
        let protocols = serde_json::to_string(&account.protocols)?;
        let oauth = match &account.oauth_profile {
            Some(profile) => Some(serde_json::to_string(profile)?),
            None => None,
        };

        sqlx::query(
            r#"
            INSERT INTO accounts (
              id, provider, protocols_json, display_name, email_address,
              oauth_profile_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
              provider = excluded.provider,
              protocols_json = excluded.protocols_json,
              display_name = excluded.display_name,
              email_address = excluded.email_address,
              oauth_profile_json = excluded.oauth_profile_json,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(account.id.to_string())
        .bind(provider)
        .bind(protocols)
        .bind(&account.display_name)
        .bind(&account.email_address)
        .bind(oauth)
        .bind(account.created_at.to_rfc3339())
        .bind(account.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_accounts(&self) -> Result<Vec<Account>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT id, provider, protocols_json, display_name, email_address,
                   oauth_profile_json, created_at, updated_at
            FROM accounts
            ORDER BY email_address
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_account).collect()
    }

    pub async fn delete_account(&self, account_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM accounts WHERE id = ?1")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM account_protocol_settings WHERE account_id = ?1")
            .bind(account_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn upsert_account_protocol_settings(
        &self,
        account_id: Uuid,
        settings: &serde_json::Value,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO account_protocol_settings (account_id, settings_json, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(account_id) DO UPDATE SET
              settings_json = excluded.settings_json,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(account_id.to_string())
        .bind(serde_json::to_string(settings)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn account_protocol_settings(
        &self,
        account_id: Uuid,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        let row = sqlx::query(
            r#"
            SELECT settings_json
            FROM account_protocol_settings
            WHERE account_id = ?1
            "#,
        )
        .bind(account_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let raw: String = row.try_get("settings_json")?;
                Ok(Some(parse_json(
                    &raw,
                    "account_protocol_settings.settings_json",
                )?))
            }
            None => Ok(None),
        }
    }

    pub async fn upsert_mail_message(
        &self,
        message: &cove_core::MailMessage,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO mail_messages (
              id, account_id, remote_id, thread_id, folder_path,
              from_json, to_json, cc_json, bcc_json, reply_to_json,
              subject, preview, body_text, body_html,
              flags_json, labels_json, headers_json, attachments_json,
              sent_at, received_at, created_at, updated_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9, ?10,
              ?11, ?12, ?13, ?14,
              ?15, ?16, ?17, ?18,
              ?19, ?20, ?21, ?22
            )
            ON CONFLICT(account_id, remote_id) DO UPDATE SET
              account_id = excluded.account_id,
              remote_id = excluded.remote_id,
              thread_id = excluded.thread_id,
              folder_path = excluded.folder_path,
              from_json = excluded.from_json,
              to_json = excluded.to_json,
              cc_json = excluded.cc_json,
              bcc_json = excluded.bcc_json,
              reply_to_json = excluded.reply_to_json,
              subject = excluded.subject,
              preview = excluded.preview,
              body_text = excluded.body_text,
              body_html = excluded.body_html,
              flags_json = excluded.flags_json,
              labels_json = excluded.labels_json,
              headers_json = excluded.headers_json,
              attachments_json = excluded.attachments_json,
              sent_at = excluded.sent_at,
              received_at = excluded.received_at,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(message.id.to_string())
        .bind(message.account_id.to_string())
        .bind(&message.remote_id)
        .bind(&message.thread_id)
        .bind(&message.folder_path)
        .bind(serde_json::to_string(&message.from)?)
        .bind(serde_json::to_string(&message.to)?)
        .bind(serde_json::to_string(&message.cc)?)
        .bind(serde_json::to_string(&message.bcc)?)
        .bind(serde_json::to_string(&message.reply_to)?)
        .bind(&message.subject)
        .bind(&message.preview)
        .bind(&message.body_text)
        .bind(&message.body_html)
        .bind(serde_json::to_string(&message.flags)?)
        .bind(serde_json::to_string(&message.labels)?)
        .bind(serde_json::to_string(&message.headers)?)
        .bind(serde_json::to_string(&message.attachments)?)
        .bind(message.sent_at.map(|value| value.to_rfc3339()))
        .bind(message.received_at.to_rfc3339())
        .bind(message.created_at.to_rfc3339())
        .bind(message.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        self.search.index_message(message).await?;
        Ok(())
    }

    pub async fn upsert_mail_messages(
        &self,
        messages: &[cove_core::MailMessage],
    ) -> Result<(), StorageError> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        for message in messages {
            sqlx::query(
                r#"
                INSERT INTO mail_messages (
                  id, account_id, remote_id, thread_id, folder_path,
                  from_json, to_json, cc_json, bcc_json, reply_to_json,
                  subject, preview, body_text, body_html,
                  flags_json, labels_json, headers_json, attachments_json,
                  sent_at, received_at, created_at, updated_at
                ) VALUES (
                  ?1, ?2, ?3, ?4, ?5,
                  ?6, ?7, ?8, ?9, ?10,
                  ?11, ?12, ?13, ?14,
                  ?15, ?16, ?17, ?18,
                  ?19, ?20, ?21, ?22
                )
                ON CONFLICT(account_id, remote_id) DO UPDATE SET
                  account_id = excluded.account_id,
                  remote_id = excluded.remote_id,
                  thread_id = excluded.thread_id,
                  folder_path = excluded.folder_path,
                  from_json = excluded.from_json,
                  to_json = excluded.to_json,
                  cc_json = excluded.cc_json,
                  bcc_json = excluded.bcc_json,
                  reply_to_json = excluded.reply_to_json,
                  subject = excluded.subject,
                  preview = excluded.preview,
                  body_text = excluded.body_text,
                  body_html = excluded.body_html,
                  flags_json = excluded.flags_json,
                  labels_json = excluded.labels_json,
                  headers_json = excluded.headers_json,
                  attachments_json = excluded.attachments_json,
                  sent_at = excluded.sent_at,
                  received_at = excluded.received_at,
                  updated_at = excluded.updated_at
                "#,
            )
            .bind(message.id.to_string())
            .bind(message.account_id.to_string())
            .bind(&message.remote_id)
            .bind(&message.thread_id)
            .bind(&message.folder_path)
            .bind(serde_json::to_string(&message.from)?)
            .bind(serde_json::to_string(&message.to)?)
            .bind(serde_json::to_string(&message.cc)?)
            .bind(serde_json::to_string(&message.bcc)?)
            .bind(serde_json::to_string(&message.reply_to)?)
            .bind(&message.subject)
            .bind(&message.preview)
            .bind(&message.body_text)
            .bind(&message.body_html)
            .bind(serde_json::to_string(&message.flags)?)
            .bind(serde_json::to_string(&message.labels)?)
            .bind(serde_json::to_string(&message.headers)?)
            .bind(serde_json::to_string(&message.attachments)?)
            .bind(message.sent_at.map(|value| value.to_rfc3339()))
            .bind(message.received_at.to_rfc3339())
            .bind(message.created_at.to_rfc3339())
            .bind(message.updated_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        self.search.index_messages(messages).await?;
        Ok(())
    }

    pub async fn list_mail_messages(
        &self,
        account_id: Uuid,
        folder: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<SearchResult<cove_core::MailMessage>, StorageError> {
        let pattern_rows = if let Some(folder) = folder {
            sqlx::query(
                r#"
                SELECT * FROM mail_messages
                WHERE account_id = ?1 AND folder_path = ?2
                ORDER BY received_at DESC
                LIMIT ?3 OFFSET ?4
                "#,
            )
            .bind(account_id.to_string())
            .bind(folder)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT * FROM mail_messages
                WHERE account_id = ?1
                ORDER BY received_at DESC
                LIMIT ?2 OFFSET ?3
                "#,
            )
            .bind(account_id.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        let items: Vec<cove_core::MailMessage> = pattern_rows
            .into_iter()
            .map(Self::row_to_mail_message)
            .collect::<Result<_, _>>()?;

        Ok(SearchResult {
            total: items.len(),
            items,
        })
    }

    pub async fn list_mail_folders(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<MailFolder>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT
              folder_path,
              COUNT(*) AS total_count,
              SUM(CASE WHEN flags_json LIKE '%"seen":false%' THEN 1 ELSE 0 END) AS unread_count
            FROM mail_messages
            WHERE account_id = ?1
            GROUP BY folder_path
            ORDER BY folder_path ASC
            "#,
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut folders = Vec::with_capacity(rows.len());
        for row in rows {
            let path: String = row.try_get("folder_path")?;
            let total_count: i64 = row.try_get("total_count")?;
            let unread_count: i64 = row.try_get("unread_count")?;
            folders.push(MailFolder {
                account_id,
                remote_id: path.clone(),
                path,
                delimiter: Some("/".to_string()),
                unread_count: unread_count.max(0) as u32,
                total_count: total_count.max(0) as u32,
            });
        }

        Ok(folders)
    }

    pub async fn list_thread_messages(
        &self,
        account_id: Uuid,
        thread_id: &str,
    ) -> Result<Vec<cove_core::MailMessage>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM mail_messages
            WHERE account_id = ?1 AND thread_id = ?2
            ORDER BY received_at ASC
            "#,
        )
        .bind(account_id.to_string())
        .bind(thread_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_mail_message).collect()
    }

    pub async fn list_unified_thread_messages(
        &self,
        thread_id: &str,
    ) -> Result<Vec<cove_core::MailMessage>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM mail_messages
            WHERE thread_id = ?1
            ORDER BY received_at ASC
            "#,
        )
        .bind(thread_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_mail_message).collect()
    }

    pub async fn get_mail_message(
        &self,
        message_id: Uuid,
    ) -> Result<Option<cove_core::MailMessage>, StorageError> {
        let row = sqlx::query("SELECT * FROM mail_messages WHERE id = ?1")
            .bind(message_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.map(Self::row_to_mail_message).transpose()
    }

    pub async fn search_mail(
        &self,
        query_text: &str,
        limit: usize,
    ) -> Result<SearchResult<cove_core::MailMessage>, StorageError> {
        let mut hits = Vec::new();
        let ids = self.search.search(query_text, limit)?;

        for id in &ids {
            let row = sqlx::query("SELECT * FROM mail_messages WHERE id = ?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
            if let Some(row) = row {
                hits.push(Self::row_to_mail_message(row)?);
            }
        }

        if hits.is_empty() {
            let like = format!("%{}%", query_text);
            let fallback = sqlx::query(
                r#"
                SELECT * FROM mail_messages
                WHERE subject LIKE ?1 OR preview LIKE ?1 OR body_text LIKE ?1
                ORDER BY received_at DESC
                LIMIT ?2
                "#,
            )
            .bind(like)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            hits = fallback
                .into_iter()
                .map(Self::row_to_mail_message)
                .collect::<Result<_, _>>()?;
        }

        Ok(SearchResult {
            total: hits.len(),
            items: hits,
        })
    }

    // -- attachment content ------------------------------------------------

    pub async fn save_attachment_content(
        &self,
        attachment_id: Uuid,
        message_id: Uuid,
        account_id: Uuid,
        content: &[u8],
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO mail_attachment_content (attachment_id, message_id, account_id, content)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(attachment_id) DO UPDATE SET content = excluded.content
            "#,
        )
        .bind(attachment_id.to_string())
        .bind(message_id.to_string())
        .bind(account_id.to_string())
        .bind(content)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_attachment_content(
        &self,
        attachment_id: Uuid,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let row = sqlx::query(
            "SELECT content FROM mail_attachment_content WHERE attachment_id = ?1",
        )
        .bind(attachment_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<Vec<u8>, _>("content")))
    }

    // -- snooze / pin / send-later ------------------------------------------

    pub async fn snooze_message(
        &self,
        message_id: Uuid,
        until: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE mail_messages SET snoozed_until = ?1 WHERE id = ?2")
            .bind(until.to_rfc3339())
            .bind(message_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn unsnooze_message(&self, message_id: Uuid) -> Result<(), StorageError> {
        sqlx::query("UPDATE mail_messages SET snoozed_until = NULL WHERE id = ?1")
            .bind(message_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_pinned(
        &self,
        message_id: Uuid,
        pinned: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE mail_messages SET pinned = ?1 WHERE id = ?2")
            .bind(pinned as i32)
            .bind(message_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_message_seen(
        &self,
        message_id: Uuid,
        seen: bool,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE mail_messages SET flags_seen = ?1 WHERE id = ?2")
            .bind(seen as i32)
            .bind(message_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn schedule_send(
        &self,
        message_id: Uuid,
        send_at: Option<DateTime<Utc>>,
    ) -> Result<(), StorageError> {
        sqlx::query("UPDATE mail_messages SET send_at = ?1 WHERE id = ?2")
            .bind(send_at.map(|t| t.to_rfc3339()))
            .bind(message_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn due_scheduled_messages(&self) -> Result<Vec<cove_core::MailMessage>, StorageError> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "SELECT * FROM mail_messages WHERE send_at IS NOT NULL AND send_at <= ?1",
        )
        .bind(&now)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_mail_message).collect()
    }

    // -- unified inbox -------------------------------------------------------

    pub async fn list_all_mail_messages(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<SearchResult<cove_core::MailMessage>, StorageError> {
        let count_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM mail_messages WHERE folder_path = 'INBOX'",
        )
        .fetch_one(&self.pool)
        .await?;
        let total: i64 = count_row.try_get("cnt")?;

        let rows = sqlx::query(
            r#"
            SELECT * FROM mail_messages
            WHERE folder_path = 'INBOX'
            ORDER BY pinned DESC, received_at DESC
            LIMIT ?1 OFFSET ?2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let items: Vec<_> = rows.into_iter().map(Self::row_to_mail_message).collect::<Result<_, _>>()?;
        Ok(SearchResult {
            total: total as usize,
            items,
        })
    }

    // -- signatures ----------------------------------------------------------

    pub async fn upsert_signature(
        &self,
        sig: &cove_core::EmailSignature,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO email_signatures (id, account_id, name, body_html, body_text, is_default)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(sig.id.to_string())
        .bind(sig.account_id.map(|id| id.to_string()))
        .bind(&sig.name)
        .bind(&sig.body_html)
        .bind(&sig.body_text)
        .bind(sig.is_default as i32)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_signatures(
        &self,
        account_id: Option<Uuid>,
    ) -> Result<Vec<cove_core::EmailSignature>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM email_signatures WHERE account_id IS ?1 OR account_id IS NULL ORDER BY is_default DESC, name",
        )
        .bind(account_id.map(|id| id.to_string()))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let acct: Option<String> = row.try_get("account_id")?;
                Ok(cove_core::EmailSignature {
                    id: parse_uuid(&id, "email_signatures.id")?,
                    account_id: acct.as_deref().map(|v| parse_uuid(v, "email_signatures.account_id")).transpose()?,
                    name: row.try_get("name")?,
                    body_html: row.try_get("body_html")?,
                    body_text: row.try_get("body_text")?,
                    is_default: row.try_get::<i32, _>("is_default")? != 0,
                })
            })
            .collect()
    }

    pub async fn delete_signature(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM email_signatures WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -- templates -----------------------------------------------------------

    pub async fn upsert_template(
        &self,
        tpl: &cove_core::EmailTemplate,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO email_templates (id, name, subject, body_html, body_text)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(tpl.id.to_string())
        .bind(&tpl.name)
        .bind(&tpl.subject)
        .bind(&tpl.body_html)
        .bind(&tpl.body_text)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_templates(&self) -> Result<Vec<cove_core::EmailTemplate>, StorageError> {
        let rows = sqlx::query("SELECT * FROM email_templates ORDER BY name")
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                Ok(cove_core::EmailTemplate {
                    id: parse_uuid(&id, "email_templates.id")?,
                    name: row.try_get("name")?,
                    subject: row.try_get("subject")?,
                    body_html: row.try_get("body_html")?,
                    body_text: row.try_get("body_text")?,
                })
            })
            .collect()
    }

    pub async fn delete_template(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM email_templates WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -- mail rules ----------------------------------------------------------

    pub async fn upsert_rule(
        &self,
        rule: &cove_core::MailRule,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO mail_rules
              (id, account_id, name, enabled, conditions_json, match_all, actions_json, stop_processing, sort_order)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(rule.id.to_string())
        .bind(rule.account_id.map(|id| id.to_string()))
        .bind(&rule.name)
        .bind(rule.enabled as i32)
        .bind(serde_json::to_string(&rule.conditions)?)
        .bind(rule.match_all as i32)
        .bind(serde_json::to_string(&rule.actions)?)
        .bind(rule.stop_processing as i32)
        .bind(rule.order)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_rules(&self) -> Result<Vec<cove_core::MailRule>, StorageError> {
        let rows = sqlx::query("SELECT * FROM mail_rules ORDER BY sort_order")
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let acct: Option<String> = row.try_get("account_id")?;
                Ok(cove_core::MailRule {
                    id: parse_uuid(&id, "mail_rules.id")?,
                    account_id: acct.as_deref().map(|v| parse_uuid(v, "mail_rules.account_id")).transpose()?,
                    name: row.try_get("name")?,
                    enabled: row.try_get::<i32, _>("enabled")? != 0,
                    conditions: parse_json(&row.try_get::<String, _>("conditions_json")?, "mail_rules.conditions_json")?,
                    match_all: row.try_get::<i32, _>("match_all")? != 0,
                    actions: parse_json(&row.try_get::<String, _>("actions_json")?, "mail_rules.actions_json")?,
                    stop_processing: row.try_get::<i32, _>("stop_processing")? != 0,
                    order: row.try_get("sort_order")?,
                })
            })
            .collect()
    }

    pub async fn delete_rule(&self, id: Uuid) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM mail_rules WHERE id = ?1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -- contacts ------------------------------------------------------------

    pub async fn upsert_contact(
        &self,
        contact: &cove_core::Contact,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO contacts (id, account_id, email, display_name, phone, organization, notes, last_contacted, contact_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(email) DO UPDATE SET
              display_name = COALESCE(excluded.display_name, contacts.display_name),
              phone = COALESCE(excluded.phone, contacts.phone),
              organization = COALESCE(excluded.organization, contacts.organization),
              notes = COALESCE(excluded.notes, contacts.notes),
              last_contacted = excluded.last_contacted,
              contact_count = excluded.contact_count
            "#,
        )
        .bind(contact.id.to_string())
        .bind(contact.account_id.map(|id| id.to_string()))
        .bind(&contact.email)
        .bind(&contact.display_name)
        .bind(&contact.phone)
        .bind(&contact.organization)
        .bind(&contact.notes)
        .bind(contact.last_contacted.map(|dt| dt.to_rfc3339()))
        .bind(contact.contact_count)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn search_contacts(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<cove_core::Contact>, StorageError> {
        let pattern = format!("%{query}%");
        let rows = sqlx::query(
            r#"
            SELECT * FROM contacts
            WHERE email LIKE ?1 OR display_name LIKE ?1
            ORDER BY contact_count DESC, display_name
            LIMIT ?2
            "#,
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                let acct: Option<String> = row.try_get("account_id")?;
                let last: Option<String> = row.try_get("last_contacted")?;
                Ok(cove_core::Contact {
                    id: parse_uuid(&id, "contacts.id")?,
                    account_id: acct.as_deref().map(|v| parse_uuid(v, "contacts.account_id")).transpose()?,
                    email: row.try_get("email")?,
                    display_name: row.try_get("display_name")?,
                    phone: row.try_get("phone")?,
                    organization: row.try_get("organization")?,
                    notes: row.try_get("notes")?,
                    last_contacted: last.as_deref().map(|v| parse_datetime(v, "contacts.last_contacted")).transpose()?,
                    contact_count: row.try_get::<u32, _>("contact_count").unwrap_or(0),
                })
            })
            .collect()
    }

    pub async fn increment_contact_count(&self, email: &str) -> Result<(), StorageError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO contacts (id, email, contact_count, last_contacted)
            VALUES (?1, ?2, 1, ?3)
            ON CONFLICT(email) DO UPDATE SET
              contact_count = contacts.contact_count + 1,
              last_contacted = excluded.last_contacted
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(email)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // -- calendar ----------------------------------------------------------

    pub async fn upsert_calendar_event(&self, event: &CalendarEvent) -> Result<(), StorageError> {
        let rsvp_str = serde_json::to_string(&event.rsvp_status)
            .unwrap_or_else(|_| "\"needs_action\"".to_string())
            .trim_matches('"')
            .to_string();

        sqlx::query(
            r#"
            INSERT INTO calendar_events (
              id, account_id, calendar_id, remote_id, title,
              description, location, timezone, starts_at, ends_at,
              all_day, recurrence_rule, attendees_json, organizer,
              alarms_json, rsvp_status, updated_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9, ?10,
              ?11, ?12, ?13, ?14,
              ?15, ?16, ?17
            )
            ON CONFLICT(id) DO UPDATE SET
              account_id = excluded.account_id,
              calendar_id = excluded.calendar_id,
              remote_id = excluded.remote_id,
              title = excluded.title,
              description = excluded.description,
              location = excluded.location,
              timezone = excluded.timezone,
              starts_at = excluded.starts_at,
              ends_at = excluded.ends_at,
              all_day = excluded.all_day,
              recurrence_rule = excluded.recurrence_rule,
              attendees_json = excluded.attendees_json,
              organizer = excluded.organizer,
              alarms_json = excluded.alarms_json,
              rsvp_status = excluded.rsvp_status,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(event.id.to_string())
        .bind(event.account_id.to_string())
        .bind(&event.calendar_id)
        .bind(&event.remote_id)
        .bind(&event.title)
        .bind(&event.description)
        .bind(&event.location)
        .bind(&event.timezone)
        .bind(event.starts_at.to_rfc3339())
        .bind(event.ends_at.to_rfc3339())
        .bind(if event.all_day { 1_i64 } else { 0_i64 })
        .bind(&event.recurrence_rule)
        .bind(serde_json::to_string(&event.attendees)?)
        .bind(&event.organizer)
        .bind(serde_json::to_string(&event.alarms)?)
        .bind(&rsvp_str)
        .bind(event.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn update_rsvp_status(
        &self,
        event_id: Uuid,
        status: &cove_core::RsvpStatus,
    ) -> Result<(), StorageError> {
        let status_str = serde_json::to_string(status)
            .unwrap_or_else(|_| "\"needs_action\"".to_string())
            .trim_matches('"')
            .to_string();
        sqlx::query("UPDATE calendar_events SET rsvp_status = ?1 WHERE id = ?2")
            .bind(&status_str)
            .bind(event_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_calendar_events(
        &self,
        account_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM calendar_events
            WHERE account_id = ?1 AND starts_at >= ?2 AND ends_at <= ?3
            ORDER BY starts_at ASC
            "#,
        )
        .bind(account_id.to_string())
        .bind(from.to_rfc3339())
        .bind(to.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_calendar_event).collect()
    }

    pub async fn upsert_task(&self, task: &ReminderTask) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO reminder_tasks (
              id, account_id, list_id, remote_id, title,
              notes, due_at, completed_at, priority, status,
              repeat_rule, parent_id, snoozed_until, created_at, updated_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9, ?10,
              ?11, ?12, ?13, ?14, ?15
            )
            ON CONFLICT(id) DO UPDATE SET
              account_id = excluded.account_id,
              list_id = excluded.list_id,
              remote_id = excluded.remote_id,
              title = excluded.title,
              notes = excluded.notes,
              due_at = excluded.due_at,
              completed_at = excluded.completed_at,
              priority = excluded.priority,
              status = excluded.status,
              repeat_rule = excluded.repeat_rule,
              parent_id = excluded.parent_id,
              snoozed_until = excluded.snoozed_until,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(task.id.to_string())
        .bind(task.account_id.to_string())
        .bind(&task.list_id)
        .bind(&task.remote_id)
        .bind(&task.title)
        .bind(&task.notes)
        .bind(task.due_at.map(|value| value.to_rfc3339()))
        .bind(task.completed_at.map(|value| value.to_rfc3339()))
        .bind(serde_json::to_string(&task.priority)?)
        .bind(serde_json::to_string(&task.status)?)
        .bind(&task.repeat_rule)
        .bind(task.parent_id.map(|value| value.to_string()))
        .bind(task.snoozed_until.map(|value| value.to_rfc3339()))
        .bind(task.created_at.to_rfc3339())
        .bind(task.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_tasks(&self, account_id: Uuid) -> Result<Vec<ReminderTask>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM reminder_tasks
            WHERE account_id = ?1
            ORDER BY due_at ASC
            "#,
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_task).collect()
    }

    /// List subtasks (tasks with a given parent_id).
    pub async fn list_subtasks(&self, parent_id: Uuid) -> Result<Vec<ReminderTask>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM reminder_tasks WHERE parent_id = ?1 ORDER BY priority DESC, due_at ASC",
        )
        .bind(parent_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_task).collect()
    }

    /// List top-level tasks (no parent) for an account, ordered by priority descending.
    pub async fn list_tasks_by_priority(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<ReminderTask>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM reminder_tasks
            WHERE account_id = ?1 AND (parent_id IS NULL OR parent_id = '')
            ORDER BY
                CASE priority
                    WHEN 'critical' THEN 0
                    WHEN 'high' THEN 1
                    WHEN 'normal' THEN 2
                    WHEN 'low' THEN 3
                    ELSE 4
                END,
                due_at ASC
            "#,
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_task).collect()
    }

    pub async fn enqueue_sync_job(&self, job: &SyncJob) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            INSERT INTO sync_queue (
              id, account_id, domain, status, payload_json,
              attempt_count, max_attempts, run_after, last_error,
              created_at, updated_at
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9,
              ?10, ?11
            )
            ON CONFLICT(id) DO UPDATE SET
              status = excluded.status,
              payload_json = excluded.payload_json,
              attempt_count = excluded.attempt_count,
              max_attempts = excluded.max_attempts,
              run_after = excluded.run_after,
              last_error = excluded.last_error,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(job.id.to_string())
        .bind(job.account_id.to_string())
        .bind(serde_json::to_string(&job.domain)?)
        .bind(serde_json::to_string(&job.status)?)
        .bind(serde_json::to_string(&job.payload_json)?)
        .bind(i64::from(job.attempt_count))
        .bind(i64::from(job.max_attempts))
        .bind(job.run_after.to_rfc3339())
        .bind(&job.last_error)
        .bind(job.created_at.to_rfc3339())
        .bind(job.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn fetch_due_sync_jobs(&self, limit: i64) -> Result<Vec<SyncJob>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM sync_queue
            WHERE status = '"queued"' AND run_after <= ?1
            ORDER BY run_after ASC
            LIMIT ?2
            "#,
        )
        .bind(Utc::now().to_rfc3339())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_sync_job).collect()
    }

    pub async fn pending_sync_jobs_count(&self) -> Result<u64, StorageError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS total
            FROM sync_queue
            WHERE status = '"queued"'
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = row.try_get("total")?;
        Ok(count.max(0) as u64)
    }

    pub async fn has_active_sync_job(
        &self,
        account_id: Uuid,
        domain: cove_core::SyncDomain,
    ) -> Result<bool, StorageError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS total
            FROM sync_queue
            WHERE account_id = ?1
              AND domain = ?2
              AND (status = '"queued"' OR status = '"running"')
            "#,
        )
        .bind(account_id.to_string())
        .bind(serde_json::to_string(&domain)?)
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = row.try_get("total")?;
        Ok(count > 0)
    }

    pub async fn has_sync_history(
        &self,
        account_id: Uuid,
        domain: cove_core::SyncDomain,
    ) -> Result<bool, StorageError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS total
            FROM sync_queue
            WHERE account_id = ?1 AND domain = ?2
            "#,
        )
        .bind(account_id.to_string())
        .bind(serde_json::to_string(&domain)?)
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = row.try_get("total")?;
        Ok(count > 0)
    }

    pub async fn update_sync_job_status(
        &self,
        id: Uuid,
        status: SyncStatus,
        last_error: Option<String>,
        attempt_count: Option<u32>,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            UPDATE sync_queue
            SET status = ?2,
                last_error = ?3,
                attempt_count = COALESCE(?4, attempt_count),
                updated_at = ?5
            WHERE id = ?1
            "#,
        )
        .bind(id.to_string())
        .bind(serde_json::to_string(&status)?)
        .bind(last_error)
        .bind(attempt_count.map(i64::from))
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn row_to_account(row: sqlx::sqlite::SqliteRow) -> Result<Account, StorageError> {
        let id_raw: String = row.try_get("id")?;
        let provider_raw: String = row.try_get("provider")?;
        let protocols_raw: String = row.try_get("protocols_json")?;
        let oauth_raw: Option<String> = row.try_get("oauth_profile_json")?;
        let created_raw: String = row.try_get("created_at")?;
        let updated_raw: String = row.try_get("updated_at")?;

        Ok(Account {
            id: parse_uuid(&id_raw, "accounts.id")?,
            provider: parse_json(&provider_raw, "accounts.provider")?,
            protocols: parse_json(&protocols_raw, "accounts.protocols_json")?,
            display_name: row.try_get("display_name")?,
            email_address: row.try_get("email_address")?,
            oauth_profile: oauth_raw
                .as_deref()
                .map(|raw| parse_json(raw, "accounts.oauth_profile_json"))
                .transpose()?,
            created_at: parse_datetime(&created_raw, "accounts.created_at")?,
            updated_at: parse_datetime(&updated_raw, "accounts.updated_at")?,
        })
    }

    fn row_to_mail_message(
        row: sqlx::sqlite::SqliteRow,
    ) -> Result<cove_core::MailMessage, StorageError> {
        let id_raw: String = row.try_get("id")?;
        let account_id_raw: String = row.try_get("account_id")?;
        let sent_at_raw: Option<String> = row.try_get("sent_at")?;
        let received_raw: String = row.try_get("received_at")?;
        let created_raw: String = row.try_get("created_at")?;
        let updated_raw: String = row.try_get("updated_at")?;
        let snoozed_raw: Option<String> = row.try_get("snoozed_until").unwrap_or(None);
        let pinned_raw: i32 = row.try_get("pinned").unwrap_or(0);
        let send_at_raw: Option<String> = row.try_get("send_at").unwrap_or(None);

        Ok(cove_core::MailMessage {
            id: parse_uuid(&id_raw, "mail_messages.id")?,
            account_id: parse_uuid(&account_id_raw, "mail_messages.account_id")?,
            remote_id: row.try_get("remote_id")?,
            thread_id: row.try_get("thread_id")?,
            folder_path: row.try_get("folder_path")?,
            from: parse_json(
                &row.try_get::<String, _>("from_json")?,
                "mail_messages.from_json",
            )?,
            to: parse_json(
                &row.try_get::<String, _>("to_json")?,
                "mail_messages.to_json",
            )?,
            cc: parse_json(
                &row.try_get::<String, _>("cc_json")?,
                "mail_messages.cc_json",
            )?,
            bcc: parse_json(
                &row.try_get::<String, _>("bcc_json")?,
                "mail_messages.bcc_json",
            )?,
            reply_to: parse_json(
                &row.try_get::<String, _>("reply_to_json")?,
                "mail_messages.reply_to_json",
            )?,
            subject: row.try_get("subject")?,
            preview: row.try_get("preview")?,
            body_text: row.try_get("body_text")?,
            body_html: row.try_get("body_html")?,
            flags: parse_json(
                &row.try_get::<String, _>("flags_json")?,
                "mail_messages.flags_json",
            )?,
            labels: parse_json(
                &row.try_get::<String, _>("labels_json")?,
                "mail_messages.labels_json",
            )?,
            headers: parse_json(
                &row.try_get::<String, _>("headers_json")?,
                "mail_messages.headers_json",
            )?,
            attachments: parse_json(
                &row.try_get::<String, _>("attachments_json")?,
                "mail_messages.attachments_json",
            )?,
            sent_at: sent_at_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "mail_messages.sent_at"))
                .transpose()?,
            received_at: parse_datetime(&received_raw, "mail_messages.received_at")?,
            created_at: parse_datetime(&created_raw, "mail_messages.created_at")?,
            updated_at: parse_datetime(&updated_raw, "mail_messages.updated_at")?,
            snoozed_until: snoozed_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "mail_messages.snoozed_until"))
                .transpose()?,
            pinned: pinned_raw != 0,
            send_at: send_at_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "mail_messages.send_at"))
                .transpose()?,
        })
    }

    fn row_to_calendar_event(row: sqlx::sqlite::SqliteRow) -> Result<CalendarEvent, StorageError> {
        let id_raw: String = row.try_get("id")?;
        let account_id_raw: String = row.try_get("account_id")?;
        let starts_raw: String = row.try_get("starts_at")?;
        let ends_raw: String = row.try_get("ends_at")?;
        let updated_raw: String = row.try_get("updated_at")?;

        Ok(CalendarEvent {
            id: parse_uuid(&id_raw, "calendar_events.id")?,
            account_id: parse_uuid(&account_id_raw, "calendar_events.account_id")?,
            calendar_id: row.try_get("calendar_id")?,
            remote_id: row.try_get("remote_id")?,
            title: row.try_get("title")?,
            description: row.try_get("description")?,
            location: row.try_get("location")?,
            timezone: row.try_get("timezone")?,
            starts_at: parse_datetime(&starts_raw, "calendar_events.starts_at")?,
            ends_at: parse_datetime(&ends_raw, "calendar_events.ends_at")?,
            all_day: row.try_get::<i64, _>("all_day")? == 1,
            recurrence_rule: row.try_get("recurrence_rule")?,
            attendees: parse_json(
                &row.try_get::<String, _>("attendees_json")?,
                "calendar_events.attendees_json",
            )?,
            organizer: row.try_get("organizer")?,
            alarms: parse_json(
                &row.try_get::<String, _>("alarms_json")?,
                "calendar_events.alarms_json",
            )?,
            rsvp_status: row.try_get::<Option<String>, _>("rsvp_status")
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
                .unwrap_or_default(),
            updated_at: parse_datetime(&updated_raw, "calendar_events.updated_at")?,
        })
    }

    fn row_to_task(row: sqlx::sqlite::SqliteRow) -> Result<ReminderTask, StorageError> {
        let id_raw: String = row.try_get("id")?;
        let account_id_raw: String = row.try_get("account_id")?;
        let due_raw: Option<String> = row.try_get("due_at")?;
        let completed_raw: Option<String> = row.try_get("completed_at")?;
        let parent_raw: Option<String> = row.try_get("parent_id")?;
        let snoozed_raw: Option<String> = row.try_get("snoozed_until")?;
        let created_raw: String = row.try_get("created_at")?;
        let updated_raw: String = row.try_get("updated_at")?;

        Ok(ReminderTask {
            id: parse_uuid(&id_raw, "reminder_tasks.id")?,
            account_id: parse_uuid(&account_id_raw, "reminder_tasks.account_id")?,
            list_id: row.try_get("list_id")?,
            remote_id: row.try_get("remote_id")?,
            title: row.try_get("title")?,
            notes: row.try_get("notes")?,
            due_at: due_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "reminder_tasks.due_at"))
                .transpose()?,
            completed_at: completed_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "reminder_tasks.completed_at"))
                .transpose()?,
            priority: parse_json(
                &row.try_get::<String, _>("priority")?,
                "reminder_tasks.priority",
            )?,
            status: parse_json(
                &row.try_get::<String, _>("status")?,
                "reminder_tasks.status",
            )?,
            repeat_rule: row.try_get("repeat_rule")?,
            parent_id: parent_raw
                .as_deref()
                .map(|raw| parse_uuid(raw, "reminder_tasks.parent_id"))
                .transpose()?,
            snoozed_until: snoozed_raw
                .as_deref()
                .map(|raw| parse_datetime(raw, "reminder_tasks.snoozed_until"))
                .transpose()?,
            created_at: parse_datetime(&created_raw, "reminder_tasks.created_at")?,
            updated_at: parse_datetime(&updated_raw, "reminder_tasks.updated_at")?,
        })
    }

    fn row_to_sync_job(row: sqlx::sqlite::SqliteRow) -> Result<SyncJob, StorageError> {
        let id_raw: String = row.try_get("id")?;
        let account_id_raw: String = row.try_get("account_id")?;
        let run_after_raw: String = row.try_get("run_after")?;
        let created_raw: String = row.try_get("created_at")?;
        let updated_raw: String = row.try_get("updated_at")?;

        let payload_raw: String = row.try_get("payload_json")?;
        let domain_raw: String = row.try_get("domain")?;
        let status_raw: String = row.try_get("status")?;

        Ok(SyncJob {
            id: parse_uuid(&id_raw, "sync_queue.id")?,
            account_id: parse_uuid(&account_id_raw, "sync_queue.account_id")?,
            domain: parse_json(&domain_raw, "sync_queue.domain")?,
            status: parse_json(&status_raw, "sync_queue.status")?,
            payload_json: parse_json(&payload_raw, "sync_queue.payload_json")?,
            attempt_count: row.try_get::<i64, _>("attempt_count")? as u32,
            max_attempts: row.try_get::<i64, _>("max_attempts")? as u32,
            run_after: parse_datetime(&run_after_raw, "sync_queue.run_after")?,
            last_error: row.try_get("last_error")?,
            created_at: parse_datetime(&created_raw, "sync_queue.created_at")?,
            updated_at: parse_datetime(&updated_raw, "sync_queue.updated_at")?,
        })
    }
}

fn parse_uuid(raw: &str, field: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(raw)
        .map_err(|err| StorageError::Data(format!("invalid uuid for {field}: {err}")))
}

fn parse_datetime(raw: &str, field: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| StorageError::Data(format!("invalid datetime for {field}: {err}")))
}

fn parse_json<T>(raw: &str, field: &str) -> Result<T, StorageError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(raw)
        .map_err(|err| StorageError::Data(format!("invalid json for {field}: {err}")))
}
