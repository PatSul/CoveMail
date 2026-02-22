use crate::{
    default_protocol_for_provider, EmailBackend, EmailError, EwsBackend, ImapSmtpBackend,
    JmapBackend, OutgoingMail, ProtocolSettings,
};
use cove_core::{
    Account, ContactSummary, MailAddress, MailAttachment, MailFolder, MailMessage,
    MailThreadSummary,
};
use cove_storage::Storage;
use chrono::{TimeZone, Utc};
use mailparse::{parse_mail, ParsedMail};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

/// Maximum concurrent sync operations per mail-server domain.
const MAX_CONCURRENT_PER_DOMAIN: usize = 2;

#[derive(Clone)]
pub struct EmailService {
    storage: Storage,
    imap_smtp: Arc<ImapSmtpBackend>,
    ews: Arc<EwsBackend>,
    jmap: Arc<JmapBackend>,
    domain_semaphores: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl EmailService {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            imap_smtp: Arc::new(ImapSmtpBackend),
            ews: Arc::new(EwsBackend::new()),
            jmap: Arc::new(JmapBackend::new()),
            domain_semaphores: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Acquire a permit for the given server domain, limiting concurrency.
    async fn acquire_domain_permit(&self, settings: &ProtocolSettings) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let domain = settings.imap_host.as_deref()
            .or(settings.smtp_host.as_deref())
            .or(settings.endpoint.as_deref())
            .unwrap_or("unknown")
            .to_lowercase();

        let sem = {
            let mut map = self.domain_semaphores.lock().await;
            map.entry(domain)
                .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PER_DOMAIN)))
                .clone()
        };

        sem.acquire_owned().await.ok()
    }

    pub async fn sync_folders(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
    ) -> Result<Vec<MailFolder>, EmailError> {
        let _permit = self.acquire_domain_permit(settings).await;
        self.backend_for(account)
            .sync_folders(account, settings)
            .await
    }

    pub async fn sync_recent_mail(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
        limit: usize,
    ) -> Result<usize, EmailError> {
        let _permit = self.acquire_domain_permit(settings).await;
        let backend = self.backend_for(account);
        let result = backend
            .fetch_recent(account, settings, folder_path, limit)
            .await?;

        self.storage.upsert_mail_messages(&result.messages).await?;

        for (att_id, msg_id, content) in &result.attachment_content {
            let _ = self
                .storage
                .save_attachment_content(*att_id, *msg_id, account.id, content)
                .await;
        }

        Ok(result.messages.len())
    }

    pub async fn send(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        outgoing: &OutgoingMail,
    ) -> Result<(), EmailError> {
        let _permit = self.acquire_domain_permit(settings).await;
        self.backend_for(account)
            .send_mail(account, settings, outgoing)
            .await
    }

    pub async fn start_idle(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
    ) -> Result<(), EmailError> {
        self.backend_for(account)
            .start_idle(account, settings, folder_path)
            .await
    }

    pub async fn import_raw_message(
        &self,
        account_id: Uuid,
        folder_path: &str,
        remote_id: &str,
        raw_rfc822: &[u8],
    ) -> Result<MailMessage, EmailError> {
        let parsed = parse_mail(raw_rfc822)?;
        let now = Utc::now();

        let subject =
            header_value(&parsed, "Subject").unwrap_or_else(|| "(No subject)".to_string());
        let message_id =
            header_value(&parsed, "Message-ID").unwrap_or_else(|| remote_id.to_string());

        let body_text = extract_text_body(&parsed);
        let body_html = extract_html_body(&parsed).map(|html| ammonia::clean(&html));
        let preview = body_text
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();

        let mut headers = BTreeMap::new();
        for header in parsed.get_headers() {
            let key = header.get_key().to_string();
            let value = header.get_value();
            headers.insert(key, value);
        }

        let sent_at = header_value(&parsed, "Date")
            .and_then(|date| mailparse::dateparse(&date).ok())
            .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single());

        let (attachments, att_content) = extract_attachments(&parsed);
        let msg_id = Uuid::new_v4();

        let message = MailMessage {
            id: msg_id,
            account_id,
            remote_id: remote_id.to_string(),
            thread_id: thread_id_from_headers(&headers, &message_id),
            folder_path: folder_path.to_string(),
            from: parse_address_list(header_value(&parsed, "From")),
            to: parse_address_list(header_value(&parsed, "To")),
            cc: parse_address_list(header_value(&parsed, "Cc")),
            bcc: parse_address_list(header_value(&parsed, "Bcc")),
            reply_to: parse_address_list(header_value(&parsed, "Reply-To")),
            subject,
            preview,
            body_text,
            body_html,
            flags: cove_core::MailFlags::default(),
            labels: vec![],
            headers,
            attachments,
            sent_at,
            received_at: sent_at.unwrap_or(now),
            created_at: now,
            updated_at: now,
        };

        self.storage.upsert_mail_message(&message).await?;
        for (att_id, content) in att_content {
            let _ = self
                .storage
                .save_attachment_content(att_id, msg_id, account_id, &content)
                .await;
        }
        Ok(message)
    }

    pub async fn list_threads(
        &self,
        account_id: Uuid,
        folder: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MailThreadSummary>, EmailError> {
        let messages = self
            .storage
            .list_mail_messages(account_id, folder, limit, offset)
            .await?
            .items;

        let mut grouped: HashMap<String, Vec<MailMessage>> = HashMap::new();
        for message in messages {
            grouped
                .entry(message.thread_id.clone())
                .or_default()
                .push(message);
        }

        let mut summaries = grouped
            .into_iter()
            .map(|(thread_id, mut items)| {
                items.sort_by_key(|msg| msg.received_at);
                let most_recent = items.last().map(|m| m.received_at).unwrap_or_else(Utc::now);
                let subject = items
                    .last()
                    .map(|m| m.subject.clone())
                    .unwrap_or_else(|| "(No subject)".to_string());

                let unread = items.iter().filter(|m| !m.flags.seen).count();
                let participants = items
                    .iter()
                    .flat_map(|m| m.from.iter().map(|addr| addr.address.clone()))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();

                MailThreadSummary {
                    thread_id,
                    subject,
                    participants,
                    message_count: items.len(),
                    unread_count: unread,
                    most_recent_at: most_recent,
                }
            })
            .collect::<Vec<_>>();

        summaries.sort_by_key(|summary| summary.most_recent_at);
        summaries.reverse();
        Ok(summaries)
    }

    pub async fn list_conversations_by_contact(
        &self,
        account: &Account,
        folder: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ContactSummary>, EmailError> {
        let messages = self
            .storage
            .list_mail_messages(account.id, folder, limit, offset)
            .await?
            .items;

        let mut grouped: HashMap<String, Vec<MailMessage>> = HashMap::new();
        for message in messages {
            let mut other_party = message
                .from
                .first()
                .map(|addr| addr.address.clone())
                .unwrap_or_default();

            if other_party.eq_ignore_ascii_case(&account.email_address) {
                if let Some(to) = message.to.first() {
                    other_party = to.address.clone();
                }
            }

            if other_party.is_empty() {
                other_party = "unknown".to_string();
            }

            grouped.entry(other_party.to_lowercase()).or_default().push(message);
        }

        let mut summaries: Vec<ContactSummary> = grouped
            .into_iter()
            .map(|(email_address, mut items)| {
                items.sort_by_key(|msg| msg.received_at);
                let latest_msg = items.last().unwrap();
                let latest_subject = latest_msg.subject.clone();
                let most_recent_at = latest_msg.received_at;
                
                let display_name = latest_msg
                    .from
                    .first()
                    .filter(|addr| addr.address.eq_ignore_ascii_case(&email_address))
                    .and_then(|addr| addr.name.clone())
                    .or_else(|| {
                        latest_msg
                            .to
                            .iter()
                            .find(|addr| addr.address.eq_ignore_ascii_case(&email_address))
                            .and_then(|addr| addr.name.clone())
                    });

                let unread_count = items.iter().filter(|m| !m.flags.seen).count();

                ContactSummary {
                    email_address,
                    display_name,
                    latest_subject,
                    message_count: items.len(),
                    unread_count,
                    most_recent_at,
                }
            })
            .collect();

        summaries.sort_by_key(|summary| summary.most_recent_at);
        summaries.reverse();
        Ok(summaries)
    }

    pub async fn get_attachment_content(
        &self,
        attachment_id: Uuid,
    ) -> Result<Option<Vec<u8>>, EmailError> {
        Ok(self.storage.get_attachment_content(attachment_id).await?)
    }

    fn backend_for(&self, account: &Account) -> Arc<dyn EmailBackend> {
        match default_protocol_for_provider(&account.provider) {
            "ews" => self.ews.clone(),
            "jmap" => self.jmap.clone(),
            _ => self.imap_smtp.clone(),
        }
    }
}

fn header_value(mail: &ParsedMail<'_>, key: &str) -> Option<String> {
    for header in mail.get_headers() {
        if header.get_key_ref().eq_ignore_ascii_case(key) {
            return Some(header.get_value());
        }
    }

    None
}

fn extract_text_body(mail: &ParsedMail<'_>) -> Option<String> {
    if mail.subparts.is_empty() {
        let content_type = mail.ctype.mimetype.to_ascii_lowercase();
        if content_type == "text/plain" || content_type == "text/markdown" {
            return mail.get_body().ok();
        }
        return None;
    }

    for part in &mail.subparts {
        if let Some(text) = extract_text_body(part) {
            return Some(text);
        }
    }

    None
}

fn extract_html_body(mail: &ParsedMail<'_>) -> Option<String> {
    if mail.subparts.is_empty() {
        let content_type = mail.ctype.mimetype.to_ascii_lowercase();
        if content_type == "text/html" {
            return mail.get_body().ok();
        }
        return None;
    }

    for part in &mail.subparts {
        if let Some(html) = extract_html_body(part) {
            return Some(html);
        }
    }

    None
}

fn parse_address_list(raw: Option<String>) -> Vec<MailAddress> {
    let Some(raw) = raw else {
        return Vec::new();
    };

    raw.split(',')
        .filter_map(|segment| {
            let value = segment.trim();
            if value.is_empty() {
                return None;
            }

            let lt = value.rfind('<');
            let gt = value.rfind('>');
            if let (Some(lt), Some(gt)) = (lt, gt) {
                if lt < gt {
                    let name = value[..lt].trim().trim_matches('"').trim().to_string();
                    let address = value[lt + 1..gt].trim().to_string();
                    if address.is_empty() {
                        return None;
                    }

                    return Some(MailAddress {
                        name: if name.is_empty() { None } else { Some(name) },
                        address,
                    });
                }
            }

            Some(MailAddress {
                name: None,
                address: value.trim_matches('"').to_string(),
            })
        })
        .collect()
}

fn extract_attachments(mail: &ParsedMail<'_>) -> (Vec<MailAttachment>, Vec<(Uuid, Vec<u8>)>) {
    let mut attachments = Vec::new();
    let mut contents = Vec::new();
    collect_attachments(mail, &mut attachments, &mut contents);
    (attachments, contents)
}

fn collect_attachments(
    mail: &ParsedMail<'_>,
    attachments: &mut Vec<MailAttachment>,
    contents: &mut Vec<(Uuid, Vec<u8>)>,
) {
    if mail.subparts.is_empty() {
        let disposition = header_value(mail, "Content-Disposition")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let name = header_filename(&disposition).or_else(|| mail.ctype.params.get("name").cloned());
        let is_attachment = disposition.contains("attachment")
            || (disposition.contains("inline") && name.is_some());

        if is_attachment {
            let raw_body = mail.get_body_raw().unwrap_or_default();
            let id = Uuid::new_v4();
            attachments.push(MailAttachment {
                id,
                file_name: name.unwrap_or_else(|| "attachment.bin".to_string()),
                mime_type: mail.ctype.mimetype.clone(),
                size: raw_body.len() as u64,
                inline: disposition.contains("inline"),
            });
            if !raw_body.is_empty() {
                contents.push((id, raw_body));
            }
        }
        return;
    }

    for part in &mail.subparts {
        collect_attachments(part, attachments, contents);
    }
}

fn header_filename(disposition: &str) -> Option<String> {
    let key = "filename=";
    let idx = disposition.find(key)?;
    let raw = disposition[idx + key.len()..].trim();

    if let Some(stripped) = raw.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(stripped[..end].to_string());
    }

    let value = raw
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(value.to_string())
}

fn thread_id_from_headers(headers: &BTreeMap<String, String>, fallback: &str) -> String {
    headers
        .get("References")
        .or_else(|| headers.get("In-Reply-To"))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}
