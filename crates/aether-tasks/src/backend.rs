use crate::TaskError;
use aether_core::{Account, ReminderTask, TaskPriority, TaskStatus};
use async_trait::async_trait;
use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSettings {
    pub endpoint: String,
    pub access_token: Option<String>,
    pub list_id: String,
}

#[async_trait]
pub trait TaskBackend: Send + Sync {
    async fn sync_tasks(
        &self,
        account: &Account,
        settings: &TaskSettings,
    ) -> Result<Vec<ReminderTask>, TaskError>;

    async fn upsert_task(
        &self,
        account: &Account,
        settings: &TaskSettings,
        task: &ReminderTask,
    ) -> Result<(), TaskError>;
}

#[derive(Debug, Default)]
pub struct CalDavTodoBackend {
    http: reqwest::Client,
}

impl CalDavTodoBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl TaskBackend for CalDavTodoBackend {
    async fn sync_tasks(
        &self,
        account: &Account,
        settings: &TaskSettings,
    ) -> Result<Vec<ReminderTask>, TaskError> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag />
    <C:calendar-data />
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VTODO" />
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#;

        let mut request = self
            .http
            .request(
                Method::from_bytes(b"REPORT").expect("valid method"),
                &settings.endpoint,
            )
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body);
        if let Some(token) = &settings.access_token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "CalDAV VTODO sync failed with status {}",
                response.status()
            )));
        }

        let payload = response.text().await?;
        Ok(parse_caldav_vtodo_data(
            account.id,
            &settings.list_id,
            &payload,
        ))
    }

    async fn upsert_task(
        &self,
        _account: &Account,
        settings: &TaskSettings,
        task: &ReminderTask,
    ) -> Result<(), TaskError> {
        let remote_id = task
            .remote_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let mut endpoint = settings.endpoint.trim_end_matches('/').to_string();
        endpoint.push('/');
        endpoint.push_str(&format!("{remote_id}.ics"));

        let ics_payload = render_single_vtodo_ics(task, &remote_id);

        let mut request = self
            .http
            .put(endpoint)
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(ics_payload);

        if let Some(token) = &settings.access_token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "CalDAV task upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct MicrosoftTodoBackend {
    http: reqwest::Client,
}

impl MicrosoftTodoBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GraphTodoResponse {
    value: Option<Vec<GraphTodoTask>>,
}

