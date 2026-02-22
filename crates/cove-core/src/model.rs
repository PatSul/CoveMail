use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Gmail,
    Outlook,
    Yahoo,
    ICloud,
    FastMail,
    ProtonBridge,
    Generic,
    Exchange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfflineSyncLimit {
    All,
    Days(u32),
}

impl Default for OfflineSyncLimit {
    fn default() -> Self {
        Self::Days(30)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountProtocol {
    ImapSmtp,
    Ews,
    Jmap,
    CalDav,
    GoogleCalendar,
    MicrosoftGraph,
    GoogleTasks,
    MicrosoftTodo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProfile {
    pub client_id: String,
    pub auth_url: Url,
    pub token_url: Url,
    pub redirect_url: Url,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub provider: Provider,
    pub protocols: Vec<AccountProtocol>,
    pub display_name: String,
    pub email_address: String,
    pub oauth_profile: Option<OAuthProfile>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailFolder {
    pub account_id: Uuid,
    pub remote_id: String,
    pub path: String,
    pub delimiter: Option<String>,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MailFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
    pub forwarded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAttachment {
    pub id: Uuid,
    pub file_name: String,
    pub mime_type: String,
    pub size: u64,
    pub inline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailMessage {
    pub id: Uuid,
    pub account_id: Uuid,
    pub remote_id: String,
    pub thread_id: String,
    pub folder_path: String,
    pub from: Vec<MailAddress>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    pub subject: String,
    pub preview: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub flags: MailFlags,
    pub labels: Vec<String>,
    pub headers: BTreeMap<String, String>,
    pub attachments: Vec<MailAttachment>,
    pub sent_at: Option<DateTime<Utc>>,
    pub received_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Snooze: message reappears after this time.
    #[serde(default)]
    pub snoozed_until: Option<DateTime<Utc>>,
    /// Pinned messages stay at top of list.
    #[serde(default)]
    pub pinned: bool,
    /// Scheduled send time (None = send immediately).
    #[serde(default)]
    pub send_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailThreadSummary {
    pub thread_id: String,
    pub subject: String,
    pub participants: Vec<String>,
    pub message_count: usize,
    pub unread_count: usize,
    pub most_recent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactSummary {
    pub email_address: String,
    pub display_name: Option<String>,
    pub latest_subject: String,
    pub message_count: usize,
    pub unread_count: usize,
    pub most_recent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarAlarm {
    pub minutes_before: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RsvpStatus {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

impl Default for RsvpStatus {
    fn default() -> Self {
        Self::NeedsAction
    }
}

/// Human-readable recurrence frequencies for the recurrence editor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceFrequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: Uuid,
    pub account_id: Uuid,
    pub calendar_id: String,
    pub remote_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub all_day: bool,
    pub recurrence_rule: Option<String>,
    pub attendees: Vec<String>,
    pub organizer: Option<String>,
    pub alarms: Vec<CalendarAlarm>,
    pub rsvp_status: RsvpStatus,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderTask {
    pub id: Uuid,
    pub account_id: Uuid,
    pub list_id: String,
    pub remote_id: Option<String>,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub repeat_rule: Option<String>,
    pub parent_id: Option<Uuid>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---- Signatures & Templates ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSignature {
    pub id: Uuid,
    pub account_id: Option<Uuid>,
    pub name: String,
    pub body_html: String,
    pub body_text: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailTemplate {
    pub id: Uuid,
    pub name: String,
    pub subject: String,
    pub body_html: String,
    pub body_text: String,
}

// ---- Rules / Filters ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleField {
    From,
    To,
    Subject,
    Body,
    HasAttachment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleOperator {
    Contains,
    NotContains,
    Equals,
    StartsWith,
    EndsWith,
    Matches,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleCondition {
    pub field: RuleField,
    pub operator: RuleOperator,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    MoveTo(String),
    Label(String),
    MarkRead,
    Archive,
    Delete,
    Pin,
    Flag,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailRule {
    pub id: Uuid,
    pub account_id: Option<Uuid>,
    pub name: String,
    pub enabled: bool,
    pub conditions: Vec<RuleCondition>,
    pub match_all: bool,
    pub actions: Vec<RuleAction>,
    pub stop_processing: bool,
    pub order: i32,
}

// ---- Contacts ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: Uuid,
    pub account_id: Option<Uuid>,
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub organization: Option<String>,
    pub notes: Option<String>,
    pub last_contacted: Option<DateTime<Utc>>,
    pub contact_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncDomain {
    Email,
    Calendar,
    Tasks,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Queued,
    Running,
    Failed,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncJob {
    pub id: Uuid,
    pub account_id: Uuid,
    pub domain: SyncDomain,
    pub status: SyncStatus,
    pub payload_json: serde_json::Value,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub run_after: DateTime<Utc>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CloudAiProvider {
    OpenAi,
    Anthropic,
    Gemini,
    Mistral,
    Groq,
    Grok,
    OpenRouter,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRequest {
    pub feature: String,
    pub prompt: String,
    pub context: serde_json::Value,
    pub mode: AiMode,
    pub cloud_provider: Option<CloudAiProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiResponse {
    pub feature: String,
    pub output: String,
    pub mode: AiMode,
    pub provider: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataProvenance {
    pub feature: String,
    pub mode: AiMode,
    pub destination: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult<T> {
    pub total: usize,
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub limit: usize,
    pub offset: usize,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
        }
    }
}
