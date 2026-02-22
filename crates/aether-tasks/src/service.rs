use crate::{
    CalDavTodoBackend, GoogleTasksBackend, MicrosoftTodoBackend, TaskBackend, TaskError,
    TaskSettings,
};
use aether_core::{Account, Provider, ReminderTask, TaskPriority, TaskStatus};
use aether_storage::Storage;
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NaturalTaskInput {
    pub text: String,
    pub account_id: Uuid,
    pub list_id: String,
}

#[derive(Clone)]
pub struct TaskService {
    storage: Storage,
    caldav: Arc<CalDavTodoBackend>,
    graph: Arc<MicrosoftTodoBackend>,
    google: Arc<GoogleTasksBackend>,
}

impl TaskService {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            caldav: Arc::new(CalDavTodoBackend::new()),
            graph: Arc::new(MicrosoftTodoBackend::new()),
            google: Arc::new(GoogleTasksBackend::new()),
        }
    }

    pub async fn sync_tasks(
        &self,
        account: &Account,
        settings: &TaskSettings,
    ) -> Result<Vec<ReminderTask>, TaskError> {
        let backend = self.backend_for(account);
        let tasks = backend.sync_tasks(account, settings).await?;
        for task in &tasks {
            self.storage.upsert_task(task).await?;
        }
        Ok(tasks)
    }

    pub async fn upsert_task(
        &self,
        account: &Account,
        settings: &TaskSettings,
        task: &ReminderTask,
    ) -> Result<(), TaskError> {
        self.storage.upsert_task(task).await?;
        self.backend_for(account)
            .upsert_task(account, settings, task)
            .await
    }

    pub async fn create_from_natural_language(
        &self,
        input: NaturalTaskInput,
    ) -> Result<ReminderTask, TaskError> {
        let parsed = parse_natural_task(&input.text)?;
        let now = Utc::now();

        let task = ReminderTask {
            id: Uuid::new_v4(),
            account_id: input.account_id,
            list_id: input.list_id,
            remote_id: None,
            title: parsed.title,
            notes: parsed.notes,
            due_at: parsed.due_at,
            completed_at: None,
            priority: parsed.priority,
            status: TaskStatus::NotStarted,
            repeat_rule: parsed.repeat_rule,
            parent_id: None,
            snoozed_until: parsed.snooze_until,
            created_at: now,
            updated_at: now,
        };

        self.storage.upsert_task(&task).await?;
        Ok(task)
    }

    fn backend_for(&self, account: &Account) -> Arc<dyn TaskBackend> {
        match account.provider {
            Provider::Gmail => self.google.clone(),
            Provider::Outlook | Provider::Exchange => self.graph.clone(),
            Provider::ICloud
            | Provider::FastMail
            | Provider::Yahoo
            | Provider::Generic
            | Provider::ProtonBridge => self.caldav.clone(),
        }
    }
}

#[derive(Debug)]
struct ParsedTask {
    title: String,
    notes: Option<String>,
    due_at: Option<DateTime<Utc>>,
    priority: TaskPriority,
    repeat_rule: Option<String>,
    snooze_until: Option<DateTime<Utc>>,
}

fn parse_natural_task(text: &str) -> Result<ParsedTask, TaskError> {
    let mut title = text.trim().to_string();
    let mut due_at = None;
    let mut repeat_rule = None;
    let mut snooze_until = None;

    let lowercase = text.to_ascii_lowercase();

    let due_re = Regex::new(r"\b(today|tomorrow|next week|in (\d+) days)\b")
        .map_err(|err| TaskError::Data(err.to_string()))?;
    if let Some(cap) = due_re.captures(&lowercase) {
        let now = Utc::now();
        let parsed = match cap.get(1).map(|m| m.as_str()) {
            Some("today") => Some(now + Duration::hours(2)),
            Some("tomorrow") => Some(now + Duration::days(1)),
            Some("next week") => Some(now + Duration::days(7)),
            Some(value) if value.starts_with("in ") => cap
                .get(2)
                .and_then(|days| days.as_str().parse::<i64>().ok())
                .map(|days| now + Duration::days(days)),
            _ => None,
        };
        due_at = parsed;
    }

    if lowercase.contains("every day") {
        repeat_rule = Some("FREQ=DAILY".to_string());
    } else if lowercase.contains("every week") {
        repeat_rule = Some("FREQ=WEEKLY".to_string());
    } else if lowercase.contains("every month") {
        repeat_rule = Some("FREQ=MONTHLY".to_string());
    }

    if let Some(cap) = Regex::new(r"snooze (\d+)h")
        .map_err(|err| TaskError::Data(err.to_string()))?
        .captures(&lowercase)
    {
        if let Some(hours) = cap.get(1).and_then(|h| h.as_str().parse::<i64>().ok()) {
            snooze_until = Some(Utc::now() + Duration::hours(hours));
        }
    }

    let priority = if lowercase.contains("p1") || lowercase.contains("critical") {
        TaskPriority::Critical
    } else if lowercase.contains("high") || lowercase.contains("urgent") {
        TaskPriority::High
    } else if lowercase.contains("low") {
        TaskPriority::Low
    } else {
        TaskPriority::Normal
    };

    if title.len() > 160 {
        title.truncate(160);
    }

    if let Some(index) = lowercase.find("notes:") {
        let notes = text[index + 6..].trim().to_string();
        title = text[..index].trim().to_string();
        return Ok(ParsedTask {
            title,
            notes: if notes.is_empty() { None } else { Some(notes) },
            due_at,
            priority,
            repeat_rule,
            snooze_until,
        });
    }

    Ok(ParsedTask {
        title,
        notes: None,
        due_at,
        priority,
        repeat_rule,
        snooze_until,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_natural_task;
    use aether_core::TaskPriority;

    #[test]
    fn parses_due_and_priority() {
        let parsed = parse_natural_task("Pay rent tomorrow high").expect("task parsed");
        assert!(parsed.due_at.is_some());
        assert_eq!(parsed.priority, TaskPriority::High);
    }

    #[test]
    fn parses_repeat_rule() {
        let parsed = parse_natural_task("Water plants every week").expect("task parsed");
        assert_eq!(parsed.repeat_rule.as_deref(), Some("FREQ=WEEKLY"));
    }
}