#[derive(Debug, Deserialize)]
struct GraphTodoTask {
    id: Option<String>,
    title: Option<String>,
    body: Option<GraphTaskBody>,
    status: Option<String>,
    importance: Option<String>,
    #[serde(rename = "dueDateTime")]
    due_date_time: Option<GraphDateTimeTimeZone>,
    #[serde(rename = "completedDateTime")]
    completed_date_time: Option<GraphDateTimeTimeZone>,
    #[serde(rename = "createdDateTime")]
    created_date_time: Option<String>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified_date_time: Option<String>,
    #[serde(rename = "parentTaskId")]
    parent_task_id: Option<String>,
    recurrence: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GraphTaskBody {
    content: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct GraphDateTimeTimeZone {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[async_trait]
impl TaskBackend for MicrosoftTodoBackend {
    async fn sync_tasks(
        &self,
        account: &Account,
        settings: &TaskSettings,
    ) -> Result<Vec<ReminderTask>, TaskError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| TaskError::Data("missing Graph token".to_string()))?;

        let response = self
            .http
            .get(format!(
                "https://graph.microsoft.com/v1.0/me/todo/lists/{}/tasks",
                settings.list_id
            ))
            .bearer_auth(token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "Graph tasks sync failed with status {}",
                response.status()
            )));
        }

        let payload: GraphTodoResponse = response.json().await?;
        let now = Utc::now();
        let mut tasks = Vec::new();
        for item in payload.value.unwrap_or_default() {
            let title = item
                .title
                .clone()
                .unwrap_or_else(|| "Untitled task".to_string());
            let notes = item.body.and_then(|body| body.content);
            let due_at = item.due_date_time.as_ref().and_then(parse_graph_datetime);
            let completed_at = item
                .completed_date_time
                .as_ref()
                .and_then(parse_graph_datetime);
            let created_at = item
                .created_date_time
                .as_deref()
                .and_then(parse_rfc3339)
                .unwrap_or(now);
            let updated_at = item
                .last_modified_date_time
                .as_deref()
                .and_then(parse_rfc3339)
                .or_else(|| item.created_date_time.as_deref().and_then(parse_rfc3339))
                .unwrap_or(now);

            tasks.push(ReminderTask {
                id: Uuid::new_v4(),
                account_id: account.id,
                list_id: settings.list_id.clone(),
                remote_id: item.id,
                title,
                notes,
                due_at,
                completed_at,
                priority: graph_importance_to_priority(item.importance.as_deref()),
                status: graph_status_to_task_status(item.status.as_deref()),
                repeat_rule: item.recurrence.map(|value| value.to_string()),
                parent_id: item
                    .parent_task_id
                    .as_deref()
                    .and_then(|raw| Uuid::parse_str(raw).ok()),
                snoozed_until: None,
                created_at,
                updated_at,
            });
        }

        Ok(tasks)
    }

    async fn upsert_task(
        &self,
        _account: &Account,
        settings: &TaskSettings,
        task: &ReminderTask,
    ) -> Result<(), TaskError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| TaskError::Data("missing Graph token".to_string()))?;

        let base = format!(
            "https://graph.microsoft.com/v1.0/me/todo/lists/{}/tasks",
            settings.list_id
        );

        let mut payload = serde_json::Map::new();
        payload.insert(
            "title".to_string(),
            serde_json::Value::String(task.title.clone()),
        );
        payload.insert(
            "status".to_string(),
            serde_json::Value::String(task_status_to_graph_status(&task.status).to_string()),
        );
        payload.insert(
            "importance".to_string(),
            serde_json::Value::String(priority_to_graph_importance(&task.priority).to_string()),
        );

        if let Some(notes) = task.notes.as_deref() {
            payload.insert(
                "body".to_string(),
                serde_json::json!({
                    "contentType": "text",
                    "content": notes,
                }),
            );
        }

        if let Some(due) = task.due_at {
            payload.insert(
                "dueDateTime".to_string(),
                serde_json::json!({
                    "dateTime": due.format("%Y-%m-%dT%H:%M:%S").to_string(),
                    "timeZone": "UTC",
                }),
            );
        }

        if let Some(completed) = task.completed_at {
            payload.insert(
                "completedDateTime".to_string(),
                serde_json::json!({
                    "dateTime": completed.format("%Y-%m-%dT%H:%M:%S").to_string(),
                    "timeZone": "UTC",
                }),
            );
        }

        if let Some(parent) = task.parent_id {
            payload.insert(
                "parentTaskId".to_string(),
                serde_json::Value::String(parent.to_string()),
            );
        }

        if let Some(recurrence) = task.repeat_rule.as_deref() {
            payload.insert(
                "recurrence".to_string(),
                serde_json::json!({ "pattern": { "type": recurrence } }),
            );
        }

        let response = if let Some(remote_id) = task.remote_id.as_deref() {
            self.http
                .patch(format!("{base}/{remote_id}"))
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await?
        } else {
            self.http
                .post(base)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await?
        };

        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "Graph task upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct GoogleTasksBackend {
    http: reqwest::Client,
}

impl GoogleTasksBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTaskListResponse {
    items: Option<Vec<GoogleTaskItem>>,
}

#[derive(Debug, Deserialize)]
struct GoogleTaskItem {
    id: Option<String>,
    title: Option<String>,
    notes: Option<String>,
    due: Option<String>,
    completed: Option<String>,
    status: Option<String>,
    parent: Option<String>,
    updated: Option<String>,
}

