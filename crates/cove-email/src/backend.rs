use crate::EmailError;
use cove_core::{
    Account, MailAddress, MailAttachment, MailFlags, MailFolder, MailMessage, Provider,
};
use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{TimeZone, Utc};
use lettre::message::{header, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::{
    transport::smtp::authentication::Credentials, AsyncSmtpTransport, AsyncTransport, Message,
    Tokio1Executor,
};
use mailparse::{parse_mail, ParsedMail};
use regex::Regex;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::task;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct ProtocolSettings {
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub endpoint: Option<String>,
    pub username: String,
    pub access_token: Option<String>,
    pub password: Option<String>,
    pub offline_sync_limit: Option<cove_core::OfflineSyncLimit>,
}

impl std::fmt::Debug for ProtocolSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolSettings")
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("endpoint", &self.endpoint)
            .field("username", &self.username)
            .field("access_token", &self.access_token.as_ref().map(|_| "[REDACTED]"))
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("offline_sync_limit", &self.offline_sync_limit)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    pub file_name: String,
    pub mime_type: String,
    pub content_base64: String,
    pub inline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMail {
    pub from: MailAddress,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
}

/// Messages plus pre-extracted attachment content returned by [`EmailBackend::fetch_recent`].
pub struct FetchResult {
    pub messages: Vec<MailMessage>,
    /// `(attachment_id, message_id, raw_bytes)` for every attachment whose content was available.
    pub attachment_content: Vec<(Uuid, Uuid, Vec<u8>)>,
}

#[async_trait]
pub trait EmailBackend: Send + Sync {
    async fn sync_folders(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
    ) -> Result<Vec<MailFolder>, EmailError>;

    async fn fetch_recent(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
        limit: usize,
    ) -> Result<FetchResult, EmailError>;

    async fn send_mail(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        outgoing: &OutgoingMail,
    ) -> Result<(), EmailError>;

    async fn start_idle(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
    ) -> Result<(), EmailError>;
}

#[derive(Debug, Default)]
pub struct ImapSmtpBackend;

#[async_trait]
impl EmailBackend for ImapSmtpBackend {
    async fn sync_folders(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
    ) -> Result<Vec<MailFolder>, EmailError> {
        if account.provider == Provider::Gmail {
            return sync_gmail_folders(account, settings).await;
        }

        let account_id = account.id;
        let provider = account.provider.clone();
        let settings = settings.clone();

        task::spawn_blocking(move || sync_folders_imap(account_id, provider, &settings))
            .await
            .map_err(|err| EmailError::Data(format!("imap folder sync task failed: {err}")))?
    }

    async fn fetch_recent(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
        limit: usize,
    ) -> Result<FetchResult, EmailError> {
        if account.provider == Provider::Gmail {
            return fetch_recent_gmail(account, settings, folder_path, limit).await;
        }

        let account_id = account.id;
        let provider = account.provider.clone();
        let folder = folder_path.to_string();
        let settings = settings.clone();

        task::spawn_blocking(move || {
            fetch_recent_imap(account_id, provider, &settings, &folder, limit)
        })
        .await
        .map_err(|err| EmailError::Data(format!("imap fetch task failed: {err}")))?
    }

    async fn send_mail(
        &self,
        _account: &Account,
        settings: &ProtocolSettings,
        outgoing: &OutgoingMail,
    ) -> Result<(), EmailError> {
        let smtp_host = settings
            .smtp_host
            .as_deref()
            .ok_or_else(|| EmailError::Data("missing smtp_host".to_string()))?;
        let smtp_port = settings.smtp_port.unwrap_or(465);

        let from = to_mailbox(&outgoing.from)?;
        let to = outgoing
            .to
            .iter()
            .map(to_mailbox)
            .collect::<Result<Vec<_>, _>>()?;

        let mut builder = Message::builder()
            .from(from)
            .subject(outgoing.subject.clone());

        for mailbox in to {
            builder = builder.to(mailbox);
        }

        for cc in &outgoing.cc {
            builder = builder.cc(to_mailbox(cc)?);
        }
        for bcc in &outgoing.bcc {
            builder = builder.bcc(to_mailbox(bcc)?);
        }
        for reply_to in &outgoing.reply_to {
            builder = builder.reply_to(to_mailbox(reply_to)?);
        }

        let alternative = if let Some(html) = &outgoing.body_html {
            MultiPart::alternative()
                .singlepart(SinglePart::plain(outgoing.body_text.clone()))
                .singlepart(
                    SinglePart::builder()
                        .header(header::ContentType::TEXT_HTML)
                        .body(html.clone()),
                )
        } else {
            MultiPart::alternative().singlepart(SinglePart::plain(outgoing.body_text.clone()))
        };

        let payload = if outgoing.attachments.is_empty() {
            alternative
        } else {
            let mut mixed = MultiPart::mixed().multipart(alternative);
            for attachment in &outgoing.attachments {
                let _ = attachment.inline;
                let bytes = STANDARD
                    .decode(attachment.content_base64.as_bytes())
                    .map_err(|err| {
                        EmailError::Build(format!("invalid attachment base64: {err}"))
                    })?;
                let mime = attachment.mime_type.parse().map_err(|err| {
                    EmailError::Build(format!("invalid attachment mime type: {err}"))
                })?;
                mixed = mixed
                    .singlepart(Attachment::new(attachment.file_name.clone()).body(bytes, mime));
            }
            mixed
        };

        let message = builder
            .multipart(payload)
            .map_err(|err| EmailError::Build(err.to_string()))?;

        let mut transport = AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
            .map_err(|err| EmailError::Smtp(err.to_string()))?
            .port(smtp_port);

        let auth_secret = settings
            .password
            .as_ref()
            .or(settings.access_token.as_ref());
        if let Some(secret) = auth_secret {
            transport =
                transport.credentials(Credentials::new(settings.username.clone(), secret.clone()));
        }

        transport
            .build()
            .send(message)
            .await
            .map_err(|err| EmailError::Smtp(err.to_string()))?;

        Ok(())
    }

    async fn start_idle(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
    ) -> Result<(), EmailError> {
        if account.provider == Provider::Gmail {
            return Ok(());
        }

        let provider = account.provider.clone();
        let folder = folder_path.to_string();
        let settings = settings.clone();
        task::spawn_blocking(move || start_idle_imap(provider, &settings, &folder))
            .await
            .map_err(|err| EmailError::Data(format!("imap idle task failed: {err}")))??;

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct EwsBackend {
    http: reqwest::Client,
}

impl EwsBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl EmailBackend for EwsBackend {
    async fn sync_folders(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
    ) -> Result<Vec<MailFolder>, EmailError> {
        let endpoint = settings
            .endpoint
            .as_deref()
            .ok_or_else(|| EmailError::Data("missing EWS endpoint".to_string()))?;

        let mut request = self
            .http
            .post(endpoint)
            .header("Content-Type", "text/xml")
            .body(
                r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
<soap:Body>
  <FindFolder xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" Traversal="Shallow">
    <FolderShape><t:BaseShape>Default</t:BaseShape></FolderShape>
    <ParentFolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></ParentFolderIds>
  </FindFolder>
</soap:Body>
</soap:Envelope>"#,
            );

        request = apply_ews_auth(request, settings);
        let response = request.send().await?;

        if response.status() != StatusCode::OK {
            return Err(EmailError::Data(format!(
                "EWS sync failed with status {}",
                response.status()
            )));
        }

        let text = response.text().await?;
        let folders = parse_ews_folders(account.id, &text);
        if folders.is_empty() {
            return Ok(default_ews_folders(account.id));
        }

        Ok(folders)
    }

    async fn fetch_recent(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
        limit: usize,
    ) -> Result<FetchResult, EmailError> {
        let endpoint = settings
            .endpoint
            .as_deref()
            .ok_or_else(|| EmailError::Data("missing EWS endpoint".to_string()))?;

        let folder = ews_distinguished_folder(folder_path);
    let mut restriction_block = String::new();
    if let Some(cove_core::OfflineSyncLimit::Days(days)) = settings.offline_sync_limit {
        let after_date = Utc::now() - chrono::Duration::days(days as i64);
        restriction_block = format!(
            r#"    <Restriction>
      <t:IsGreaterThanOrEqualTo>
        <t:FieldURI FieldURI="item:DateTimeReceived" />
        <t:FieldURIOrConstant>
          <t:Constant Value="{}" />
        </t:FieldURIOrConstant>
      </t:IsGreaterThanOrEqualTo>
    </Restriction>"#,
            after_date.format("%Y-%m-%dT%H:%M:%SZ")
        );
    }

    let soap = format!(
        r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
<soap:Body>
  <FindItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" Traversal="Shallow">
    <ItemShape><t:BaseShape>AllProperties</t:BaseShape></ItemShape>
    <IndexedPageItemView MaxEntriesReturned="{limit}" Offset="0" BasePoint="Beginning"/>
{restriction_block}
    <ParentFolderIds><t:DistinguishedFolderId Id="{folder}"/></ParentFolderIds>
  </FindItem>
</soap:Body>
</soap:Envelope>"#,
    );

        let mut request = self
            .http
            .post(endpoint)
            .header("Content-Type", "text/xml")
            .body(soap);
        request = apply_ews_auth(request, settings);
        let response = request.send().await?;

        if response.status() != StatusCode::OK {
            return Err(EmailError::Data(format!(
                "EWS fetch failed with status {}",
                response.status()
            )));
        }

        let text = response.text().await?;
        Ok(FetchResult {
            messages: parse_ews_messages(account.id, folder_path, &text),
            attachment_content: Vec::new(), // EWS attachment content not yet implemented
        })
    }

    async fn send_mail(
        &self,
        _account: &Account,
        settings: &ProtocolSettings,
        outgoing: &OutgoingMail,
    ) -> Result<(), EmailError> {
        let endpoint = settings
            .endpoint
            .as_deref()
            .ok_or_else(|| EmailError::Data("missing EWS endpoint".to_string()))?;

        let to_recipients = outgoing
            .to
            .iter()
            .map(|addr| {
                format!(
                    "<t:Mailbox><t:Name>{}</t:Name><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>",
                    escape_xml(addr.name.clone().unwrap_or_default().as_str()),
                    escape_xml(&addr.address)
                )
            })
            .collect::<Vec<_>>()
            .join("");

        let body_content = outgoing
            .body_html
            .as_deref()
            .unwrap_or(outgoing.body_text.as_str());
        let body_type = if outgoing.body_html.is_some() {
            "HTML"
        } else {
            "Text"
        };

        let soap = format!(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
<soap:Body>
  <CreateItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" MessageDisposition="SendAndSaveCopy">
    <SavedItemFolderId><t:DistinguishedFolderId Id="sentitems"/></SavedItemFolderId>
    <Items>
      <t:Message>
        <t:Subject>{}</t:Subject>
        <t:Body BodyType="{body_type}">{}</t:Body>
        <t:ToRecipients>{to_recipients}</t:ToRecipients>
      </t:Message>
    </Items>
  </CreateItem>
</soap:Body>
</soap:Envelope>"#,
            escape_xml(&outgoing.subject),
            escape_xml(body_content),
        );

        let mut request = self
            .http
            .post(endpoint)
            .header("Content-Type", "text/xml")
            .body(soap);
        request = apply_ews_auth(request, settings);
        let response = request.send().await?;

        if response.status() != StatusCode::OK {
            return Err(EmailError::Data(format!(
                "EWS send failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn start_idle(
        &self,
        _account: &Account,
        _settings: &ProtocolSettings,
        _folder_path: &str,
    ) -> Result<(), EmailError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct JmapBackend {
    http: reqwest::Client,
}

impl JmapBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl EmailBackend for JmapBackend {
    async fn sync_folders(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
    ) -> Result<Vec<MailFolder>, EmailError> {
        let (api_url, mail_account, _) = jmap_session(&self.http, settings).await?;
        let response = jmap_request(
            &self.http,
            &api_url,
            settings,
            serde_json::json!({
                "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                "methodCalls": [
                    ["Mailbox/query", {"accountId": mail_account, "sort": [{"property":"name"}]}, "m1"],
                    ["Mailbox/get", {
                        "accountId": mail_account,
                        "#ids": {"resultOf": "m1", "name": "Mailbox/query", "path": "/ids"}
                    }, "m2"]
                ]
            }),
        )
        .await?;

        Ok(parse_jmap_folders(account.id, &response))
    }

    async fn fetch_recent(
        &self,
        account: &Account,
        settings: &ProtocolSettings,
        folder_path: &str,
        limit: usize,
    ) -> Result<FetchResult, EmailError> {
        let (api_url, mail_account, _) = jmap_session(&self.http, settings).await?;
        
        let mut filter = serde_json::json!({
            "inMailbox": {"resultOf":"m1", "name":"Mailbox/query", "path":"/ids/0"}
        });

        if let Some(cove_core::OfflineSyncLimit::Days(days)) = settings.offline_sync_limit {
            let after_date = Utc::now() - chrono::Duration::days(days as i64);
            filter.as_object_mut().unwrap().insert(
                "after".to_string(),
                serde_json::json!(after_date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            );
        }

        let response = jmap_request(
            &self.http,
            &api_url,
            settings,
            serde_json::json!({
                "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                "methodCalls": [
                    ["Mailbox/query", {"accountId": mail_account, "filter": {"name": folder_path}, "limit": 1}, "m1"],
                    ["Email/query", {
                        "accountId": mail_account,
                        "filter": filter,
                        "sort": [{"property":"receivedAt", "isAscending": false}],
                        "limit": limit
                    }, "m2"],
                    ["Email/get", {
                        "accountId": mail_account,
                        "#ids": {"resultOf":"m2", "name":"Email/query", "path":"/ids"},
                        "properties": ["id","threadId","subject","from","to","cc","bcc","replyTo","preview","keywords","receivedAt","sentAt","textBody","htmlBody","bodyValues"],
                        "fetchTextBodyValues": true,
                        "fetchHTMLBodyValues": true
                    }, "m3"]
                ]
            }),
        )
        .await?;

        Ok(FetchResult {
            messages: parse_jmap_messages(account.id, folder_path, &response),
            attachment_content: Vec::new(), // JMAP attachment content not yet implemented
        })
    }

    async fn send_mail(
        &self,
        _account: &Account,
        settings: &ProtocolSettings,
        outgoing: &OutgoingMail,
    ) -> Result<(), EmailError> {
        let (api_url, mail_account, submission_account) =
            jmap_session(&self.http, settings).await?;

        let get_drafts = jmap_request(
            &self.http,
            &api_url,
            settings,
            serde_json::json!({
                "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                "methodCalls": [
                    ["Mailbox/query", {"accountId": mail_account, "filter": {"role":"drafts"}, "limit": 1}, "m1"]
                ]
            }),
        )
        .await?;

        let draft_mailbox = jmap_first_id(&get_drafts, "Mailbox/query")
            .ok_or_else(|| EmailError::Data("JMAP draft mailbox not found".to_string()))?;

        let to_addresses = outgoing
            .to
            .iter()
            .map(|addr| {
                serde_json::json!({
                    "name": addr.name.clone().unwrap_or_default(),
                    "email": addr.address
                })
            })
            .collect::<Vec<_>>();

        let cc_addresses = outgoing
            .cc
            .iter()
            .map(|addr| {
                serde_json::json!({
                    "name": addr.name.clone().unwrap_or_default(),
                    "email": addr.address
                })
            })
            .collect::<Vec<_>>();

        let bcc_addresses = outgoing
            .bcc
            .iter()
            .map(|addr| {
                serde_json::json!({
                    "name": addr.name.clone().unwrap_or_default(),
                    "email": addr.address
                })
            })
            .collect::<Vec<_>>();

        let mut mailbox_ids = serde_json::Map::new();
        mailbox_ids.insert(draft_mailbox.clone(), serde_json::Value::Bool(true));

        let payload = serde_json::json!({
            "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"],
            "methodCalls": [
                ["Email/set", {
                    "accountId": mail_account,
                    "create": {
                        "draft1": {
                            "mailboxIds": serde_json::Value::Object(mailbox_ids),
                            "from": [{"name": outgoing.from.name.clone().unwrap_or_default(), "email": outgoing.from.address}],
                            "to": to_addresses,
                            "cc": cc_addresses,
                            "bcc": bcc_addresses,
                            "subject": outgoing.subject,
                            "textBody": [{"partId": "1", "type":"text/plain", "value": outgoing.body_text}],
                            "htmlBody": outgoing.body_html.as_ref().map(|html| vec![serde_json::json!({"partId":"2","type":"text/html","value": html})]).unwrap_or_default(),
                            "keywords": {"$draft": true}
                        }
                    }
                }, "m1"],
                ["EmailSubmission/set", {
                    "accountId": submission_account,
                    "create": {
                        "send1": {
                            "emailId": "#draft1"
                        }
                    },
                    "onSuccessDestroyEmail": ["#draft1"]
                }, "m2"]
            ]
        });

        let response = jmap_request(&self.http, &api_url, settings, payload).await?;
        if jmap_has_error(&response) {
            return Err(EmailError::Data(
                "JMAP send returned method error".to_string(),
            ));
        }

        Ok(())
    }

    async fn start_idle(
        &self,
        _account: &Account,
        _settings: &ProtocolSettings,
        _folder_path: &str,
    ) -> Result<(), EmailError> {
        Ok(())
    }
}

pub fn default_protocol_for_provider(provider: &Provider) -> &'static str {
    match provider {
        Provider::Exchange => "ews",
        Provider::FastMail => "jmap",
        Provider::Generic
        | Provider::Gmail
        | Provider::Outlook
        | Provider::Yahoo
        | Provider::ICloud
        | Provider::ProtonBridge => "imap_smtp",
    }
}

#[derive(Debug, Deserialize)]
struct GmailLabelListResponse {
    labels: Option<Vec<GmailLabel>>,
}

#[derive(Debug, Deserialize)]
struct GmailLabel {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "messagesUnread")]
    messages_unread: Option<u32>,
    #[serde(rename = "messagesTotal")]
    messages_total: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GmailListMessagesResponse {
    messages: Option<Vec<GmailMessageRef>>,
}

#[derive(Debug, Deserialize)]
struct GmailMessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GmailMessageRawResponse {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
    raw: Option<String>,
    snippet: Option<String>,
}

async fn sync_gmail_folders(
    account: &Account,
    settings: &ProtocolSettings,
) -> Result<Vec<MailFolder>, EmailError> {
    let token = settings
        .access_token
        .as_ref()
        .ok_or_else(|| EmailError::Data("missing Gmail access token".to_string()))?;

    let response = reqwest::Client::new()
        .get("https://gmail.googleapis.com/gmail/v1/users/me/labels")
        .bearer_auth(token)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(EmailError::Data(format!(
            "Gmail labels sync failed with status {}",
            response.status()
        )));
    }

    let payload: GmailLabelListResponse = response.json().await?;
    let folders = payload
        .labels
        .unwrap_or_default()
        .into_iter()
        .map(|label| MailFolder {
            account_id: account.id,
            remote_id: label.id.clone().unwrap_or_else(|| "unknown".to_string()),
            path: label.name.unwrap_or_else(|| "UNKNOWN".to_string()),
            delimiter: Some("/".to_string()),
            unread_count: label.messages_unread.unwrap_or(0),
            total_count: label.messages_total.unwrap_or(0),
        })
        .collect::<Vec<_>>();

    Ok(folders)
}

async fn fetch_recent_gmail(
    account: &Account,
    settings: &ProtocolSettings,
    folder_path: &str,
    limit: usize,
) -> Result<FetchResult, EmailError> {
    let token = settings
        .access_token
        .as_ref()
        .ok_or_else(|| EmailError::Data("missing Gmail access token".to_string()))?;

    let client = reqwest::Client::new();
    let mut query = vec![("maxResults", limit.to_string())];
    
    // Gmail API uses `q` for general search queries, we can use `label:` and `after:`
    let mut q_string = format!("label:{}", folder_path);
    if let Some(cove_core::OfflineSyncLimit::Days(days)) = settings.offline_sync_limit {
        let after_date = Utc::now() - chrono::Duration::days(days as i64);
        q_string.push_str(&format!(" after:{}", after_date.format("%Y/%m/%d")));
    }
    
    query.push(("q", q_string));

    let list = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/messages")
        .bearer_auth(token)
        .query(&query)
        .send()
        .await?;

    if !list.status().is_success() {
        return Err(EmailError::Data(format!(
            "Gmail list messages failed with status {}",
            list.status()
        )));
    }

    let list_payload: GmailListMessagesResponse = list.json().await?;
    let mut messages = Vec::new();
    let mut all_attachment_content: Vec<(Uuid, Uuid, Vec<u8>)> = Vec::new();

    for item in list_payload.messages.unwrap_or_default() {
        let detail = client
            .get(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}",
                item.id
            ))
            .bearer_auth(token)
            .query(&[("format", "raw")])
            .send()
            .await?;

        if !detail.status().is_success() {
            continue;
        }

        let payload: GmailMessageRawResponse = detail.json().await?;
        let raw = match payload.raw {
            Some(raw) => raw,
            None => continue,
        };

        let decoded = decode_gmail_raw(&raw)?;
        let parsed = parse_mail(&decoded)?;
        let now = Utc::now();

        let headers = headers_map(&parsed);
        let subject =
            header_value(&parsed, "Subject").unwrap_or_else(|| "(No subject)".to_string());
        let message_id = header_value(&parsed, "Message-ID").unwrap_or_else(|| payload.id.clone());
        let body_text = extract_text_body(&parsed);
        let body_html = extract_html_body(&parsed).map(|html| ammonia::clean(&html));
        let preview = payload.snippet.clone().unwrap_or_else(|| {
            body_text
                .clone()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        });

        let label_ids = payload.label_ids.unwrap_or_default();
        let (attachments, att_content) = extract_attachments(&parsed);
        let sent_at = parsed_message_date(&parsed);
        let received_at = payload
            .internal_date
            .as_deref()
            .and_then(parse_gmail_internal_date)
            .or(sent_at)
            .unwrap_or(now);

        let msg_id = Uuid::new_v4();
        for (att_id, bytes) in att_content {
            all_attachment_content.push((att_id, msg_id, bytes));
        }

        let message = MailMessage {
            id: msg_id,
            account_id: account.id,
            remote_id: payload.id.clone(),
            thread_id: payload
                .thread_id
                .unwrap_or_else(|| thread_id_from_headers(&headers, &message_id)),
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
            flags: MailFlags {
                seen: !label_ids.iter().any(|label| label == "UNREAD"),
                answered: label_ids.iter().any(|label| label == "ANSWERED"),
                flagged: label_ids.iter().any(|label| label == "STARRED"),
                deleted: label_ids.iter().any(|label| label == "TRASH"),
                draft: label_ids.iter().any(|label| label == "DRAFT"),
                forwarded: false,
            },
            labels: label_ids,
            headers,
            attachments,
            sent_at,
            received_at,
            created_at: now,
            updated_at: now,
        };

        messages.push(message);
    }

    Ok(FetchResult {
        messages,
        attachment_content: all_attachment_content,
    })
}

