use crate::{
    CalDavBackend, CalendarBackend, CalendarError, CalendarSettings, GoogleCalendarBackend,
    MicrosoftGraphCalendarBackend,
};
use cove_core::{Account, CalendarAlarm, CalendarEvent, Provider};
use cove_storage::Storage;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::io::Cursor;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct CalendarService {
    storage: Storage,
    caldav: Arc<CalDavBackend>,
    google: Arc<GoogleCalendarBackend>,
    graph: Arc<MicrosoftGraphCalendarBackend>,
}

impl CalendarService {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            caldav: Arc::new(CalDavBackend::new()),
            google: Arc::new(GoogleCalendarBackend::new()),
            graph: Arc::new(MicrosoftGraphCalendarBackend::new()),
        }
    }

    pub async fn sync_range(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let backend = self.backend_for(account);
        let events = backend.sync_range(account, settings, from, to).await?;
        for event in &events {
            self.storage.upsert_calendar_event(event).await?;
        }
        Ok(events)
    }

    pub async fn import_ics(
        &self,
        account_id: Uuid,
        calendar_id: &str,
        ics_payload: &str,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let reader = Cursor::new(ics_payload.as_bytes());
        let parser = ical::IcalParser::new(reader);

        let mut imported = Vec::new();
        for calendar in parser {
            let calendar = calendar.map_err(|err| CalendarError::Parse(err.to_string()))?;
            for event in calendar.events {
                let uid = property_value(&event.properties, "UID")
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                let title = property_value(&event.properties, "SUMMARY")
                    .unwrap_or_else(|| "Untitled Event".to_string());
                let description = property_value(&event.properties, "DESCRIPTION");
                let location = property_value(&event.properties, "LOCATION");
                let recurrence_rule = property_value(&event.properties, "RRULE");

                let starts_at_raw = property_value(&event.properties, "DTSTART")
                    .ok_or_else(|| CalendarError::Data("VEVENT missing DTSTART".to_string()))?;
                let ends_at_raw = property_value(&event.properties, "DTEND")
                    .ok_or_else(|| CalendarError::Data("VEVENT missing DTEND".to_string()))?;

                let starts_at = parse_ical_datetime(&starts_at_raw)?;
                let ends_at = parse_ical_datetime(&ends_at_raw)?;

                let imported_event = CalendarEvent {
                    id: Uuid::new_v4(),
                    account_id,
                    calendar_id: calendar_id.to_string(),
                    remote_id: uid,
                    title,
                    description,
                    location,
                    timezone: None,
                    starts_at,
                    ends_at,
                    all_day: is_all_day(&starts_at_raw),
                    recurrence_rule,
                    attendees: vec![],
                    organizer: property_value(&event.properties, "ORGANIZER"),
                    alarms: vec![CalendarAlarm {
                        minutes_before: 10,
                        message: Some("Upcoming event".to_string()),
                    }],
                    rsvp_status: cove_core::RsvpStatus::NeedsAction,
                    updated_at: Utc::now(),
                };

                self.storage.upsert_calendar_event(&imported_event).await?;
                imported.push(imported_event);
            }
        }

        Ok(imported)
    }

    pub fn export_ics(&self, events: &[CalendarEvent]) -> String {
        let mut output =
            String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Cove Mail//EN\r\n");

        for event in events {
            output.push_str("BEGIN:VEVENT\r\n");
            output.push_str(&format!("UID:{}\r\n", event.remote_id));
            output.push_str(&format!(
                "DTSTAMP:{}\r\n",
                Utc::now().format("%Y%m%dT%H%M%SZ")
            ));
            output.push_str(&format!(
                "DTSTART:{}\r\n",
                event.starts_at.format("%Y%m%dT%H%M%SZ")
            ));
            output.push_str(&format!(
                "DTEND:{}\r\n",
                event.ends_at.format("%Y%m%dT%H%M%SZ")
            ));
            output.push_str(&format!("SUMMARY:{}\r\n", escape_ical(&event.title)));
            if let Some(desc) = &event.description {
                output.push_str(&format!("DESCRIPTION:{}\r\n", escape_ical(desc)));
            }
            if let Some(location) = &event.location {
                output.push_str(&format!("LOCATION:{}\r\n", escape_ical(location)));
            }
            if let Some(rrule) = &event.recurrence_rule {
                output.push_str(&format!("RRULE:{}\r\n", rrule));
            }
            output.push_str("END:VEVENT\r\n");
        }

        output.push_str("END:VCALENDAR\r\n");
        output
    }

    pub async fn detect_conflicts(
        &self,
        account_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(CalendarEvent, CalendarEvent)>, CalendarError> {
        let mut events = self
            .storage
            .list_calendar_events(account_id, from, to)
            .await?;
        events.sort_by_key(|event| event.starts_at);

        let mut conflicts = Vec::new();
        for i in 0..events.len() {
            for j in (i + 1)..events.len() {
                if events[j].starts_at >= events[i].ends_at {
                    break;
                }
                conflicts.push((events[i].clone(), events[j].clone()));
            }
        }

        Ok(conflicts)
    }

    fn backend_for(&self, account: &Account) -> Arc<dyn CalendarBackend> {
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

fn property_value(properties: &[ical::property::Property], key: &str) -> Option<String> {
    properties
        .iter()
        .find(|property| property.name.eq_ignore_ascii_case(key))
        .and_then(|property| property.value.clone())
}

fn parse_ical_datetime(raw: &str) -> Result<DateTime<Utc>, CalendarError> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Ok(value.with_timezone(&Utc));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%SZ") {
        return Ok(Utc.from_utc_datetime(&value));
    }
    if let Ok(value) = NaiveDate::parse_from_str(raw, "%Y%m%d") {
        let datetime = value
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| CalendarError::Parse(format!("invalid all-day date: {raw}")))?;
        return Ok(Utc.from_utc_datetime(&datetime));
    }

    Err(CalendarError::Parse(format!(
        "unsupported datetime format in ICS: {raw}"
    )))
}

fn is_all_day(raw: &str) -> bool {
    raw.len() == 8 && raw.chars().all(|char| char.is_ascii_digit())
}

fn escape_ical(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}