#[async_trait]
impl TaskBackend for GoogleTasksBackend {
    async fn sync_tasks(
        &self,
        account: &Account,
        settings: &TaskSettings,
    ) -> Result<Vec<ReminderTask>, TaskError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| TaskError::Data("missing Google token".to_string()))?;

        let response = self
            .http
            .get(format!(
                "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks",
                settings.list_id
            ))
            .bearer_auth(token)
            .query(&[
                ("showCompleted", "true"),
                ("showHidden", "true"),
                ("maxResults", "200"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "Google tasks sync failed with status {}",
                response.status()
            )));
        }

        let payload: GoogleTaskListResponse = response.json().await?;
        let now = Utc::now();
        let mut tasks = Vec::new();

        for item in payload.items.unwrap_or_default() {
            let title = item
                .title
                .clone()
                .unwrap_or_else(|| "Untitled task".to_string());

            let status = match item.status.as_deref() {
                Some("completed") => TaskStatus::Completed,
                Some("needsAction") | None => TaskStatus::NotStarted,
                _ => TaskStatus::InProgress,
            };

            let completed_at = item.completed.as_deref().and_then(parse_rfc3339);
            let due_at = item.due.as_deref().and_then(parse_rfc3339);
            let updated_at = item
                .updated
                .as_deref()
                .and_then(parse_rfc3339)
                .unwrap_or(now);

            let priority = infer_priority(&title, item.notes.as_deref().unwrap_or_default());

            tasks.push(ReminderTask {
                id: uuid::Uuid::new_v4(),
                account_id: account.id,
                list_id: settings.list_id.clone(),
                remote_id: item.id,
                title,
                notes: item.notes,
                due_at,
                completed_at,
                priority,
                status,
                repeat_rule: None,
                parent_id: item
                    .parent
                    .as_deref()
                    .and_then(|raw| uuid::Uuid::parse_str(raw).ok()),
                snoozed_until: None,
                created_at: updated_at,
                updated_at,
            });
        }

        Ok(tasks)
    }

    async fn upsert_task(
        &self,
        _account: &Account,
        settings: &TaskSettings,
        task: &ReminderTask,
    ) -> Result<(), TaskError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| TaskError::Data("missing Google token".to_string()))?;

        let base = format!(
            "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks",
            settings.list_id
        );

        let body = serde_json::json!({
            "title": task.title,
            "notes": task.notes,
            "due": task.due_at.map(|due| due.to_rfc3339()),
            "status": match &task.status {
                TaskStatus::Completed => "completed",
                TaskStatus::NotStarted => "needsAction",
                TaskStatus::InProgress => "needsAction",
                TaskStatus::Canceled => "needsAction",
            },
            "parent": task.parent_id.map(|parent| parent.to_string()),
        });

        let response = if let Some(remote_id) = &task.remote_id {
            self.http
                .patch(format!("{base}/{remote_id}"))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?
        } else {
            self.http
                .post(base)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?
        };

        if !response.status().is_success() {
            return Err(TaskError::Data(format!(
                "Google task upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

fn parse_caldav_vtodo_data(account_id: Uuid, list_id: &str, payload: &str) -> Vec<ReminderTask> {
    let data_re = Regex::new(
        r"(?is)<(?:[a-z0-9_]+:)?calendar-data[^>]*>(.*?)</(?:[a-z0-9_]+:)?calendar-data>",
    )
    .expect("valid CalDAV VTODO regex");

    let mut tasks = Vec::new();
    for capture in data_re.captures_iter(payload) {
        let Some(raw_ics) = capture.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let calendar_data = unescape_xml_entities(raw_ics);
        tasks.extend(parse_ical_vtodo_entries(
            account_id,
            list_id,
            &calendar_data,
        ));
    }

    tasks
}

fn parse_ical_vtodo_entries(
    account_id: Uuid,
    list_id: &str,
    ics_payload: &str,
) -> Vec<ReminderTask> {
    let lines = unfold_ical_lines(ics_payload);
    let mut tasks = Vec::new();
    let now = Utc::now();

    let mut in_todo = false;
    let mut uid: Option<String> = None;
    let mut title: Option<String> = None;
    let mut notes: Option<String> = None;
    let mut due_at: Option<DateTime<Utc>> = None;
    let mut completed_at: Option<DateTime<Utc>> = None;
    let mut priority = TaskPriority::Normal;
    let mut status = TaskStatus::NotStarted;
    let mut repeat_rule: Option<String> = None;
    let mut parent_id: Option<Uuid> = None;
    let mut created_at: Option<DateTime<Utc>> = None;
    let mut updated_at: Option<DateTime<Utc>> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("BEGIN:VTODO") {
            in_todo = true;
            uid = None;
            title = None;
            notes = None;
            due_at = None;
            completed_at = None;
            priority = TaskPriority::Normal;
            status = TaskStatus::NotStarted;
            repeat_rule = None;
            parent_id = None;
            created_at = None;
            updated_at = None;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("END:VTODO") {
            in_todo = false;
            let created = created_at.unwrap_or_else(|| updated_at.unwrap_or(now));
            let updated = updated_at.unwrap_or(created);
            tasks.push(ReminderTask {
                id: Uuid::new_v4(),
                account_id,
                list_id: list_id.to_string(),
                remote_id: uid.clone(),
                title: title.clone().unwrap_or_else(|| "Untitled task".to_string()),
                notes: notes.clone(),
                due_at,
                completed_at,
                priority: priority.clone(),
                status: status.clone(),
                repeat_rule: repeat_rule.clone(),
                parent_id,
                snoozed_until: None,
                created_at: created,
                updated_at: updated,
            });
            continue;
        }

        if !in_todo {
            continue;
        }

        let Some((raw_property, raw_value)) = trimmed.split_once(':') else {
            continue;
        };
        let property = raw_property.trim();
        let value = raw_value.trim();
        let property_upper = property.to_ascii_uppercase();

        if property_upper.starts_with("UID") {
            if !value.is_empty() {
                uid = Some(value.to_string());
            }
            continue;
        }

        if property_upper.starts_with("SUMMARY") {
            title = Some(unescape_ical_text(value));
            continue;
        }

        if property_upper.starts_with("DESCRIPTION") {
            notes = Some(unescape_ical_text(value));
            continue;
        }

        if property_upper.starts_with("DUE") {
            due_at = parse_ical_datetime_with_property(property, value);
            continue;
        }

        if property_upper.starts_with("COMPLETED") {
            completed_at = parse_ical_datetime_with_property(property, value);
            continue;
        }

        if property_upper.starts_with("STATUS") {
            status = ical_status_to_task_status(value);
            continue;
        }

        if property_upper.starts_with("PRIORITY") {
            priority = ical_priority_to_task_priority(value);
            continue;
        }

        if property_upper.starts_with("RRULE") {
            repeat_rule = Some(value.to_string());
            continue;
        }

        if property_upper.starts_with("RELATED-TO") {
            parent_id = Uuid::parse_str(value).ok();
            continue;
        }

        if property_upper.starts_with("CREATED") {
            created_at = parse_ical_datetime_with_property(property, value);
            continue;
        }

        if property_upper.starts_with("LAST-MODIFIED") || property_upper.starts_with("DTSTAMP") {
            if let Some(parsed) = parse_ical_datetime_with_property(property, value) {
                updated_at = Some(parsed);
            }
        }
    }

    tasks
}

fn render_single_vtodo_ics(task: &ReminderTask, remote_id: &str) -> String {
    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//AegisInbox//EN\r\n");
    out.push_str("BEGIN:VTODO\r\n");
    out.push_str(&format!("UID:{}\r\n", escape_ical_text(remote_id)));
    out.push_str(&format!(
        "DTSTAMP:{}\r\n",
        Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    out.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(&task.title)));
    if let Some(notes) = task.notes.as_deref() {
        out.push_str(&format!("DESCRIPTION:{}\r\n", escape_ical_text(notes)));
    }
    if let Some(due_at) = task.due_at {
        out.push_str(&format!("DUE:{}\r\n", due_at.format("%Y%m%dT%H%M%SZ")));
    }
    if let Some(completed_at) = task.completed_at {
        out.push_str(&format!(
            "COMPLETED:{}\r\n",
            completed_at.format("%Y%m%dT%H%M%SZ")
        ));
    }
    out.push_str(&format!(
        "STATUS:{}\r\n",
        task_status_to_ical_status(&task.status)
    ));
    out.push_str(&format!(
        "PRIORITY:{}\r\n",
        task_priority_to_ical_priority(&task.priority)
    ));
    if let Some(rrule) = task.repeat_rule.as_deref() {
        out.push_str(&format!("RRULE:{}\r\n", rrule));
    }
    if let Some(parent) = task.parent_id {
        out.push_str(&format!("RELATED-TO:{}\r\n", parent));
    }
    out.push_str("END:VTODO\r\nEND:VCALENDAR\r\n");
    out
}

fn parse_graph_datetime(value: &GraphDateTimeTimeZone) -> Option<DateTime<Utc>> {
    let raw = value.date_time.as_deref()?;
    if let Some(parsed) = parse_rfc3339(raw) {
        return Some(parsed);
    }

    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;

    if let Some(zone_name) = value.time_zone.as_deref() {
        if let Ok(zone) = zone_name.parse::<Tz>() {
            return match zone.from_local_datetime(&naive) {
                LocalResult::Single(datetime) => Some(datetime.with_timezone(&Utc)),
                LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
                LocalResult::None => Some(Utc.from_utc_datetime(&naive)),
            };
        }
    }

    Some(Utc.from_utc_datetime(&naive))
}

fn graph_importance_to_priority(value: Option<&str>) -> TaskPriority {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "high" => TaskPriority::High,
        "low" => TaskPriority::Low,
        _ => TaskPriority::Normal,
    }
}

fn priority_to_graph_importance(priority: &TaskPriority) -> &'static str {
    match priority {
        TaskPriority::Critical | TaskPriority::High => "high",
        TaskPriority::Low => "low",
        TaskPriority::Normal => "normal",
    }
}

fn graph_status_to_task_status(value: Option<&str>) -> TaskStatus {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "completed" => TaskStatus::Completed,
        "inprogress" => TaskStatus::InProgress,
        "waitingonothers" | "deferred" | "notstarted" | "" => TaskStatus::NotStarted,
        _ => TaskStatus::NotStarted,
    }
}

fn task_status_to_graph_status(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "completed",
        TaskStatus::InProgress => "inProgress",
        TaskStatus::Canceled => "deferred",
        TaskStatus::NotStarted => "notStarted",
    }
}