fn decode_gmail_raw(raw: &str) -> Result<Vec<u8>, EmailError> {
    URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .or_else(|_| URL_SAFE.decode(raw.as_bytes()))
        .map_err(|err| EmailError::Data(format!("invalid Gmail raw payload: {err}")))
}

fn headers_map(parsed: &ParsedMail<'_>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for header in parsed.get_headers() {
        headers.insert(header.get_key().to_string(), header.get_value());
    }
    headers
}

fn header_value(mail: &ParsedMail<'_>, key: &str) -> Option<String> {
    for header in mail.get_headers() {
        if header.get_key_ref().eq_ignore_ascii_case(key) {
            return Some(header.get_value());
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

fn parsed_message_date(parsed: &ParsedMail<'_>) -> Option<chrono::DateTime<Utc>> {
    let raw = header_value(parsed, "Date")?;
    let timestamp = mailparse::dateparse(&raw).ok()?;
    Utc.timestamp_opt(timestamp, 0).single()
}

fn parse_gmail_internal_date(raw: &str) -> Option<chrono::DateTime<Utc>> {
    let millis = raw.parse::<i64>().ok()?;
    Utc.timestamp_millis_opt(millis).single()
}

fn thread_id_from_headers(headers: &BTreeMap<String, String>, fallback: &str) -> String {
    headers
        .get("References")
        .or_else(|| headers.get("In-Reply-To"))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

#[derive(Clone)]
struct XOAuth2Authenticator {
    user: String,
    access_token: String,
}

impl imap::Authenticator for XOAuth2Authenticator {
    type Response = String;

    fn process(&self, _data: &[u8]) -> Self::Response {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.user, self.access_token
        )
    }
}

fn sync_folders_imap(
    account_id: Uuid,
    provider: Provider,
    settings: &ProtocolSettings,
) -> Result<Vec<MailFolder>, EmailError> {
    let mut session = connect_imap_session(settings, &provider)?;
    let names = session.list(None, Some("*")).map_err(imap_error_to_email)?;

    let mut folders = Vec::new();
    for name in names.iter() {
        let path = name.name().to_string();
        folders.push(MailFolder {
            account_id,
            remote_id: path.clone(),
            path,
            delimiter: name.delimiter().map(str::to_string),
            unread_count: 0,
            total_count: 0,
        });
    }

    if folders.is_empty() {
        folders.push(MailFolder {
            account_id,
            remote_id: "INBOX".to_string(),
            path: "INBOX".to_string(),
            delimiter: Some("/".to_string()),
            unread_count: 0,
            total_count: 0,
        });
    }

    let _ = session.logout();
    Ok(folders)
}

fn fetch_recent_imap(
    account_id: Uuid,
    provider: Provider,
    settings: &ProtocolSettings,
    folder_path: &str,
    limit: usize,
) -> Result<FetchResult, EmailError> {
    let mut session = connect_imap_session(settings, &provider)?;
    let mailbox = session.select(folder_path).map_err(imap_error_to_email)?;
    let _ = mailbox; // Kept to ensure mailbox selection succeeded

    let sequence = if let Some(cove_core::OfflineSyncLimit::Days(days)) = settings.offline_sync_limit {
        let after_date = Utc::now() - chrono::Duration::days(days as i64);
        let date_str = after_date.format("%d-%b-%Y").to_string();
        
        // Search for message UIDs since the given date
        let uids = session.uid_search(format!("SINCE {}", date_str)).map_err(imap_error_to_email)?;
        if uids.is_empty() {
            let _ = session.logout();
            return Ok(FetchResult { messages: Vec::new(), attachment_content: Vec::new() });
        }
        
        // Convert the set of UIDs to a comma-separated string, taking up to `limit` many
        let mut uid_vec: Vec<u32> = uids.into_iter().collect();
        uid_vec.sort_unstable(); // Sort to get the most recent ones if we truncate
        if uid_vec.len() > limit {
            uid_vec = uid_vec.into_iter().rev().take(limit).collect();
        }
        uid_vec.into_iter().map(|uid| uid.to_string()).collect::<Vec<String>>().join(",")
    } else {
        if mailbox.exists == 0 {
            let _ = session.logout();
            return Ok(FetchResult { messages: Vec::new(), attachment_content: Vec::new() });
        }
        let start = if mailbox.exists > limit as u32 {
            mailbox.exists - limit as u32 + 1
        } else {
            1
        };
        format!("{start}:{}", mailbox.exists)
    };

    let fetches = session
        .uid_fetch(sequence, "(UID FLAGS INTERNALDATE RFC822)")
        .map_err(imap_error_to_email)?;

    let mut messages = Vec::new();
    let mut all_attachment_content = Vec::new();
    for fetched in fetches.iter() {
        let body = match fetched.body() {
            Some(body) => body,
            None => continue,
        };

        let parsed = parse_mail(body)?;
        let headers = headers_map(&parsed);
        let subject =
            header_value(&parsed, "Subject").unwrap_or_else(|| "(No subject)".to_string());
        let message_id = header_value(&parsed, "Message-ID").unwrap_or_else(|| {
            fetched
                .uid
                .map(|uid| uid.to_string())
                .unwrap_or_else(|| fetched.message.to_string())
        });
        let body_text = extract_text_body(&parsed);
        let body_html = extract_html_body(&parsed).map(|html| ammonia::clean(&html));
        let preview = body_text
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();

        let mut flags = MailFlags::default();
        for flag in fetched.flags() {
            match flag {
                imap::types::Flag::Seen => flags.seen = true,
                imap::types::Flag::Answered => flags.answered = true,
                imap::types::Flag::Flagged => flags.flagged = true,
                imap::types::Flag::Deleted => flags.deleted = true,
                imap::types::Flag::Draft => flags.draft = true,
                _ => {}
            }
        }

        let sent_at = parsed_message_date(&parsed);
        let received_at = fetched
            .internal_date()
            .map(|datetime| datetime.with_timezone(&Utc))
            .or(sent_at)
            .unwrap_or_else(Utc::now);

        let (attachments, att_content) = extract_attachments(&parsed);
        let msg_id = Uuid::new_v4();
        for (att_id, bytes) in att_content {
            all_attachment_content.push((att_id, msg_id, bytes));
        }

        messages.push(MailMessage {
            id: msg_id,
            account_id,
            remote_id: fetched
                .uid
                .map(|uid| uid.to_string())
                .unwrap_or_else(|| fetched.message.to_string()),
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
            flags,
            labels: Vec::new(),
            headers,
            attachments,
            sent_at,
            received_at,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    }

    let _ = session.logout();
    Ok(FetchResult {
        messages,
        attachment_content: all_attachment_content,
    })
}

fn start_idle_imap(
    provider: Provider,
    settings: &ProtocolSettings,
    folder_path: &str,
) -> Result<(), EmailError> {
    let mut session = connect_imap_session(settings, &provider)?;
    session.select(folder_path).map_err(imap_error_to_email)?;

    let _ = session.noop().map_err(imap_error_to_email)?;
    let _ = session
        .idle()
        .timeout(Duration::from_secs(55))
        .keepalive(false)
        .wait_while(|_| false)
        .map_err(imap_error_to_email)?;

    let _ = session.logout();
    Ok(())
}

fn connect_imap_session(
    settings: &ProtocolSettings,
    provider: &Provider,
) -> Result<imap::Session<imap::Connection>, EmailError> {
    let host = settings
        .imap_host
        .as_deref()
        .ok_or_else(|| EmailError::Data("missing imap_host".to_string()))?;
    let port = settings.imap_port.unwrap_or(993);
    let builder = imap::ClientBuilder::new(host, port);
    let client = builder.connect().map_err(imap_error_to_email)?;
    login_imap_client(client, settings, provider)
}

fn login_imap_client(
    client: imap::Client<imap::Connection>,
    settings: &ProtocolSettings,
    provider: &Provider,
) -> Result<imap::Session<imap::Connection>, EmailError> {
    if let Some(token) = settings.access_token.as_ref() {
        let auth = XOAuth2Authenticator {
            user: settings.username.clone(),
            access_token: token.clone(),
        };

        match client.authenticate("XOAUTH2", &auth) {
            Ok(session) => return Ok(session),
            Err((auth_err, fallback_client)) => {
                if matches!(provider, Provider::Gmail) {
                    return Err(imap_error_to_email(auth_err));
                }

                if let Some(password) = settings.password.as_ref() {
                    return fallback_client
                        .login(settings.username.clone(), password.clone())
                        .map_err(|err| imap_error_to_email(err.0));
                }

                return fallback_client
                    .login(settings.username.clone(), token.clone())
                    .map_err(|err| imap_error_to_email(err.0));
            }
        }
    }

    if let Some(password) = settings.password.as_ref() {
        return client
            .login(settings.username.clone(), password.clone())
            .map_err(|err| imap_error_to_email(err.0));
    }

    Err(EmailError::Data(
        "missing authentication material for IMAP login".to_string(),
    ))
}

fn imap_error_to_email(error: imap::Error) -> EmailError {
    EmailError::Data(format!("imap error: {error}"))
}

fn apply_ews_auth(
    request: reqwest::RequestBuilder,
    settings: &ProtocolSettings,
) -> reqwest::RequestBuilder {
    if let Some(token) = settings.access_token.as_ref() {
        request.bearer_auth(token)
    } else if let Some(password) = settings.password.as_ref() {
        request.basic_auth(settings.username.clone(), Some(password.clone()))
    } else {
        request
    }
}

fn default_ews_folders(account_id: Uuid) -> Vec<MailFolder> {
    ["Inbox", "Sent Items", "Drafts", "Archive", "Deleted Items"]
        .into_iter()
        .map(|name| MailFolder {
            account_id,
            remote_id: name.to_ascii_lowercase().replace(' ', "_"),
            path: name.to_string(),
            delimiter: Some("/".to_string()),
            unread_count: 0,
            total_count: 0,
        })
        .collect()
}

fn parse_ews_folders(account_id: Uuid, payload: &str) -> Vec<MailFolder> {
    let pattern = Regex::new(r"(?s)<t:DisplayName>(.*?)</t:DisplayName>")
        .expect("valid EWS display name regex");
    pattern
        .captures_iter(payload)
        .filter_map(|capture| capture.get(1).map(|m| unescape_xml_entities(m.as_str())))
        .map(|name| MailFolder {
            account_id,
            remote_id: name.to_ascii_lowercase().replace(' ', "_"),
            path: name,
            delimiter: Some("/".to_string()),
            unread_count: 0,
            total_count: 0,
        })
        .collect()
}

fn ews_distinguished_folder(folder_path: &str) -> &'static str {
    match folder_path.to_ascii_lowercase().as_str() {
        "sent" | "sent items" => "sentitems",
        "drafts" => "drafts",
        "archive" => "archiveinbox",
        "trash" | "deleted items" => "deleteditems",
        _ => "inbox",
    }
}

fn parse_ews_messages(account_id: Uuid, folder_path: &str, payload: &str) -> Vec<MailMessage> {
    let message_re =
        Regex::new(r"(?s)<t:Message>(.*?)</t:Message>").expect("valid EWS message regex");
    let id_re = Regex::new(r#"ItemId[^>]*\bId=\"([^\"]+)\""#).expect("valid EWS id regex");
    let subject_re =
        Regex::new(r"(?s)<t:Subject>(.*?)</t:Subject>").expect("valid EWS subject regex");
    let body_re = Regex::new(r"(?s)<t:Body[^>]*>(.*?)</t:Body>").expect("valid EWS body regex");
    let preview_re = Regex::new(r"(?s)<t:BodyPreview>(.*?)</t:BodyPreview>")
        .expect("valid EWS body preview regex");
    let sent_re = Regex::new(r"(?s)<t:DateTimeSent>(.*?)</t:DateTimeSent>")
        .expect("valid EWS sent date regex");
    let received_re = Regex::new(r"(?s)<t:DateTimeReceived>(.*?)</t:DateTimeReceived>")
        .expect("valid EWS received date regex");
    let from_re = Regex::new(r"(?s)<t:From>.*?<t:EmailAddress>(.*?)</t:EmailAddress>.*?</t:From>")
        .expect("valid EWS from regex");
    let to_block_re = Regex::new(r"(?s)<t:ToRecipients>(.*?)</t:ToRecipients>")
        .expect("valid EWS to block regex");
    let addr_re =
        Regex::new(r"(?s)<t:EmailAddress>(.*?)</t:EmailAddress>").expect("valid EWS address regex");

    let mut messages = Vec::new();
    for capture in message_re.captures_iter(payload) {
        let block = match capture.get(1) {
            Some(value) => value.as_str(),
            None => continue,
        };

        let remote_id = id_re
            .captures(block)
            .and_then(|caps| caps.get(1).map(|value| value.as_str().to_string()))
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let subject = subject_re
            .captures(block)
            .and_then(|caps| {
                caps.get(1)
                    .map(|value| unescape_xml_entities(value.as_str()))
            })
            .unwrap_or_else(|| "(No subject)".to_string());
        let body_text = body_re.captures(block).and_then(|caps| {
            caps.get(1)
                .map(|value| unescape_xml_entities(value.as_str()))
        });
        let preview = preview_re
            .captures(block)
            .and_then(|caps| {
                caps.get(1)
                    .map(|value| unescape_xml_entities(value.as_str()))
            })
            .or_else(|| {
                body_text
                    .as_deref()
                    .map(|text| text.chars().take(200).collect::<String>())
            })
            .unwrap_or_default();
        let sent_at = sent_re
            .captures(block)
            .and_then(|caps| caps.get(1))
            .and_then(|value| parse_ews_datetime(value.as_str()));
        let received_at = received_re
            .captures(block)
            .and_then(|caps| caps.get(1))
            .and_then(|value| parse_ews_datetime(value.as_str()))
            .or(sent_at)
            .unwrap_or_else(Utc::now);

        let from = from_re
            .captures(block)
            .and_then(|caps| caps.get(1))
            .map(|value| {
                vec![MailAddress {
                    name: None,
                    address: unescape_xml_entities(value.as_str()),
                }]
            })
            .unwrap_or_default();
        let to = to_block_re
            .captures(block)
            .and_then(|caps| caps.get(1).map(|value| value.as_str().to_string()))
            .map(|section| {
                addr_re
                    .captures_iter(&section)
                    .filter_map(|addr| {
                        addr.get(1).map(|value| MailAddress {
                            name: None,
                            address: unescape_xml_entities(value.as_str()),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let now = Utc::now();
        messages.push(MailMessage {
            id: Uuid::new_v4(),
            account_id,
            remote_id: remote_id.clone(),
            thread_id: remote_id,
            folder_path: folder_path.to_string(),
            from,
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject,
            preview,
            body_text,
            body_html: None,
            flags: MailFlags::default(),
            labels: Vec::new(),
            headers: BTreeMap::new(),
            attachments: Vec::new(),
            sent_at,
            received_at,
            created_at: now,
            updated_at: now,
        });
    }

    messages
}

fn parse_ews_datetime(raw: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn unescape_xml_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn jmap_session(
    http: &reqwest::Client,
    settings: &ProtocolSettings,
) -> Result<(String, String, String), EmailError> {
    let endpoint = settings
        .endpoint
        .as_deref()
        .ok_or_else(|| EmailError::Data("missing JMAP endpoint".to_string()))?;
    let token = settings
        .access_token
        .as_deref()
        .ok_or_else(|| EmailError::Data("missing JMAP access token".to_string()))?;

    let response = http.get(endpoint).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(EmailError::Data(format!(
            "JMAP session failed with status {}",
            response.status()
        )));
    }

    let payload: serde_json::Value = response.json().await?;
    let api_url = payload
        .get("apiUrl")
        .and_then(|value| value.as_str())
        .ok_or_else(|| EmailError::Data("JMAP session missing apiUrl".to_string()))?
        .to_string();

    let primary = payload
        .get("primaryAccounts")
        .and_then(|value| value.as_object())
        .ok_or_else(|| EmailError::Data("JMAP session missing primaryAccounts".to_string()))?;

    let mail_account = primary
        .get("urn:ietf:params:jmap:mail")
        .and_then(|value| value.as_str())
        .or_else(|| primary.values().find_map(|value| value.as_str()))
        .ok_or_else(|| EmailError::Data("JMAP session missing mail account".to_string()))?
        .to_string();

    let submission_account = primary
        .get("urn:ietf:params:jmap:submission")
        .and_then(|value| value.as_str())
        .unwrap_or(mail_account.as_str())
        .to_string();

    Ok((api_url, mail_account, submission_account))
}

async fn jmap_request(
    http: &reqwest::Client,
    api_url: &str,
    settings: &ProtocolSettings,
    payload: serde_json::Value,
) -> Result<serde_json::Value, EmailError> {
    let token = settings
        .access_token
        .as_deref()
        .ok_or_else(|| EmailError::Data("missing JMAP access token".to_string()))?;

    let response = http
        .post(api_url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(EmailError::Data(format!(
            "JMAP method call failed with status {}",
            response.status()
        )));
    }

    response
        .json::<serde_json::Value>()
        .await
        .map_err(EmailError::from)
}

fn parse_jmap_folders(account_id: Uuid, payload: &serde_json::Value) -> Vec<MailFolder> {
    let methods = payload
        .get("methodResponses")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for method in methods {
        let Some(parts) = method.as_array() else {
            continue;
        };
        if parts.first().and_then(|value| value.as_str()) != Some("Mailbox/get") {
            continue;
        }

        let list = parts
            .get(1)
            .and_then(|value| value.get("list"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        return list
            .into_iter()
            .filter_map(|entry| {
                let id = entry.get("id").and_then(|value| value.as_str())?;
                let name = entry
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Mailbox");
                let unread = entry
                    .get("unreadEmails")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let total = entry
                    .get("totalEmails")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);

                Some(MailFolder {
                    account_id,
                    remote_id: id.to_string(),
                    path: name.to_string(),
                    delimiter: Some("/".to_string()),
                    unread_count: unread as u32,
                    total_count: total as u32,
                })
            })
            .collect();
    }

    Vec::new()
}

fn parse_jmap_messages(
    account_id: Uuid,
    folder_path: &str,
    payload: &serde_json::Value,
) -> Vec<MailMessage> {
    let methods = payload
        .get("methodResponses")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for method in methods {
        let Some(parts) = method.as_array() else {
            continue;
        };
        if parts.first().and_then(|value| value.as_str()) != Some("Email/get") {
            continue;
        }

        let list = parts
            .get(1)
            .and_then(|value| value.get("list"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        return list
            .into_iter()
            .map(|entry| {
                let id = entry
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let thread_id = entry
                    .get("threadId")
                    .and_then(|value| value.as_str())
                    .unwrap_or(id);
                let subject = entry
                    .get("subject")
                    .and_then(|value| value.as_str())
                    .unwrap_or("(No subject)")
                    .to_string();
                let preview = entry
                    .get("preview")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string();
                let headers = BTreeMap::new();
                let sent_at = entry
                    .get("sentAt")
                    .and_then(|value| value.as_str())
                    .and_then(parse_iso_datetime);
                let received_at = entry
                    .get("receivedAt")
                    .and_then(|value| value.as_str())
                    .and_then(parse_iso_datetime)
                    .or(sent_at)
                    .unwrap_or_else(Utc::now);

                let body_values = entry
                    .get("bodyValues")
                    .and_then(|value| value.as_object())
                    .cloned()
                    .unwrap_or_default();

                let body_text = pick_jmap_body(entry.get("textBody"), &body_values);
                let body_html = pick_jmap_body(entry.get("htmlBody"), &body_values);

                let mut flags = MailFlags::default();
                if let Some(keywords) = entry.get("keywords").and_then(|value| value.as_object()) {
                    flags.seen = keywords.contains_key("$seen");
                    flags.answered = keywords.contains_key("$answered");
                    flags.flagged = keywords.contains_key("$flagged");
                    flags.draft = keywords.contains_key("$draft");
                }

                let attachments = entry
                    .get("attachments")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|attachment| MailAttachment {
                        id: Uuid::new_v4(),
                        file_name: attachment
                            .get("name")
                            .and_then(|value| value.as_str())
                            .unwrap_or("attachment.bin")
                            .to_string(),
                        mime_type: attachment
                            .get("type")
                            .and_then(|value| value.as_str())
                            .unwrap_or("application/octet-stream")
                            .to_string(),
                        size: attachment
                            .get("size")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0),
                        inline: attachment
                            .get("disposition")
                            .and_then(|value| value.as_str())
                            .map(|value| value.eq_ignore_ascii_case("inline"))
                            .unwrap_or(false),
                    })
                    .collect::<Vec<_>>();

                MailMessage {
                    id: Uuid::new_v4(),
                    account_id,
                    remote_id: id.to_string(),
                    thread_id: thread_id.to_string(),
                    folder_path: folder_path.to_string(),
                    from: parse_jmap_addresses(entry.get("from")),
                    to: parse_jmap_addresses(entry.get("to")),
                    cc: parse_jmap_addresses(entry.get("cc")),
                    bcc: parse_jmap_addresses(entry.get("bcc")),
                    reply_to: parse_jmap_addresses(entry.get("replyTo")),
                    subject,
                    preview,
                    body_text,
                    body_html,
                    flags,
                    labels: Vec::new(),
                    headers,
                    attachments,
                    sent_at,
                    received_at,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }
            })
            .collect();
    }

    Vec::new()
}

fn pick_jmap_body(
    body_parts: Option<&serde_json::Value>,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let parts = body_parts?.as_array()?;
    for part in parts {
        let part_id = part.get("partId").and_then(|value| value.as_str())?;
        let value = values.get(part_id)?;
        if let Some(text) = value.get("value").and_then(|value| value.as_str()) {
            return Some(text.to_string());
        }
    }

    None
}

fn parse_jmap_addresses(value: Option<&serde_json::Value>) -> Vec<MailAddress> {
    value
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let address = entry.get("email").and_then(|value| value.as_str())?;
            Some(MailAddress {
                name: entry
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string)
                    .filter(|value| !value.is_empty()),
                address: address.to_string(),
            })
        })
        .collect()
}

fn parse_iso_datetime(raw: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn jmap_first_id(payload: &serde_json::Value, method_name: &str) -> Option<String> {
    let methods = payload.get("methodResponses")?.as_array()?;
    for method in methods {
        let parts = method.as_array()?;
        if parts.first()?.as_str()? != method_name {
            continue;
        }
        let ids = parts.get(1)?.get("ids")?.as_array()?;
        return ids
            .first()
            .and_then(|value| value.as_str())
            .map(str::to_string);
    }

    None
}

fn jmap_has_error(payload: &serde_json::Value) -> bool {
    let methods = payload
        .get("methodResponses")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    methods.into_iter().any(|method| {
        method
            .as_array()
            .and_then(|parts| parts.first())
            .and_then(|name| name.as_str())
            .map(|name| name.eq_ignore_ascii_case("error"))
            .unwrap_or(false)
    })
}

fn to_mailbox(address: &MailAddress) -> Result<Mailbox, EmailError> {
    let email = address
        .address
        .parse()
        .map_err(|err| EmailError::Build(format!("invalid email {}: {err}", address.address)))?;

    Ok(Mailbox::new(address.name.clone(), email))
}
