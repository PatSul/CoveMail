use crate::CalendarError;
use cove_core::{Account, CalendarAlarm, CalendarEvent};
use async_trait::async_trait;
use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSettings {
    pub endpoint: String,
    pub access_token: Option<String>,
    pub calendar_id: String,
}

#[async_trait]
pub trait CalendarBackend: Send + Sync {
    async fn sync_range(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError>;

    async fn create_or_update_event(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        event: &CalendarEvent,
    ) -> Result<(), CalendarError>;
}

#[derive(Debug, Default)]
pub struct CalDavBackend {
    http: reqwest::Client,
}

impl CalDavBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl CalendarBackend for CalDavBackend {
    async fn sync_range(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag />
    <C:calendar-data />
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT">
        <C:time-range start="{}" end="{}" />
      </C:comp-filter>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#,
            from.format("%Y%m%dT%H%M%SZ"),
            to.format("%Y%m%dT%H%M%SZ")
        );

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
            return Err(CalendarError::Data(format!(
                "CalDAV sync failed with status {}",
                response.status()
            )));
        }

        let body = response.text().await?;
        Ok(parse_caldav_calendar_data(
            account.id,
            &settings.calendar_id,
            &body,
        ))
    }

    async fn create_or_update_event(
        &self,
        _account: &Account,
        settings: &CalendarSettings,
        event: &CalendarEvent,
    ) -> Result<(), CalendarError> {
        let mut endpoint = settings.endpoint.trim_end_matches('/').to_string();
        endpoint.push('/');
        endpoint.push_str(&format!("{}.ics", event.remote_id));

        let ics = render_single_event_ics(event);
        let mut request = self
            .http
            .put(endpoint)
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(ics);

        if let Some(token) = &settings.access_token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(CalendarError::Data(format!(
                "CalDAV event upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct GoogleCalendarBackend {
    http: reqwest::Client,
}

impl GoogleCalendarBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarEventsResponse {
    items: Option<Vec<GoogleCalendarEvent>>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarEvent {
    id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<GoogleCalendarDateTime>,
    end: Option<GoogleCalendarDateTime>,
    recurrence: Option<Vec<String>>,
    attendees: Option<Vec<GoogleCalendarAttendee>>,
    organizer: Option<GoogleCalendarOrganizer>,
    updated: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    date: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarAttendee {
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarOrganizer {
    email: Option<String>,
}

#[async_trait]
impl CalendarBackend for GoogleCalendarBackend {
    async fn sync_range(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| CalendarError::Data("missing Google access token".to_string()))?;

        let calendar_id = settings.calendar_id.replace('/', "%2F");
        let response = self
            .http
            .get(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{calendar_id}/events"
            ))
            .bearer_auth(token)
            .query(&[
                ("timeMin", from.to_rfc3339()),
                ("timeMax", to.to_rfc3339()),
                ("singleEvents", "true".to_string()),
                ("showDeleted", "false".to_string()),
                ("maxResults", "500".to_string()),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CalendarError::Data(format!(
                "Google Calendar sync failed with status {}",
                response.status()
            )));
        }

        let payload: GoogleCalendarEventsResponse = response.json().await?;
        let mut events = Vec::new();

        for raw in payload.items.unwrap_or_default() {
            if matches!(raw.status.as_deref(), Some("cancelled")) {
                continue;
            }

            let starts_at = raw
                .start
                .as_ref()
                .and_then(parse_google_datetime)
                .ok_or_else(|| {
                    CalendarError::Parse("Google event missing start time".to_string())
                })?;
            let ends_at = raw
                .end
                .as_ref()
                .and_then(parse_google_datetime)
                .ok_or_else(|| CalendarError::Parse("Google event missing end time".to_string()))?;
            let all_day = raw
                .start
                .as_ref()
                .and_then(|start| start.date.clone())
                .is_some();

            let attendees = raw
                .attendees
                .unwrap_or_default()
                .into_iter()
                .filter_map(|attendee| attendee.email)
                .collect::<Vec<_>>();

            let timezone = raw
                .start
                .as_ref()
                .and_then(|start| start.time_zone.clone())
                .or_else(|| raw.end.as_ref().and_then(|end| end.time_zone.clone()));

            events.push(CalendarEvent {
                id: uuid::Uuid::new_v4(),
                account_id: account.id,
                calendar_id: settings.calendar_id.clone(),
                remote_id: raw.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                title: raw.summary.unwrap_or_else(|| "Untitled event".to_string()),
                description: raw.description,
                location: raw.location,
                timezone,
                starts_at,
                ends_at,
                all_day,
                recurrence_rule: raw.recurrence.and_then(|rules| rules.into_iter().next()),
                attendees,
                organizer: raw.organizer.and_then(|org| org.email),
                alarms: vec![CalendarAlarm {
                    minutes_before: 10,
                    message: Some("Upcoming event".to_string()),
                }],
                rsvp_status: cove_core::RsvpStatus::NeedsAction,
                updated_at: raw
                    .updated
                    .as_deref()
                    .and_then(parse_rfc3339_to_utc)
                    .unwrap_or_else(Utc::now),
            });
        }

        Ok(events)
    }

    async fn create_or_update_event(
        &self,
        _account: &Account,
        settings: &CalendarSettings,
        event: &CalendarEvent,
    ) -> Result<(), CalendarError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| CalendarError::Data("missing Google access token".to_string()))?;

        let calendar_id = settings.calendar_id.replace('/', "%2F");
        let endpoint = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{calendar_id}/events/{}",
            event.remote_id
        );

        let body = serde_json::json!({
            "summary": event.title,
            "description": event.description,
            "location": event.location,
            "start": { "dateTime": event.starts_at.to_rfc3339() },
            "end": { "dateTime": event.ends_at.to_rfc3339() },
            "recurrence": event
                .recurrence_rule
                .as_ref()
                .map(|rule| vec![rule.clone()])
                .unwrap_or_default(),
            "attendees": event
                .attendees
                .iter()
                .map(|email| serde_json::json!({"email": email}))
                .collect::<Vec<_>>(),
        });

        let response = self
            .http
            .patch(endpoint)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CalendarError::Data(format!(
                "Google Calendar upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct MicrosoftGraphCalendarBackend {
    http: reqwest::Client,
}

impl MicrosoftGraphCalendarBackend {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GraphCalendarResponse {
    value: Option<Vec<GraphCalendarEvent>>,
}

#[derive(Debug, Deserialize)]
struct GraphCalendarEvent {
    id: Option<String>,
    subject: Option<String>,
    #[serde(rename = "bodyPreview")]
    body_preview: Option<String>,
    location: Option<GraphLocation>,
    start: Option<GraphDateTime>,
    end: Option<GraphDateTime>,
    attendees: Option<Vec<GraphAttendee>>,
    organizer: Option<GraphOrganizer>,
    #[serde(rename = "isAllDay")]
    is_all_day: Option<bool>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified: Option<String>,
    recurrence: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GraphLocation {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphAttendee {
    #[serde(rename = "emailAddress")]
    email_address: Option<GraphEmailAddress>,
}

#[derive(Debug, Deserialize)]
struct GraphOrganizer {
    #[serde(rename = "emailAddress")]
    email_address: Option<GraphEmailAddress>,
}

#[derive(Debug, Deserialize)]
struct GraphEmailAddress {
    address: Option<String>,
}

#[async_trait]
impl CalendarBackend for MicrosoftGraphCalendarBackend {
    async fn sync_range(
        &self,
        account: &Account,
        settings: &CalendarSettings,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| CalendarError::Data("missing Graph access token".to_string()))?;

        let response = self
            .http
            .get(format!(
                "https://graph.microsoft.com/v1.0/me/calendars/{}/calendarView",
                settings.calendar_id
            ))
            .bearer_auth(token)
            .query(&[
                ("startDateTime", from.to_rfc3339()),
                ("endDateTime", to.to_rfc3339()),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CalendarError::Data(format!(
                "Graph calendar sync failed with status {}",
                response.status()
            )));
        }

        let payload: GraphCalendarResponse = response.json().await?;
        let mut events = Vec::new();

        for raw in payload.value.unwrap_or_default() {
            let starts_at = raw
                .start
                .as_ref()
                .and_then(parse_graph_datetime)
                .ok_or_else(|| CalendarError::Parse("Graph event missing start".to_string()))?;
            let ends_at = raw
                .end
                .as_ref()
                .and_then(parse_graph_datetime)
                .ok_or_else(|| CalendarError::Parse("Graph event missing end".to_string()))?;

            let attendees = raw
                .attendees
                .unwrap_or_default()
                .into_iter()
                .filter_map(|attendee| attendee.email_address.and_then(|email| email.address))
                .collect::<Vec<_>>();

            events.push(CalendarEvent {
                id: Uuid::new_v4(),
                account_id: account.id,
                calendar_id: settings.calendar_id.clone(),
                remote_id: raw.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                title: raw.subject.unwrap_or_else(|| "Untitled event".to_string()),
                description: raw.body_preview,
                location: raw.location.and_then(|value| value.display_name),
                timezone: raw.start.and_then(|start| start.time_zone),
                starts_at,
                ends_at,
                all_day: raw.is_all_day.unwrap_or(false),
                recurrence_rule: raw.recurrence.map(|value| value.to_string()),
                attendees,
                organizer: raw
                    .organizer
                    .and_then(|org| org.email_address)
                    .and_then(|addr| addr.address),
                alarms: vec![CalendarAlarm {
                    minutes_before: 10,
                    message: Some("Upcoming event".to_string()),
                }],
                rsvp_status: cove_core::RsvpStatus::NeedsAction,
                updated_at: raw
                    .last_modified
                    .as_deref()
                    .and_then(parse_rfc3339_to_utc)
                    .unwrap_or_else(Utc::now),
            });
        }

        Ok(events)
    }

    async fn create_or_update_event(
        &self,
        _account: &Account,
        settings: &CalendarSettings,
        event: &CalendarEvent,
    ) -> Result<(), CalendarError> {
        let token = settings
            .access_token
            .as_ref()
            .ok_or_else(|| CalendarError::Data("missing Graph access token".to_string()))?;

        let endpoint = if event.remote_id.is_empty() {
            format!(
                "https://graph.microsoft.com/v1.0/me/calendars/{}/events",
                settings.calendar_id
            )
        } else {
            format!(
                "https://graph.microsoft.com/v1.0/me/events/{}",
                event.remote_id
            )
        };

        let payload = serde_json::json!({
            "subject": event.title,
            "body": {
                "contentType": "text",
                "content": event.description.clone().unwrap_or_default()
            },
            "start": {
                "dateTime": event.starts_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
                "timeZone": event.timezone.clone().unwrap_or_else(|| "UTC".to_string())
            },
            "end": {
                "dateTime": event.ends_at.format("%Y-%m-%dT%H:%M:%S").to_string(),
                "timeZone": event.timezone.clone().unwrap_or_else(|| "UTC".to_string())
            },
            "location": {
                "displayName": event.location.clone().unwrap_or_default()
            },
            "attendees": event
                .attendees
                .iter()
                .map(|email| serde_json::json!({
                    "emailAddress": { "address": email, "name": email },
                    "type": "required"
                }))
                .collect::<Vec<_>>(),
            "isAllDay": event.all_day
        });

        let response = if event.remote_id.is_empty() {
            self.http
                .post(endpoint)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await?
        } else {
            self.http
                .patch(endpoint)
                .bearer_auth(token)
                .json(&payload)
                .send()
                .await?
        };

        if !response.status().is_success() {
            return Err(CalendarError::Data(format!(
                "Graph event upsert failed with status {}",
                response.status()
            )));
        }

        Ok(())
    }
}

fn parse_caldav_calendar_data(
    account_id: Uuid,
    calendar_id: &str,
    payload: &str,
) -> Vec<CalendarEvent> {
    let data_re = Regex::new(
        r"(?is)<(?:[a-z0-9_]+:)?calendar-data[^>]*>(.*?)</(?:[a-z0-9_]+:)?calendar-data>",
    )
    .expect("valid CalDAV calendar-data regex");

    let mut events = Vec::new();
    for capture in data_re.captures_iter(payload) {
        let Some(raw_ics) = capture.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let calendar_data = unescape_xml_entities(raw_ics);
        events.extend(parse_ical_events(account_id, calendar_id, &calendar_data));
    }

    events
}

fn parse_ical_events(account_id: Uuid, calendar_id: &str, ics_payload: &str) -> Vec<CalendarEvent> {
    let mut events = Vec::new();
    let lines = unfold_ical_lines(ics_payload);

    let mut in_event = false;
    let mut uid: Option<String> = None;
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut location: Option<String> = None;
    let mut timezone: Option<String> = None;
    let mut starts_at: Option<DateTime<Utc>> = None;
    let mut ends_at: Option<DateTime<Utc>> = None;
    let mut all_day = false;
    let mut recurrence_rule: Option<String> = None;
    let mut attendees: Vec<String> = Vec::new();
    let mut organizer: Option<String> = None;
    let mut updated_at: Option<DateTime<Utc>> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("BEGIN:VEVENT") {
            in_event = true;
            uid = None;
            title = None;
            description = None;
            location = None;
            timezone = None;
            starts_at = None;
            ends_at = None;
            all_day = false;
            recurrence_rule = None;
            attendees.clear();
            organizer = None;
            updated_at = None;
            continue;
        }

        if trimmed.eq_ignore_ascii_case("END:VEVENT") {
            in_event = false;
            if let Some(starts) = starts_at {
                let ends = ends_at.unwrap_or_else(|| {
                    if all_day {
                        starts + Duration::days(1)
                    } else {
                        starts + Duration::hours(1)
                    }
                });

                events.push(CalendarEvent {
                    id: Uuid::new_v4(),
                    account_id,
                    calendar_id: calendar_id.to_string(),
                    remote_id: uid.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
                    title: title
                        .clone()
                        .unwrap_or_else(|| "Untitled event".to_string()),
                    description: description.clone(),
                    location: location.clone(),
                    timezone: timezone.clone(),
                    starts_at: starts,
                    ends_at: if ends <= starts {
                        starts + Duration::hours(1)
                    } else {
                        ends
                    },
                    all_day,
                    recurrence_rule: recurrence_rule.clone(),
                    attendees: attendees.clone(),
                    organizer: organizer.clone(),
                    alarms: vec![CalendarAlarm {
                        minutes_before: 10,
                        message: Some("Upcoming event".to_string()),
                    }],
                    rsvp_status: cove_core::RsvpStatus::NeedsAction,
                    updated_at: updated_at.unwrap_or_else(Utc::now),
                });
            }

            uid = None;
            title = None;
            description = None;
            location = None;
            timezone = None;
            starts_at = None;
            ends_at = None;
            all_day = false;
            recurrence_rule = None;
            attendees.clear();
            organizer = None;
            updated_at = None;
            continue;
        }

        if !in_event {
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
            description = Some(unescape_ical_text(value));
            continue;
        }

        if property_upper.starts_with("LOCATION") {
            location = Some(unescape_ical_text(value));
            continue;
        }

        if property_upper.starts_with("DTSTART") {
            starts_at = parse_ical_datetime_with_property(property, value);
            all_day = property_has_value_date(property)
                || (value.len() == 8 && value.chars().all(|ch| ch.is_ascii_digit()));
            if let Some(tzid) = property_tzid(property) {
                timezone = Some(tzid);
            }
            continue;
        }

        if property_upper.starts_with("DTEND") {
            ends_at = parse_ical_datetime_with_property(property, value);
            continue;
        }

        if property_upper.starts_with("RRULE") {
            recurrence_rule = Some(value.to_string());
            continue;
        }

        if property_upper.starts_with("ATTENDEE") {
            if let Some(email) = parse_ical_mail_address(value) {
                attendees.push(email);
            }
            continue;
        }

        if property_upper.starts_with("ORGANIZER") {
            organizer = parse_ical_mail_address(value).or_else(|| {
                if value.is_empty() {
                    None
                } else {
                    Some(unescape_ical_text(value))
                }
            });
            continue;
        }

        if property_upper.starts_with("LAST-MODIFIED") || property_upper.starts_with("DTSTAMP") {
            if let Some(parsed) = parse_ical_datetime_with_property(property, value) {
                updated_at = Some(parsed);
            }
        }
    }

    events
}

fn render_single_event_ics(event: &CalendarEvent) -> String {
    let uid = if event.remote_id.trim().is_empty() {
        Uuid::new_v4().to_string()
    } else {
        event.remote_id.clone()
    };

    let mut out = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Cove Mail//EN\r\n");
    out.push_str("BEGIN:VEVENT\r\n");
    out.push_str(&format!("UID:{}\r\n", escape_ical_text(&uid)));
    out.push_str(&format!(
        "DTSTAMP:{}\r\n",
        Utc::now().format("%Y%m%dT%H%M%SZ")
    ));

    if event.all_day {
        out.push_str(&format!(
            "DTSTART;VALUE=DATE:{}\r\n",
            event.starts_at.format("%Y%m%d")
        ));
        let all_day_end = if event.ends_at <= event.starts_at {
            event.starts_at + Duration::days(1)
        } else {
            event.ends_at
        };
        out.push_str(&format!(
            "DTEND;VALUE=DATE:{}\r\n",
            all_day_end.format("%Y%m%d")
        ));
    } else {
        out.push_str(&format!(
            "DTSTART:{}\r\n",
            event.starts_at.format("%Y%m%dT%H%M%SZ")
        ));
        let timed_end = if event.ends_at <= event.starts_at {
            event.starts_at + Duration::hours(1)
        } else {
            event.ends_at
        };
        out.push_str(&format!("DTEND:{}\r\n", timed_end.format("%Y%m%dT%H%M%SZ")));
    }

    out.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(&event.title)));
    if let Some(description) = event.description.as_deref() {
        out.push_str(&format!(
            "DESCRIPTION:{}\r\n",
            escape_ical_text(description)
        ));
    }
    if let Some(location) = event.location.as_deref() {
        out.push_str(&format!("LOCATION:{}\r\n", escape_ical_text(location)));
    }
    if let Some(rrule) = event.recurrence_rule.as_deref() {
        out.push_str(&format!("RRULE:{}\r\n", rrule));
    }
    if let Some(organizer) = event.organizer.as_deref() {
        out.push_str(&format!(
            "ORGANIZER:mailto:{}\r\n",
            escape_ical_text(organizer)
        ));
    }
    for attendee in &event.attendees {
        out.push_str(&format!(
            "ATTENDEE:mailto:{}\r\n",
            escape_ical_text(attendee)
        ));
    }

    out.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    out
}

fn parse_graph_datetime(value: &GraphDateTime) -> Option<DateTime<Utc>> {
    let raw = value.date_time.as_deref()?;
    if let Some(parsed) = parse_rfc3339_to_utc(raw) {
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

fn property_has_value_date(property: &str) -> bool {
    for part in property.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("VALUE") && value.eq_ignore_ascii_case("DATE") {
            return true;
        }
    }

    false
}

fn parse_ical_datetime_with_property(property: &str, value: &str) -> Option<DateTime<Utc>> {
    if let Some(parsed) = parse_ical_like(value) {
        return Some(parsed);
    }

    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y%m%d") {
        let midnight = date.and_hms_opt(0, 0, 0)?;
        return Some(Utc.from_utc_datetime(&midnight));
    }

    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M"))
        .ok()?;

    if let Some(zone_name) = property_tzid(property) {
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

fn parse_ical_mail_address(value: &str) -> Option<String> {
    let lowered = value.to_ascii_lowercase();
    if lowered.starts_with("mailto:") {
        return Some(unescape_ical_text(&value[7..]));
    }
    if value.contains('@') {
        return Some(unescape_ical_text(value));
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

fn parse_google_datetime(value: &GoogleCalendarDateTime) -> Option<DateTime<Utc>> {
    if let Some(date_time) = value.date_time.as_deref() {
        return parse_rfc3339_to_utc(date_time);
    }

    let date = value.date.as_deref()?;
    let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let midnight = parsed.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&midnight))
}

fn parse_rfc3339_to_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[allow(dead_code)]
fn parse_ical_like(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(value.with_timezone(&Utc));
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%SZ") {
        return Some(Utc.from_utc_datetime(&value));
    }
    None
}