fn ical_status_to_task_status(value: &str) -> TaskStatus {
    match value.to_ascii_uppercase().as_str() {
        "COMPLETED" => TaskStatus::Completed,
        "IN-PROCESS" => TaskStatus::InProgress,
        "CANCELLED" => TaskStatus::Canceled,
        _ => TaskStatus::NotStarted,
    }
}

fn task_status_to_ical_status(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Completed => "COMPLETED",
        TaskStatus::InProgress => "IN-PROCESS",
        TaskStatus::Canceled => "CANCELLED",
        TaskStatus::NotStarted => "NEEDS-ACTION",
    }
}

fn ical_priority_to_task_priority(value: &str) -> TaskPriority {
    match value.parse::<u8>().ok() {
        Some(1) => TaskPriority::Critical,
        Some(2..=4) => TaskPriority::High,
        Some(6..=9) => TaskPriority::Low,
        _ => TaskPriority::Normal,
    }
}

fn task_priority_to_ical_priority(priority: &TaskPriority) -> u8 {
    match priority {
        TaskPriority::Critical => 1,
        TaskPriority::High => 3,
        TaskPriority::Normal => 5,
        TaskPriority::Low => 7,
    }
}

fn unfold_ical_lines(payload: &str) -> Vec<String> {
    let normalized = payload.replace("\r\n", "\n").replace('\r', "\n");
    let mut unfolded: Vec<String> = Vec::new();
    for raw_line in normalized.lines() {
        if let Some(last) = unfolded.last_mut() {
            if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
                last.push_str(raw_line.trim_start());
                continue;
            }
        }
        unfolded.push(raw_line.to_string());
    }
    unfolded
}

