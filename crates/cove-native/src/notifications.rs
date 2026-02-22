//! Desktop notification support for new mail and calendar/task reminders.

use chrono::Utc;
use cove_config::NotificationConfig;
use cove_core::{CalendarEvent, ReminderTask};
use notify_rust::Notification;
use std::collections::HashSet;
use uuid::Uuid;

/// Tracks which notifications have already been shown to avoid duplicates.
pub struct NotificationState {
    /// Message IDs for which we've already sent a new-mail notification.
    notified_messages: HashSet<Uuid>,
    /// (event/task id, minutes_before) pairs we've already fired.
    notified_reminders: HashSet<(Uuid, i64)>,
}

impl NotificationState {
    pub fn new() -> Self {
        Self {
            notified_messages: HashSet::new(),
            notified_reminders: HashSet::new(),
        }
    }

    /// Check for new unseen messages and send desktop notifications.
    /// Returns the number of notifications sent.
    pub fn check_new_mail(
        &mut self,
        config: &NotificationConfig,
        messages: &[cove_core::MailMessage],
    ) -> usize {
        if !config.new_mail_enabled || is_quiet_hours(config) {
            return 0;
        }

        let mut count = 0;
        for msg in messages {
            if msg.flags.seen {
                continue;
            }
            if self.notified_messages.contains(&msg.id) {
                continue;
            }
            self.notified_messages.insert(msg.id);

            let sender = msg
                .from
                .first()
                .map(|a| {
                    a.name
                        .as_deref()
                        .unwrap_or(&a.address)
                        .to_string()
                })
                .unwrap_or_else(|| "Unknown sender".to_string());

            let _ = Notification::new()
                .summary(&format!("New mail from {sender}"))
                .body(&msg.subject)
                .appname("Cove Mail")
                .timeout(8000)
                .show();

            count += 1;
        }

        // Prune old entries to prevent unbounded growth.
        if self.notified_messages.len() > 5000 {
            self.notified_messages.clear();
        }

        count
    }

    /// Check calendar events for upcoming reminders and send notifications.
    pub fn check_calendar_reminders(
        &mut self,
        config: &NotificationConfig,
        events: &[CalendarEvent],
    ) -> usize {
        if !config.reminder_enabled || is_quiet_hours(config) {
            return 0;
        }

        let now = Utc::now();
        let mut count = 0;

        for event in events {
            let start = event.starts_at;
            if start < now {
                continue;
            }

            let minutes_until = (start - now).num_minutes();

            for &mins in &config.reminder_minutes_before {
                let key = (event.id, mins);
                if self.notified_reminders.contains(&key) {
                    continue;
                }
                if minutes_until <= mins && minutes_until >= 0 {
                    self.notified_reminders.insert(key);

                    let label = if mins == 0 {
                        "now".to_string()
                    } else {
                        format!("in {mins} min")
                    };

                    let _ = Notification::new()
                        .summary(&format!("{} - {label}", event.title))
                        .body(&event.location.clone().unwrap_or_default())
                        .appname("Cove Mail")
                        .timeout(10000)
                        .show();

                    count += 1;
                }
            }
        }

        // Prune old entries.
        if self.notified_reminders.len() > 2000 {
            self.notified_reminders.clear();
        }

        count
    }

    /// Check tasks with due dates for reminders.
    pub fn check_task_reminders(
        &mut self,
        config: &NotificationConfig,
        tasks: &[ReminderTask],
    ) -> usize {
        if !config.reminder_enabled || is_quiet_hours(config) {
            return 0;
        }

        let now = Utc::now();
        let mut count = 0;

        for task in tasks {
            // Skip completed/canceled tasks.
            if task.completed_at.is_some() {
                continue;
            }

            // Respect snooze.
            if let Some(snoozed) = task.snoozed_until {
                if now < snoozed {
                    continue;
                }
            }

            let due = match task.due_at {
                Some(dt) => dt,
                None => continue,
            };
            if due < now {
                continue;
            }

            let minutes_until = (due - now).num_minutes();

            for &mins in &config.reminder_minutes_before {
                let key = (task.id, mins);
                if self.notified_reminders.contains(&key) {
                    continue;
                }
                if minutes_until <= mins && minutes_until >= 0 {
                    self.notified_reminders.insert(key);

                    let label = if mins == 0 {
                        "now".to_string()
                    } else {
                        format!("in {mins} min")
                    };

                    let _ = Notification::new()
                        .summary(&format!("Task due {label}"))
                        .body(&task.title)
                        .appname("Cove Mail")
                        .timeout(10000)
                        .show();

                    count += 1;
                }
            }
        }

        count
    }
}

/// Check if current time falls within quiet hours.
fn is_quiet_hours(config: &NotificationConfig) -> bool {
    if !config.quiet_hours_enabled {
        return false;
    }

    let now = chrono::Local::now().time();

    let start = parse_hhmm(&config.quiet_hours_start);
    let end = parse_hhmm(&config.quiet_hours_end);

    let (Some(start), Some(end)) = (start, end) else {
        return false;
    };

    if start <= end {
        // Same-day range (e.g., 22:00 - 23:59 doesn't wrap).
        now >= start && now < end
    } else {
        // Overnight range (e.g., 22:00 - 08:00).
        now >= start || now < end
    }
}

fn parse_hhmm(s: &str) -> Option<chrono::NaiveTime> {
    chrono::NaiveTime::parse_from_str(s, "%H:%M").ok()
}