fn parse_ical_datetime_with_property(property: &str, value: &str) -> Option<DateTime<Utc>> {
    if let Some(parsed) = parse_rfc3339(value) {
        return Some(parsed);
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ") {
        return Some(Utc.from_utc_datetime(&value));
    }
    if let Ok(value) = NaiveDate::parse_from_str(value, "%Y%m%d") {
        let midnight = value.and_hms_opt(0, 0, 0)?;
        return Some(Utc.from_utc_datetime(&midnight));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
        if let Some(zone_name) = property_tzid(property) {
            if let Ok(zone) = zone_name.parse::<Tz>() {
                return match zone.from_local_datetime(&value) {
                    LocalResult::Single(datetime) => Some(datetime.with_timezone(&Utc)),
                    LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
                    LocalResult::None => Some(Utc.from_utc_datetime(&value)),
                };
            }
        }
        return Some(Utc.from_utc_datetime(&value));
    }

    None
}

fn property_tzid(property: &str) -> Option<String> {
    for part in property.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("TZID") {
            return Some(value.trim_matches('"').to_string());
        }
    }

    None
}

fn unescape_ical_text(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(current) = chars.next() {
        if current != '\\' {
            result.push(current);
            continue;
        }

        match chars.next() {
            Some('n') | Some('N') => result.push('\n'),
            Some('\\') => result.push('\\'),
            Some(';') => result.push(';'),
            Some(',') => result.push(','),
            Some(other) => result.push(other),
            None => {}
        }
    }
    result
}

fn escape_ical_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn unescape_xml_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn infer_priority(title: &str, notes: &str) -> TaskPriority {
    let text = format!("{title} {notes}").to_ascii_lowercase();
    if text.contains("p1") || text.contains("critical") {
        TaskPriority::Critical
    } else if text.contains("high") || text.contains("urgent") {
        TaskPriority::High
    } else if text.contains("low") {
        TaskPriority::Low
    } else {
        TaskPriority::Normal
    }
}
