mod export;
mod html_render;
mod notifications;

use cove_ai::{AiRuntimeConfig, AiService, CloudProviderRuntime, LocalRuntime};
use cove_calendar::{CalendarService, CalendarSettings};
use cove_config::{AppConfig, ConfigManager};
use cove_core::{
    Account, AccountProtocol, AiMode, CloudAiProvider, ContactSummary, MailAddress, MailFolder,
    MailMessage, MailThreadSummary, Provider,
};
use cove_email::{EmailService, OutgoingAttachment, OutgoingMail, ProtocolSettings};
use cove_security::{OAuthWorkflow, SecretKey, SecretStore};
use cove_storage::Storage;
use cove_tasks::{TaskService, TaskSettings};
use anyhow::Context;
use base64::Engine;
use chrono::{Datelike, Duration, Timelike, Utc};
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use uuid::Uuid;

fn main() -> anyhow::Result<()> {
    let mut options = eframe::NativeOptions::default();
    options.viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true);
    eframe::run_native(
        "Cove Mail Native",
        options,
        Box::new(|_cc| {
            apply_midnight_theme(&_cc.egui_ctx);
            Ok(Box::new(
                NativeApp::initialize().expect("native init"),
            ))
        }),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn apply_midnight_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    // Increase typography sizing for a more bold, gorgeous look
    for font_id in style.text_styles.values_mut() {
        font_id.size *= 1.0;
    }

    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(14);

    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = egui::Color32::from_rgb(0x17, 0x2b, 0x46);
    visuals.panel_fill = egui::Color32::from_rgb(0x0a, 0x12, 0x20);

    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x1b, 0x35, 0x53);
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2f, 0x4a, 0x68));
    visuals.widgets.noninteractive.fg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0xe4, 0xef, 0xff));

    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x17, 0x2b, 0x46);
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2f, 0x4a, 0x68));
    visuals.widgets.inactive.fg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0xac, 0xc3, 0xdf));

    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x1d, 0x3a, 0x5a);
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x37, 0xbf, 0xae));
    visuals.widgets.hovered.fg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0xe4, 0xef, 0xff));

    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x2b, 0xa5, 0x95);
    visuals.widgets.active.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x37, 0xbf, 0xae));
    visuals.widgets.active.fg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0xee, 0xff, 0xf9));

    visuals.selection.bg_fill = egui::Color32::from_rgb(0x37, 0xbf, 0xae);
    visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0xe4, 0xef, 0xff));

    visuals.window_corner_radius = egui::CornerRadius::same(12);

    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 16],
        blur: 16,
        spread: 0,
        color: egui::Color32::from_black_alpha(180),
    };

    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 8,
        spread: 0,
        color: egui::Color32::from_black_alpha(120),
    };

    style.visuals = visuals;
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    SetupWizard,
    Inbox,
    Chat,
    Calendar,
    Tasks,
    Ai,
    Security,
    Settings,
    Contacts,
    Rules,
    Analytics,
    Integrations,
    Notes,
}

struct OAuthDraft {
    provider: Provider,
    email: String,
    display_name: String,
    client_id: String,
    redirect_url: String,
    code: String,
    csrf_state: String,
    expected_state: String,
    auth_url: String,
    pkce_verifier: String,
    started: bool,
    sync_limit: cove_core::OfflineSyncLimit,
}

struct GenericSetupDraft {
    email: String,
    password: String,
    display_name: String,
    imap_server: String,
    imap_port: u16,
    smtp_server: String,
    smtp_port: u16,
}

impl Default for GenericSetupDraft {
    fn default() -> Self {
        Self {
            email: String::new(),
            password: String::new(),
            display_name: String::new(),
            imap_server: String::new(),
            imap_port: 993,
            smtp_server: String::new(),
            smtp_port: 465,
        }
    }
}

struct NativeApp {
    runtime: tokio::runtime::Runtime,
    config: AppConfig,
    config_manager: ConfigManager,
    storage: Storage,
    secrets: SecretStore,
    email: EmailService,
    calendar: CalendarService,
    tasks: TaskService,
    ai: AiService,
    accounts: Vec<Account>,
    selected_account: Option<Uuid>,
    view: View,
    mail_query: String,
    folders: Vec<MailFolder>,
    selected_folder: String,
    threads: Vec<MailThreadSummary>,
    selected_thread: Option<String>,
    thread_messages: Vec<MailMessage>,
    selected_message: Option<Uuid>,
    compose_to: String,
    compose_subject: String,
    compose_body: String,
    attachment_path: String,
    attachment_paths: Vec<String>,
    ai_subject: String,
    ai_body: String,
    ai_output: String,
    openai_key: String,
    anthropic_key: String,
    gemini_key: String,
    mistral_key: String,
    groq_key: String,
    grok_key: String,
    openrouter_key: String,
    status: String,
    oauth: OAuthDraft,
    generic_setup: GenericSetupDraft,
    
    // Chat View State
    chat_contacts: Vec<ContactSummary>,
    selected_chat_contact: Option<String>,
    chat_messages: Vec<MailMessage>,
    chat_compose_body: String,

    // Magic Compose State
    magic_compose_prompt: String,
    magic_compose_format: String,
    magic_compose_tone: String,
    magic_compose_length: String,
    show_magic_compose: bool,
    show_compose_window: bool,

    // AI Provider Configuration
    ai_mode: AiMode,
    ai_cloud_provider: Option<CloudAiProvider>,

    export_password: String,
    import_password: String,

    // Attachment handling
    pending_attachment_save: Option<(Uuid, String)>,
    pending_attachment_open: Option<(Uuid, String)>,

    // Notifications
    notification_state: notifications::NotificationState,
    last_notification_check: std::time::Instant,

    // Unified inbox
    unified_inbox: bool,

    // Command palette
    show_command_palette: bool,
    command_query: String,

    // Snooze dialog
    pending_snooze: Option<Uuid>,

    // Undo send
    undo_send_message: Option<(Account, ProtocolSettings, OutgoingMail, std::time::Instant)>,

    // Contact autocomplete suggestions
    contact_suggestions: Vec<cove_core::Contact>,
}
impl NativeApp {
    fn initialize() -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build tokio runtime")?;

        let config_manager = ConfigManager::new().context("initialize config manager")?;
        let config = config_manager.load().context("load app config")?;

        let secrets =
            SecretStore::new_with_legacy("io.covemail.desktop", "io.aether.desktop");
        let db_key = secrets
            .get(&SecretKey {
                namespace: "database".to_string(),
                id: "sqlcipher_key".to_string(),
            })
            .context("load sqlcipher key")?;

        let sqlcipher = if config.database.sqlcipher_enabled {
            let key = db_key.context("sqlcipher enabled but key missing")?;
            if key.len() < 16 {
                anyhow::bail!("sqlcipher key must be at least 16 chars");
            }
            Some(key)
        } else {
            None
        };

        let db_path = config_manager.data_dir().join(&config.database.file_name);
        let search_path = config_manager.cache_dir().join("mail-index");
        let storage = runtime
            .block_on(Storage::connect(
                &db_path,
                &search_path,
                sqlcipher.as_deref(),
            ))
            .context("connect storage")?;

        let email = EmailService::new(storage.clone());
        let calendar = CalendarService::new(storage.clone());
        let tasks = TaskService::new(storage.clone());
        let ai = AiService::new(ai_runtime_from_config(&config), secrets.clone());

        let accounts = runtime
            .block_on(storage.list_accounts())
            .context("load accounts")?;
        let selected_account = accounts.first().map(|account| account.id);

        let initial_view = if accounts.is_empty() {
            View::SetupWizard
        } else {
            View::Inbox
        };

        Ok(Self {
            runtime,
            config,
            config_manager,
            storage,
            secrets: secrets.clone(),
            email,
            calendar,
            tasks,
            ai,
            accounts,
            selected_account,
            view: initial_view,
            mail_query: String::new(),
            folders: Vec::new(),
            selected_folder: "INBOX".to_string(),
            threads: Vec::new(),
            selected_thread: None,
            thread_messages: Vec::new(),
            selected_message: None,
            compose_to: String::new(),
            compose_subject: String::new(),
            compose_body: String::new(),
            attachment_path: String::new(),
            attachment_paths: Vec::new(),
            ai_subject: String::new(),
            ai_body: String::new(),
            ai_output: String::new(),
            openai_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "openai".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            anthropic_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "anthropic".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            gemini_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "gemini".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            mistral_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "mistral".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            groq_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "groq".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            grok_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "grok".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            openrouter_key: secrets
                .get(&SecretKey {
                    namespace: "ai_api_key".to_string(),
                    id: "openrouter".to_string(),
                })
                .unwrap_or_default()
                .unwrap_or_default(),
            status: "Ready".to_string(),
            oauth: OAuthDraft {
                provider: Provider::Gmail,
                email: String::new(),
                display_name: String::new(),
                client_id: String::new(),
                redirect_url: "http://127.0.0.1:8765/oauth/callback".to_string(),
                code: String::new(),
                csrf_state: String::new(),
                expected_state: String::new(),
                auth_url: String::new(),
                pkce_verifier: String::new(),
                started: false,
                sync_limit: cove_core::OfflineSyncLimit::Days(30),
            },
            generic_setup: GenericSetupDraft::default(),
            chat_contacts: Vec::new(),
            selected_chat_contact: None,
            chat_messages: Vec::new(),
            chat_compose_body: String::new(),
            magic_compose_prompt: String::new(),
            magic_compose_format: "Email".to_string(),
            magic_compose_tone: "Friendly".to_string(),
            magic_compose_length: "Short".to_string(),
            show_magic_compose: false,
            show_compose_window: false,
            ai_mode: AiMode::Local,
            ai_cloud_provider: Some(CloudAiProvider::OpenAi),
            export_password: String::new(),
            import_password: String::new(),
            pending_attachment_save: None,
            pending_attachment_open: None,
            notification_state: notifications::NotificationState::new(),
            last_notification_check: std::time::Instant::now(),
            unified_inbox: false,
            show_command_palette: false,
            command_query: String::new(),
            pending_snooze: None,
            undo_send_message: None,
            contact_suggestions: Vec::new(),
        })
    }

    fn account(&self) -> Option<&Account> {
        let id = self.selected_account?;
        self.accounts.iter().find(|account| account.id == id)
    }

    fn reload_accounts(&mut self) {
        match self.runtime.block_on(self.storage.list_accounts()) {
            Ok(accounts) => {
                let previous = self.selected_account;
                self.accounts = accounts;
                self.selected_account = previous
                    .filter(|id| self.accounts.iter().any(|account| account.id == *id))
                    .or_else(|| self.accounts.first().map(|account| account.id));
            }
            Err(err) => self.status = format!("load accounts failed: {err}"),
        }
    }

    fn load_folders(&mut self, refresh_remote: bool) {
        let Some(account) = self.account().cloned() else {
            self.folders.clear();
            return;
        };

        let mut folders = match self.runtime.block_on(self.storage.list_mail_folders(account.id)) {
            Ok(folders) => folders,
            Err(err) => {
                self.status = format!("folder load failed: {err}");
                return;
            }
        };

        if refresh_remote {
            let mut settings = match self.load_email_settings(account.id) {
                Ok(settings) => settings,
                Err(err) => {
                    self.status = err;
                    return;
                }
            };
            hydrate_email_secrets(account.id, &self.secrets, &mut settings);

            match self.runtime.block_on(self.email.sync_folders(&account, &settings)) {
                Ok(remote) => merge_folder_lists(&mut folders, remote),
                Err(err) => self.status = format!("folder refresh failed: {err}"),
            }
        }

        self.folders = folders;
        if !self.folders.iter().any(|folder| folder.path == self.selected_folder) {
            if let Some(inbox) = self
                .folders
                .iter()
                .find(|folder| folder.path.eq_ignore_ascii_case("INBOX"))
                .map(|folder| folder.path.clone())
            {
                self.selected_folder = inbox;
            } else if let Some(first) = self.folders.first() {
                self.selected_folder = first.path.clone();
            }
        }
    }

    fn load_threads(&mut self) {
        if self.unified_inbox {
            match self.runtime.block_on(self.email.list_unified_threads(200, 0)) {
                Ok(threads) => {
                    self.threads = threads;
                    self.selected_thread = self.threads.first().map(|t| t.thread_id.clone());
                    self.status = format!("Unified inbox: {} threads", self.threads.len());
                }
                Err(err) => self.status = format!("unified inbox failed: {err}"),
            }
            self.load_thread_messages();
            return;
        }

        let Some(account) = self.account().cloned() else {
            self.threads.clear();
            return;
        };

        match self
            .runtime
            .block_on(
                self.email
                    .list_threads(account.id, Some(&self.selected_folder), 200, 0),
            )
        {
            Ok(threads) => {
                self.threads = threads;
                self.selected_thread = self
                    .threads
                    .first()
                    .map(|thread| thread.thread_id.clone());
                self.status = format!("Loaded {} threads", self.threads.len());
            }
            Err(err) => self.status = format!("thread load failed: {err}"),
        }
    }

    fn load_thread_messages(&mut self) {
        let Some(thread_id) = self.selected_thread.clone() else {
            return;
        };

        let result = if self.unified_inbox {
            self.runtime.block_on(self.storage.list_unified_thread_messages(&thread_id))
        } else {
            let Some(account_id) = self.selected_account else {
                return;
            };
            self.runtime.block_on(self.storage.list_thread_messages(account_id, &thread_id))
        };

        match result {
            Ok(messages) => {
                self.selected_message = messages.last().map(|message| message.id);
                self.thread_messages = messages;
            }
            Err(err) => self.status = format!("message load failed: {err}"),
        }
    }

    fn run_sync_now(&mut self) {
        let Some(account) = self.account().cloned() else {
            self.status = "No account selected".to_string();
            return;
        };

        let mut email_settings = match self.load_email_settings(account.id) {
            Ok(settings) => settings,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        let mut calendar_settings = match self.load_calendar_settings(account.id) {
            Ok(settings) => settings,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        let mut task_settings = match self.load_task_settings(account.id) {
            Ok(settings) => settings,
            Err(err) => {
                self.status = err;
                return;
            }
        };

        hydrate_email_secrets(account.id, &self.secrets, &mut email_settings);
        hydrate_calendar_secrets(account.id, &self.secrets, &mut calendar_settings);
        hydrate_task_secrets(account.id, &self.secrets, &mut task_settings);

        let email_count = self.runtime.block_on(self.email.sync_recent_mail(
            &account,
            &email_settings,
            "INBOX",
            100,
        ));
        let calendar_count = self.runtime.block_on(self.calendar.sync_range(
            &account,
            &calendar_settings,
            Utc::now() - Duration::days(30),
            Utc::now() + Duration::days(365),
        ));
        let task_count = self
            .runtime
            .block_on(self.tasks.sync_tasks(&account, &task_settings));

        match (email_count, calendar_count, task_count) {
            (Ok(mail), Ok(calendar), Ok(tasks)) => {
                self.status = format!(
                    "Sync complete: {mail} emails, {} calendar events, {} tasks",
                    calendar.len(),
                    tasks.len()
                );
                self.load_folders(false);
                self.load_threads();
            }
            (mail, calendar, tasks) => {
                let err_msg = format!(
                    "Sync failed: email={:?}, calendar={:?}, tasks={:?}",
                    mail.err(),
                    calendar.err(),
                    tasks.err()
                );
                self.status = err_msg.clone();
                self.notification_state.notify_sync_error(&self.config.notifications, "A sync operation failed. Please check your credentials or network.");
            }
        }
    }

    fn load_chat_contacts(&mut self) {
        let Some(account) = self.account().cloned() else {
            self.chat_contacts.clear();
            return;
        };

        match self
            .runtime
            .block_on(
                self.email
                    .list_conversations_by_contact(&account, Some(&self.selected_folder), 200, 0),
            )
        {
            Ok(contacts) => {
                self.chat_contacts = contacts;
                if self.selected_chat_contact.is_none() {
                    self.selected_chat_contact = self
                        .chat_contacts
                        .first()
                        .map(|contact| contact.email_address.clone());
                }
            }
            Err(err) => self.status = format!("contact load failed: {err}"),
        }
    }

    fn load_chat_messages(&mut self) {
        let Some(account_id) = self.selected_account else {
            return;
        };
        let Some(contact_email) = self.selected_chat_contact.clone() else {
            return;
        };

        match self
            .runtime
            .block_on(self.storage.list_mail_messages(account_id, Some(&self.selected_folder), 1000, 0))
        {
            Ok(result) => {
                // Filter messages to only those involving this contact
                let messages: Vec<_> = result.items.into_iter().filter(|msg| {
                    let from_contact = msg.from.iter().any(|addr| addr.address.eq_ignore_ascii_case(&contact_email));
                    let to_contact = msg.to.iter().any(|addr| addr.address.eq_ignore_ascii_case(&contact_email));
                    from_contact || to_contact
                }).collect();

                let mut sorted_messages = messages;
                sorted_messages.sort_by_key(|msg| msg.received_at);
                self.chat_messages = sorted_messages;
            }
            Err(err) => self.status = format!("message load failed: {err}"),
        }
    }

    fn send_chat_reply(&mut self) {
        let Some(account) = self.account().cloned() else {
            self.status = "No account selected".to_string();
            return;
        };
        let Some(contact_email) = self.selected_chat_contact.clone() else {
            return;
        };

        if self.chat_compose_body.trim().is_empty() {
            return;
        }

        let mut settings = match self.load_email_settings(account.id) {
            Ok(settings) => settings,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        hydrate_email_secrets(account.id, &self.secrets, &mut settings);

        let subject = self.chat_messages.last().map(|m| {
            if m.subject.to_lowercase().starts_with("re:") {
                m.subject.clone()
            } else {
                format!("Re: {}", m.subject)
            }
        }).unwrap_or_else(|| "Chat Reply".to_string());

        let outgoing = OutgoingMail {
            from: MailAddress {
                name: Some(account.display_name.clone()),
                address: account.email_address.clone(),
            },
            to: vec![MailAddress {
                name: None,
                address: contact_email,
            }],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject,
            body_text: self.chat_compose_body.clone(),
            body_html: None,
            attachments: Vec::new(),
        };

        match self
            .runtime
            .block_on(self.email.send(&account, &settings, &outgoing))
        {
            Ok(()) => {
                self.status = "Chat reply sent".to_string();
                self.chat_compose_body.clear();
            }
            Err(err) => self.status = format!("chat reply failed: {err}"),
        }
    }

    fn search_mail(&mut self) {
        let query = self.mail_query.trim().to_string();
        // Check for search operators.
        let has_operators = query.contains(':')
            && ["from:", "to:", "subject:", "has:", "is:", "before:", "after:", "label:"]
                .iter()
                .any(|op| query.to_lowercase().contains(op));

        if has_operators {
            self.search_with_operators(&query);
        } else {
            match self.runtime.block_on(self.storage.search_mail(&query, 100)) {
                Ok(result) => {
                    self.selected_thread = None;
                    self.selected_message = result.items.last().map(|message| message.id);
                    self.thread_messages = result.items;
                    self.status = format!("Search returned {} message(s)", self.thread_messages.len());
                }
                Err(err) => self.status = format!("search failed: {err}"),
            }
        }
    }

    fn search_with_operators(&mut self, query: &str) {
        let Some(account_id) = self.selected_account else {
            self.status = "No account selected for operator search".to_string();
            return;
        };

        // Load a wide set of messages and filter client-side.
        let all_messages = match self.runtime.block_on(
            self.storage.list_mail_messages(account_id, None, 2000, 0),
        ) {
            Ok(result) => result.items,
            Err(err) => {
                self.status = format!("search failed: {err}");
                return;
            }
        };

        let mut results = all_messages;

        // Parse operator tokens.
        for token in query.split_whitespace() {
            let lower = token.to_lowercase();
            if let Some(val) = lower.strip_prefix("from:") {
                results.retain(|m| {
                    m.from.iter().any(|a| {
                        a.address.to_lowercase().contains(val)
                            || a.name.as_deref().unwrap_or("").to_lowercase().contains(val)
                    })
                });
            } else if let Some(val) = lower.strip_prefix("to:") {
                results.retain(|m| m.to.iter().any(|a| a.address.to_lowercase().contains(val)));
            } else if let Some(val) = lower.strip_prefix("subject:") {
                results.retain(|m| m.subject.to_lowercase().contains(val));
            } else if lower == "has:attachment" {
                results.retain(|m| !m.attachments.is_empty());
            } else if lower == "is:unread" {
                results.retain(|m| !m.flags.seen);
            } else if lower == "is:pinned" {
                results.retain(|m| m.pinned);
            } else if let Some(val) = lower.strip_prefix("label:") {
                results.retain(|m| m.labels.iter().any(|l| l.to_lowercase().contains(val)));
            } else if let Some(val) = lower.strip_prefix("before:") {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d") {
                    let dt = date.and_hms_opt(23, 59, 59)
                        .and_then(|ndt| chrono::TimeZone::from_utc_datetime(&Utc, &ndt).into());
                    if let Some(cutoff) = dt {
                        results.retain(|m| m.received_at <= cutoff);
                    }
                }
            } else if let Some(val) = lower.strip_prefix("after:") {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(val, "%Y-%m-%d") {
                    let dt = date.and_hms_opt(0, 0, 0)
                        .and_then(|ndt| chrono::TimeZone::from_utc_datetime(&Utc, &ndt).into());
                    if let Some(cutoff) = dt {
                        results.retain(|m| m.received_at >= cutoff);
                    }
                }
            }
            // Plain text tokens: match against subject/preview.
            else {
                let val = token.to_lowercase();
                results.retain(|m| {
                    m.subject.to_lowercase().contains(&val)
                        || m.preview.to_lowercase().contains(&val)
                });
            }
        }

        self.selected_thread = None;
        self.selected_message = results.last().map(|m| m.id);
        self.thread_messages = results;
        self.status = format!("Operator search: {} result(s)", self.thread_messages.len());
    }

    fn send_compose(&mut self) {
        let Some(account) = self.account().cloned() else {
            self.status = "No account selected".to_string();
            return;
        };

        let mut settings = match self.load_email_settings(account.id) {
            Ok(settings) => settings,
            Err(err) => {
                self.status = err;
                return;
            }
        };
        hydrate_email_secrets(account.id, &self.secrets, &mut settings);

        let mut attachments = Vec::new();
        for path in &self.attachment_paths {
            let file_path = Path::new(path);
            let Ok(bytes) = std::fs::read(file_path) else {
                continue;
            };
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let file_name = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("attachment.bin")
                .to_string();
            attachments.push(OutgoingAttachment {
                file_name,
                mime_type: "application/octet-stream".to_string(),
                content_base64: encoded,
                inline: false,
            });
        }

        let to = self
            .compose_to
            .split(',')
            .map(str::trim)
            .filter(|email| !email.is_empty())
            .map(|email| MailAddress {
                name: None,
                address: email.to_string(),
            })
            .collect::<Vec<_>>();

        if to.is_empty() {
            self.status = "Compose requires at least one recipient".to_string();
            return;
        }

        let outgoing = OutgoingMail {
            from: MailAddress {
                name: Some(account.display_name.clone()),
                address: account.email_address.clone(),
            },
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: self.compose_subject.clone(),
            body_text: self.compose_body.clone(),
            body_html: None,
            attachments,
        };

        self.undo_send_message = Some((
            account.clone(),
            settings.clone(),
            outgoing,
            std::time::Instant::now()
        ));
        self.status = "Draft ready to send (Undo available for 5s)".to_string();
        self.compose_subject.clear();
        self.compose_body.clear();
        self.attachment_paths.clear();
    }

    fn process_scheduled_messages(&mut self) {
        let Ok(due_messages) = self.runtime.block_on(self.storage.due_scheduled_messages()) else { return; };
        
        for msg in due_messages {
            let mut settings = match self.load_email_settings(msg.account_id) {
                Ok(settings) => settings,
                Err(_) => continue,
            };
            hydrate_email_secrets(msg.account_id, &self.secrets, &mut settings);
            
            let Some(account) = self.accounts.iter().find(|a| a.id == msg.account_id).cloned() else { continue; };
            
            let mut attachments = Vec::new(); // Simplifying here, we aren't pulling DB attachment blobs for scheduled yet
            
            // To be accurate, we need to extract raw recipients
            let to = msg.to.clone();
            
            let outgoing = OutgoingMail {
                from: msg.from.first().cloned().unwrap_or(MailAddress { name: None, address: account.email_address.clone() }),
                to,
                cc: msg.cc.clone(),
                bcc: msg.bcc.clone(),
                reply_to: msg.reply_to.clone(),
                subject: msg.subject.clone(),
                body_text: msg.body_text.clone().unwrap_or_default(),
                body_html: msg.body_html.clone(),
                attachments,
            };
            
            if self.runtime.block_on(self.email.send(&account, &settings, &outgoing)).is_ok() {
                // Remove scheduled flag
                let _ = self.runtime.block_on(self.storage.schedule_send(msg.id, None));
                self.status = "Scheduled message sent".to_string();
            }
        }
    }

    fn summarize_ai(&mut self) {
        let response = self.runtime.block_on(self.ai.summarize_email(
            &self.ai_subject,
            &self.ai_body,
            self.ai_mode.clone(),
            self.ai_cloud_provider.clone(),
        ));

        match response {
            Ok((result, provenance)) => {
                self.ai_output = result.output;
                self.status = format!("AI {} via {}", provenance.feature, provenance.destination);
            }
            Err(err) => self.status = format!("AI failed: {err}"),
        }
    }

    fn generate_magic_message(&mut self) {
        if self.magic_compose_prompt.trim().is_empty() {
            self.status = "Please enter a prompt for magic compose".to_string();
            return;
        }

        self.status = "Generating message...".to_string();
        
        let response = self.runtime.block_on(self.ai.generate_message(
            &self.magic_compose_prompt,
            &self.magic_compose_format,
            &self.magic_compose_tone,
            &self.magic_compose_length,
            self.ai_mode.clone(),
            self.ai_cloud_provider.clone(),
        ));

        match response {
            Ok((result, provenance)) => {
                if !self.compose_body.is_empty() && !self.compose_body.ends_with("\n\n") {
                    self.compose_body.push_str("\n\n");
                }
                self.compose_body.push_str(&result.output);
                self.status = format!("AI Message generated via {}", provenance.destination);
                self.show_magic_compose = false; // Hide panel on success
                self.magic_compose_prompt.clear();
            }
            Err(err) => self.status = format!("AI generation failed: {err}"),
        }
    }

    fn begin_oauth(&mut self) {
        match oauth_profile_for_provider(
            self.oauth.provider.clone(),
            self.oauth.client_id.trim(),
            self.oauth.redirect_url.trim(),
        ) {
            Ok(profile) => {
                let workflow = match OAuthWorkflow::new(profile) {
                    Ok(w) => w,
                    Err(err) => {
                        self.status = format!("OAuth validation failed: {err}");
                        return;
                    }
                };
                match workflow.begin_pkce_session() {
                    Ok(session) => {
                        self.oauth.auth_url = session.authorization_url;
                        self.oauth.expected_state = session.csrf_state;
                        self.oauth.csrf_state.clear();
                        self.oauth.pkce_verifier = session.pkce_verifier;
                        self.oauth.started = true;
                        self.status =
                            "OAuth session started. Open URL and paste code/state".to_string();
                    }
                    Err(err) => self.status = format!("OAuth start failed: {err}"),
                }
            }
            Err(err) => self.status = err,
        }
    }

    fn complete_oauth(&mut self) {
        let Some(profile) = oauth_profile_for_provider(
            self.oauth.provider.clone(),
            self.oauth.client_id.trim(),
            self.oauth.redirect_url.trim(),
        )
        .ok() else {
            self.status = "Invalid OAuth profile".to_string();
            return;
        };

        let workflow = match OAuthWorkflow::new(profile) {
            Ok(w) => w,
            Err(err) => {
                self.status = format!("OAuth validation failed: {err}");
                return;
            }
        };
        if self.oauth.expected_state.trim().is_empty() {
            self.status = "OAuth session missing expected state; restart flow".to_string();
            return;
        }
        if self.oauth.csrf_state.trim().is_empty() {
            self.status = "OAuth callback state is required".to_string();
            return;
        }
        if self.oauth.csrf_state.trim() != self.oauth.expected_state {
            self.status = "OAuth state mismatch".to_string();
            return;
        }

        let token = self
            .runtime
            .block_on(workflow.exchange_code(self.oauth.code.trim(), &self.oauth.pkce_verifier));

        let token = match token {
            Ok(token) => token,
            Err(err) => {
                self.status = format!("OAuth exchange failed: {err}");
                return;
            }
        };

        let now = Utc::now();
        let account = Account {
            id: Uuid::new_v4(),
            provider: self.oauth.provider.clone(),
            protocols: vec![
                AccountProtocol::ImapSmtp,
                AccountProtocol::GoogleCalendar,
                AccountProtocol::GoogleTasks,
            ],
            display_name: self.oauth.display_name.clone(),
            email_address: self.oauth.email.clone(),
            oauth_profile: Some(
                oauth_profile_for_provider(
                    self.oauth.provider.clone(),
                    self.oauth.client_id.trim(),
                    self.oauth.redirect_url.trim(),
                )
                .expect("validated profile"),
            ),
            created_at: now,
            updated_at: now,
        };

        let settings = serde_json::json!({
            "email": {
                "imap_host": "imap.gmail.com",
                "imap_port": 993,
                "smtp_host": "smtp.gmail.com",
                "smtp_port": 465,
                "endpoint": null,
                "username": self.oauth.email,
                "password": null,
                "access_token": null,
                "offline_sync_limit": self.oauth.sync_limit,
            },
            "calendar": {
                "endpoint": "https://www.googleapis.com/calendar/v3",
                "access_token": null,
                "calendar_id": "primary"
            },
            "tasks": {
                "endpoint": "https://tasks.googleapis.com/tasks/v1",
                "access_token": null,
                "list_id": "@default"
            }
        });

        let result = self.runtime.block_on(async {
            self.storage.upsert_account(&account).await?;
            self.storage
                .upsert_account_protocol_settings(account.id, &settings)
                .await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => {
                let _ = set_secret_guarded(
                    &self.secrets,
                    SecretKey {
                        namespace: "oauth_access_token".to_string(),
                        id: account.id.to_string(),
                    },
                    &token.access_token,
                );
                if let Some(refresh_token) = token.refresh_token {
                    let _ = set_secret_guarded(
                        &self.secrets,
                        SecretKey {
                            namespace: "oauth_refresh_token".to_string(),
                            id: account.id.to_string(),
                        },
                        &refresh_token,
                    );
                }
                self.oauth.code.clear();
                self.oauth.csrf_state.clear();
                self.oauth.expected_state.clear();
                self.reload_accounts();
                self.status = "OAuth account added".to_string();
            }
            Err(err) => self.status = format!("save account failed: {err}"),
        }
    }
    fn load_email_settings(&self, account_id: Uuid) -> Result<ProtocolSettings, String> {
        let raw = self
            .runtime
            .block_on(self.storage.account_protocol_settings(account_id))
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "protocol settings missing".to_string())?;

        parse_domain_settings(&raw, "email").map_err(|err| err.to_string())
    }

    fn load_calendar_settings(&self, account_id: Uuid) -> Result<CalendarSettings, String> {
        let raw = self
            .runtime
            .block_on(self.storage.account_protocol_settings(account_id))
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "protocol settings missing".to_string())?;

        parse_domain_settings(&raw, "calendar").map_err(|err| err.to_string())
    }

    fn load_task_settings(&self, account_id: Uuid) -> Result<TaskSettings, String> {
        let raw = self
            .runtime
            .block_on(self.storage.account_protocol_settings(account_id))
            .map_err(|err| err.to_string())?
            .ok_or_else(|| "protocol settings missing".to_string())?;

        parse_domain_settings(&raw, "tasks").map_err(|err| err.to_string())
    }

    fn complete_generic_setup(&mut self) {
        if self.generic_setup.email.trim().is_empty() || self.generic_setup.imap_server.trim().is_empty() {
            self.status = "Please fill out all required fields".to_string();
            return;
        }

        let now = Utc::now();
        let account = Account {
            id: Uuid::new_v4(),
            provider: Provider::Generic,
            protocols: vec![AccountProtocol::ImapSmtp],
            display_name: self.generic_setup.display_name.clone(),
            email_address: self.generic_setup.email.clone(),
            oauth_profile: None,
            created_at: now,
            updated_at: now,
        };

        let settings = serde_json::json!({
            "email": {
                "imap_host": self.generic_setup.imap_server,
                "imap_port": self.generic_setup.imap_port,
                "smtp_host": self.generic_setup.smtp_server,
                "smtp_port": self.generic_setup.smtp_port,
                "endpoint": null,
                "username": self.generic_setup.email,
                "password": null,
                "access_token": null,
                "offline_sync_limit": self.oauth.sync_limit,
            }
        });

        let result = self.runtime.block_on(async {
            self.storage.upsert_account(&account).await?;
            self.storage
                .upsert_account_protocol_settings(account.id, &settings)
                .await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => {
                let _ = set_secret_guarded(
                    &self.secrets,
                    SecretKey {
                        namespace: "account_password".to_string(),
                        id: account.id.to_string(),
                    },
                    &self.generic_setup.password,
                );

                self.status = "Account added successfully!".to_string();
                self.reload_accounts();
                self.generic_setup = GenericSetupDraft::default();
            }
            Err(err) => {
                self.status = format!("Failed to save account: {err}");
            }
        }
    }

    fn show_setup_wizard(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading(egui::RichText::new("Welcome to Cove Mail").size(36.0).strong());
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Let's set up your first email account to get started.").size(18.0));
                ui.add_space(40.0);
                
                let frame = egui::Frame::window(&ctx.style())
                    .inner_margin(32.0)
                    .corner_radius(16.0)
                    .fill(ctx.style().visuals.panel_fill);
                
                frame.show(ui, |ui| {
                    ui.set_width(450.0);
                    
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height() - 20.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            
                            let mut provider_changed = false;
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Provider").strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    egui::ComboBox::from_id_salt("WizardProvider")
                                        .selected_text(format!("{:?}", self.oauth.provider))
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_value(&mut self.oauth.provider, Provider::Gmail, "Gmail").changed() { provider_changed = true; }
                                            if ui.selectable_value(&mut self.oauth.provider, Provider::Outlook, "Outlook").changed() { provider_changed = true; }
                                            if ui.selectable_value(&mut self.oauth.provider, Provider::Exchange, "Exchange").changed() { provider_changed = true; }
                                            if ui.selectable_value(&mut self.oauth.provider, Provider::Generic, "Generic IMAP/SMTP").changed() { provider_changed = true; }
                                        });
                                });
                            });
                            
                            if provider_changed {
                                // Default redirect URL for desktop clients
                                self.oauth.redirect_url = "http://127.0.0.1:8765/oauth/callback".to_string();
                            }
                            
                            if self.oauth.provider == Provider::Generic {
                                ui.group(|ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(egui::RichText::new("IMAP/SMTP Manual Setup").strong());
                                    ui.add_space(4.0);

                                    ui.horizontal(|ui| { ui.label("Email:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::TextEdit::singleline(&mut self.generic_setup.email).min_size(egui::vec2(250.0, 24.0)))); });
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| { ui.label("Password:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::TextEdit::singleline(&mut self.generic_setup.password).password(true).min_size(egui::vec2(250.0, 24.0)))); });
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| { ui.label("Display Name:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::TextEdit::singleline(&mut self.generic_setup.display_name).min_size(egui::vec2(250.0, 24.0)))); });
                                    ui.add_space(16.0);

                                    ui.horizontal(|ui| { ui.label("IMAP Server:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::TextEdit::singleline(&mut self.generic_setup.imap_server).min_size(egui::vec2(250.0, 24.0)))); });
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| { ui.label("IMAP Port:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::DragValue::new(&mut self.generic_setup.imap_port))); });
                                    ui.add_space(16.0);

                                    ui.horizontal(|ui| { ui.label("SMTP Server:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::TextEdit::singleline(&mut self.generic_setup.smtp_server).min_size(egui::vec2(250.0, 24.0)))); });
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| { ui.label("SMTP Port:"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::DragValue::new(&mut self.generic_setup.smtp_port))); });
                                    ui.add_space(16.0);

                                    if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(egui::RichText::new("Save Credentials").size(16.0).strong())).clicked() {
                                        self.complete_generic_setup();
                                        if !self.accounts.is_empty() {
                                            self.view = View::Inbox;
                                        }
                                    }
                                });
                            } else {
                                ui.add_space(16.0);
                                ui.group(|ui| {
                                    ui.set_width(ui.available_width());
                                    match self.oauth.provider {
                                        Provider::Gmail => {
                                            ui.label(egui::RichText::new("Google Cloud Setup:").strong());
                                            ui.label("1. Go to console.cloud.google.com and create a project.");
                                            ui.label("2. Enable the Gmail, Google Calendar, and Tasks APIs.");
                                            ui.label("3. Create an OAuth Client ID for a 'Desktop app' (or 'Web app' if you need a specific redirect).");
                                            ui.label("4. Copy the Client ID here.");
                                            ui.label("5. Ensure the Redirect URL matches what is configured in Google Cloud.");
                                        }
                                        Provider::Outlook | Provider::Exchange => {
                                            ui.label(egui::RichText::new("Azure Portal Setup:").strong());
                                            ui.label("1. Go to portal.azure.com and register an App (Azure AD).");
                                            ui.label("2. Under Authentication, add a 'Mobile and desktop applications' platform.");
                                            ui.label("3. Select the default redirect URI or specify one below.");
                                            ui.label("4. Copy the Application (client) ID here.");
                                        }
                                        _ => {
                                            ui.label("Please follow your provider's OAuth documentation.");
                                        }
                                    }
                                });
                                ui.add_space(16.0);
                                
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Email").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut self.oauth.email).min_size(egui::vec2(250.0, 24.0)));
                                    });
                                });
                                ui.add_space(16.0);
                                
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Display Name").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut self.oauth.display_name).min_size(egui::vec2(250.0, 24.0)));
                                    });
                                });
                                ui.add_space(16.0);
                                
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Client ID").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut self.oauth.client_id).min_size(egui::vec2(250.0, 24.0)));
                                    });
                                });
                                ui.add_space(16.0);
                                
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Redirect URL").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut self.oauth.redirect_url).min_size(egui::vec2(250.0, 24.0)));
                                    });
                                });
                                ui.add_space(32.0);
                                
                                if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(egui::RichText::new("Begin OAuth").size(16.0).strong())).clicked() {
                                    self.begin_oauth();
                                }
                                
                                if !self.oauth.auth_url.is_empty() {
                                    ui.add_space(32.0);
                                    ui.separator();
                                    ui.add_space(16.0);
                                    ui.label(egui::RichText::new("Please open this URL in your browser to authenticate:").strong());
                                    ui.hyperlink(&self.oauth.auth_url);
                                    ui.add_space(24.0);
                                    
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("State").strong());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add(egui::TextEdit::singleline(&mut self.oauth.csrf_state).min_size(egui::vec2(250.0, 24.0)));
                                        });
                                    });
                                    ui.add_space(16.0);
                                    
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Code").strong());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.add(egui::TextEdit::singleline(&mut self.oauth.code).min_size(egui::vec2(250.0, 24.0)));
                                        });
                                    });
                                    ui.add_space(32.0);
                                    
                                    if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(egui::RichText::new("Complete Setup").size(16.0).strong())).clicked() {
                                        self.complete_oauth();
                                        if !self.accounts.is_empty() {
                                            self.view = View::Inbox;
                                        }
                                    }
                                }
                            }
                        });
                    
                    if !self.status.is_empty() && self.status != "Ready" {
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new(&self.status).color(ui.visuals().warn_fg_color));
                    }
                });
            });
        });
    }
}

impl eframe::App for NativeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.view == View::SetupWizard {
            self.show_setup_wizard(ctx);
            return;
        }

        // Global keyboard shortcuts.
        let modifiers = ctx.input(|i| i.modifiers);
        if ctx.input(|i| i.key_pressed(egui::Key::K) && modifiers.command) {
            self.show_command_palette = !self.show_command_palette;
            self.command_query.clear();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::N) && modifiers.command) {
            self.show_compose_window = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_command_palette = false;
        }

        // Undo send countdown (5 seconds).
        if let Some((account, settings, outgoing, sent_at)) = self.undo_send_message.clone() {
            if sent_at.elapsed() >= std::time::Duration::from_secs(5) {
                match self.runtime.block_on(self.email.send(&account, &settings, &outgoing)) {
                    Ok(()) => self.status = "Message sent successfully".to_string(),
                    Err(err) => self.status = format!("Send failed: {err}")
                }
                self.undo_send_message = None;
            }
        }

        // Periodic notification check (every 30 seconds).
        if self.last_notification_check.elapsed() >= std::time::Duration::from_secs(30) {
            self.last_notification_check = std::time::Instant::now();
            
            self.process_scheduled_messages();
            let notif_config = &self.config.notifications;

            // New-mail notifications for the current thread list.
            self.notification_state.check_new_mail(notif_config, &self.thread_messages);

            // Calendar reminder notifications.
            if let Some(account_id) = self.selected_account {
                let now = Utc::now();
                let window_end = now + chrono::Duration::minutes(
                    *notif_config.reminder_minutes_before.iter().max().unwrap_or(&15) + 1
                );
                if let Ok(events) = self.runtime.block_on(
                    self.storage.list_calendar_events(account_id, now, window_end)
                ) {
                    self.notification_state.check_calendar_reminders(notif_config, &events);
                }

                if let Ok(tasks) = self.runtime.block_on(
                    self.storage.list_tasks(account_id)
                ) {
                    self.notification_state.check_task_reminders(notif_config, &tasks);
                }
            }
        }

        // Custom draggable titlebar with space for macOS traffic lights
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::default().fill(ctx.style().visuals.panel_fill).inner_margin(egui::Margin { left: 80, right: 8, top: 12, bottom: 8 })) // 80px left padding for macOS traffic lights
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.interact(ui.max_rect(), ui.id().with("drag"), egui::Sense::drag()).dragged() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }

                    for (view, label) in [
                        (View::Inbox, "Inbox"),
                        (View::Chat, "Chat"),
                        (View::Calendar, "Calendar"),
                        (View::Tasks, "Tasks"),
                        (View::Notes, "Notes"),
                        (View::Contacts, "Contacts"),
                        (View::Rules, "Rules"),
                        (View::Analytics, "Analytics"),
                        (View::Integrations, "Apps"),
                        (View::Ai, "AI"),
                        (View::Security, "Security"),
                        (View::Settings, "Settings"),
                    ] {
                        if ui.selectable_label(self.view == view, label).clicked() {
                            self.view = view;
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(self.unified_inbox, "Unified").on_hover_text("Unified inbox across all accounts").clicked() {
                        self.unified_inbox = !self.unified_inbox;
                        self.load_threads();
                    }
                    ui.separator();
                    if ui.button("Sync Now").clicked() {
                        self.run_sync_now();
                    }
                    if ui.button("Reload Accounts").clicked() {
                        self.reload_accounts();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(&self.status);
                    });
                });
            });

        egui::SidePanel::left("accounts").frame(egui::Frame::default().fill(ctx.style().visuals.panel_fill).inner_margin(12.0)).show(ctx, |ui| {
            ui.heading("Accounts");
            ui.add_space(8.0);
            let mut next_account = None;
            for account in &self.accounts {
                let selected = self.selected_account == Some(account.id);
                if ui
                    .selectable_label(selected, format!("{}", account.email_address))
                    .clicked()
                {
                    next_account = Some(account.id);
                }
            }
            if let Some(account_id) = next_account {
                self.selected_account = Some(account_id);
                self.load_folders(true);
                self.load_threads();
                self.load_thread_messages();
                self.load_chat_contacts();
                self.load_chat_messages();
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.view {
            View::Inbox => {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.mail_query);
                    if ui.button("Search").clicked() {
                        self.search_mail();
                    }
                    if ui.button("Refresh Folders").clicked() {
                        self.load_folders(true);
                    }
                    if ui.button("Load Threads").clicked() {
                        self.load_threads();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(egui::RichText::new("✏ Compose").strong()).clicked() {
                            self.show_compose_window = true;
                        }
                    });
                });
                ui.add_space(8.0);

                let available_height = ui.available_height();
                
                egui::SidePanel::left("folders_panel")
                    .resizable(true)
                    .default_width(200.0)
                    .width_range(150.0..=400.0)
                    .frame(egui::Frame::default().inner_margin(8.0))
                    .show_inside(ui, |ui| {
                        ui.heading(egui::RichText::new("Folders").strong());
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(available_height - 20.0)
                            .show(ui, |ui| {
                                let mut next_folder = None;
                                for folder in &self.folders {
                                    let is_selected = self.selected_folder == folder.path;
                                    let label = format!(
                                        "{} ({}/{})",
                                        folder.path, folder.unread_count, folder.total_count
                                    );
                                    
                                    let mut frame = egui::Frame::default()
                                        .inner_margin(egui::Margin::symmetric(8, 4))
                                        .corner_radius(6.0);
                                        
                                    if is_selected {
                                        frame = frame.fill(ui.visuals().selection.bg_fill);
                                    }
                                    
                                    frame.show(ui, |ui| {
                                        let text_color = if is_selected {
                                            ui.visuals().selection.stroke.color
                                        } else {
                                            ui.visuals().text_color()
                                        };
                                        if ui.add(egui::SelectableLabel::new(is_selected, egui::RichText::new(label).color(text_color))).clicked() {
                                            next_folder = Some(folder.path.clone());
                                        }
                                    });
                                }
                                if let Some(folder) = next_folder {
                                    self.selected_folder = folder;
                                    self.load_threads();
                                }
                            });
                    });

                egui::SidePanel::left("threads_panel")
                    .resizable(true)
                    .default_width(350.0)
                    .width_range(250.0..=600.0)
                    .frame(egui::Frame::default().inner_margin(8.0))
                    .show_inside(ui, |ui| {
                        ui.heading(egui::RichText::new("Threads").strong());
                        ui.add_space(4.0);
                        let mut next_thread = None;
                        egui::ScrollArea::vertical()
                            .max_height(available_height - 20.0)
                            .show(ui, |ui| {
                                for thread in &self.threads {
                                    let is_selected = self.selected_thread.as_deref() == Some(&thread.thread_id);
                                    
                                    let participants = if thread.participants.is_empty() {
                                        "No participants".to_string()
                                    } else {
                                        thread.participants.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
                                    };
                                    
                                    let mut frame = egui::Frame::window(&ctx.style())
                                        .inner_margin(12.0)
                                        .corner_radius(8.0);
                                        
                                    if is_selected {
                                        frame = frame.fill(ui.visuals().selection.bg_fill);
                                    }
                                    
                                    ui.add_space(4.0);
                                    let response = frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        let text_color = if is_selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
                                        let title_color = if is_selected { text_color } else { ui.visuals().strong_text_color() };
                                        
                                        ui.label(egui::RichText::new(&thread.subject).strong().color(title_color).size(15.0));
                                        ui.add_space(2.0);
                                        ui.label(egui::RichText::new(format!("{} ({} unread / {})", participants, thread.unread_count, thread.message_count)).color(text_color).size(13.0));
                                    }).response;
                                    
                                    if response.interact(egui::Sense::click()).clicked() {
                                        next_thread = Some(thread.thread_id.clone());
                                    }
                                }
                            });
                        if let Some(thread_id) = next_thread {
                            self.selected_thread = Some(thread_id);
                            self.load_thread_messages();
                        }
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::default().inner_margin(16.0))
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("Message").strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("AI Draft Reply").clicked() {
                                    if let Some(msg) = self.thread_messages.last() {
                                        let sender = msg.from.first()
                                            .map(|a| a.address.clone())
                                            .unwrap_or_default();
                                        let body = msg.body_text.as_deref().unwrap_or(&msg.preview);
                                        match self.runtime.block_on(self.ai.draft_reply_suggestion(
                                            &sender, &msg.subject, body,
                                            self.ai_mode.clone(), self.ai_cloud_provider.clone(),
                                        )) {
                                            Ok((reply, _)) => {
                                                self.compose_body = reply.output;
                                                self.compose_subject = if msg.subject.to_lowercase().starts_with("re:") {
                                                    msg.subject.clone()
                                                } else {
                                                    format!("Re: {}", msg.subject)
                                                };
                                                self.compose_to = sender;
                                                self.show_compose_window = true;
                                                self.status = "AI draft reply generated.".to_string();
                                            }
                                            Err(err) => self.status = format!("AI draft failed: {err}"),
                                        }
                                    }
                                }
                                if ui.small_button("AI Summarize").clicked() {
                                    let msgs: Vec<_> = self.thread_messages.iter().map(|m| {
                                        let sender = m.from.first()
                                            .map(|a| a.address.clone())
                                            .unwrap_or_default();
                                        let body = m.body_text.as_deref().unwrap_or(&m.preview).to_string();
                                        (sender, m.subject.clone(), body)
                                    }).collect();
                                    if !msgs.is_empty() {
                                        match self.runtime.block_on(self.ai.summarize_thread(
                                            &msgs, self.ai_mode.clone(), self.ai_cloud_provider.clone(),
                                        )) {
                                            Ok((summary, _)) => {
                                                self.ai_output = summary.output;
                                                self.status = "Thread summary generated.".to_string();
                                            }
                                            Err(err) => self.status = format!("AI summarize failed: {err}"),
                                        }
                                    }
                                }
                            });
                        });

                        // Show AI summary if available.
                        if !self.ai_output.is_empty() {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("AI Summary").strong().size(13.0));
                                    if ui.small_button("Dismiss").clicked() {
                                        self.ai_output.clear();
                                    }
                                });
                                ui.label(egui::RichText::new(&self.ai_output).size(13.0));
                            });
                        }
                        ui.add_space(4.0);
                        // Snapshot data needed from thread_messages before drawing.
                        let messages_snapshot: Vec<_> = self.thread_messages.iter().map(|m| {
                            (m.id, m.pinned, m.subject.clone(), m.preview.clone(),
                             m.received_at,
                             m.from.clone(), m.to.clone(), m.cc.clone(),
                             m.headers.clone(), m.attachments.clone(),
                             m.body_html.clone(), m.body_text.clone(),
                             m.flags.clone())
                        }).collect();
                        let selected_msg = self.selected_message;

                        let mut deferred_pin: Option<(Uuid, bool)> = None;
                        let mut deferred_snooze: Option<Uuid> = None;
                        let mut deferred_save: Option<(Uuid, String)> = None;
                        let mut deferred_open: Option<(Uuid, String)> = None;
                        let mut deferred_read: Option<(Uuid, bool)> = None;
                        let mut next_message = None;

                        egui::ScrollArea::vertical()
                            .max_height(available_height - 20.0)
                            .show(ui, |ui| {
                                for (msg_id, pinned, subject, preview, received_at,
                                     from, _to, _cc, headers, attachments,
                                     body_html, body_text, _flags) in &messages_snapshot
                                {
                                    let selected = selected_msg == Some(*msg_id);
                                    let mut frame = egui::Frame::window(&ctx.style())
                                        .inner_margin(16.0)
                                        .corner_radius(8.0);
                                    if selected {
                                        frame = frame.stroke(egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
                                    }

                                    ui.add_space(8.0);
                                    let response = frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        let sender = from.first()
                                            .map(|f| f.name.clone().unwrap_or_else(|| f.address.clone()))
                                            .unwrap_or_else(|| "Unknown sender".to_string());
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(&sender).strong().size(15.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(received_at.to_string()).size(12.0));
                                            });
                                        });
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(subject).strong().size(16.0));
                                        ui.add_space(8.0);

                                        if selected {
                                            // Action bar: Pin, Snooze, Unsubscribe, Tracker info
                                            ui.horizontal(|ui| {
                                                let pin_label = if *pinned { "Unpin" } else { "Pin" };
                                                if ui.small_button(pin_label).clicked() {
                                                    deferred_pin = Some((*msg_id, !pinned));
                                                }
                                                let read_label = if _flags.seen { "Mark Unread" } else { "Mark Read" };
                                                if ui.small_button(read_label).clicked() {
                                                    deferred_read = Some((*msg_id, !_flags.seen));
                                                }
                                                if ui.small_button("Snooze").clicked() {
                                                    deferred_snooze = Some(*msg_id);
                                                }
                                                // 1-click unsubscribe: check List-Unsubscribe header
                                                if let Some(unsub) = headers.get("List-Unsubscribe") {
                                                    if ui.small_button("Unsubscribe").on_hover_text(unsub).clicked() {
                                                        if let Some(url) = extract_unsubscribe_url(unsub) {
                                                            let _ = open::that(&url);
                                                        }
                                                    }
                                                }
                                                // Tracking pixel info
                                                if let Some(html) = body_html {
                                                    let trackers = EmailService::detect_trackers(html);
                                                    if !trackers.is_empty() {
                                                        ui.label(
                                                            egui::RichText::new(format!("Trackers: {}", trackers.join(", ")))
                                                                .size(11.0)
                                                                .color(egui::Color32::from_rgb(200, 100, 50))
                                                        );
                                                    }
                                                }
                                            });
                                            ui.add_space(4.0);

                                            if !attachments.is_empty() {
                                                ui.label(egui::RichText::new("Attachments:").strong());
                                                for attachment in attachments {
                                                    ui.horizontal(|ui| {
                                                        let size_str = if attachment.size >= 1_048_576 {
                                                            format!("{:.1} MB", attachment.size as f64 / 1_048_576.0)
                                                        } else if attachment.size >= 1024 {
                                                            format!("{:.0} KB", attachment.size as f64 / 1024.0)
                                                        } else {
                                                            format!("{} B", attachment.size)
                                                        };
                                                        ui.label(format!("{} ({})", attachment.file_name, size_str));

                                                        let ext = std::path::Path::new(&attachment.file_name)
                                                            .extension()
                                                            .and_then(|s| s.to_str())
                                                            .unwrap_or("")
                                                            .to_lowercase();
                                                        let is_dangerous = matches!(ext.as_str(), "exe" | "sh" | "bat" | "cmd" | "vbs" | "scr" | "js" | "jar" | "app" | "scpt");
                                                        
                                                        if is_dangerous {
                                                            ui.label(egui::RichText::new("Blocked").color(egui::Color32::RED).strong())
                                                                .on_hover_text("This file type is blocked for security reasons.");
                                                        } else {
                                                            if ui.small_button("Save").clicked() {
                                                                deferred_save = Some((attachment.id, attachment.file_name.clone()));
                                                            }
                                                            if ui.small_button("Open").clicked() {
                                                                deferred_open = Some((attachment.id, attachment.file_name.clone()));
                                                            }
                                                        }
                                                    });
                                                }
                                                ui.add_space(8.0);
                                            }
                                            let rendered = body_html.as_deref()
                                                .map(|html| html_render::render_html(ui, html))
                                                .unwrap_or(false);
                                            if !rendered {
                                                let body = body_text.as_deref().unwrap_or(preview);
                                                ui.label(egui::RichText::new(body).size(14.0).line_height(Some(20.0)));
                                            }
                                        } else {
                                            ui.label(egui::RichText::new(preview).size(13.0));
                                        }
                                    }).response;

                                    if response.interact(egui::Sense::click()).clicked() {
                                        next_message = Some(*msg_id);
                                    }
                                }
                            });

                        // Apply deferred actions after the borrow of thread_messages is released.
                        if let Some(message_id) = next_message {
                            self.selected_message = Some(message_id);
                        }
                        if let Some((msg_id, pin_value)) = deferred_pin {
                            let _ = self.runtime.block_on(self.email.set_pinned(msg_id, pin_value));
                            self.load_thread_messages();
                        }
                        if let Some((msg_id, read_value)) = deferred_read {
                            let _ = self.runtime.block_on(self.email.set_message_seen(msg_id, read_value));
                            self.load_thread_messages();
                            self.load_threads(); // To refresh unread count on the thread side panel
                        }
                        if let Some(msg_id) = deferred_snooze {
                            self.pending_snooze = Some(msg_id);
                        }
                        if let Some(save) = deferred_save {
                            self.pending_attachment_save = Some(save);
                        }
                        if let Some(open_att) = deferred_open {
                            self.pending_attachment_open = Some(open_att);
                        }
                    });

                let mut show_compose = self.show_compose_window;
                let mut close_window = false;
                if show_compose {
                    egui::Window::new("Compose Message")
                        .open(&mut show_compose)
                        .default_width(600.0)
                        .default_height(500.0)
                        .vscroll(true)
                        .show(ctx, |ui| {
                            // Magic Compose Panel Toggle
                            let magic_btn_text = if self.show_magic_compose {
                                "✨ Hide Magic Compose"
                            } else {
                                "✨ Magic Compose"
                            };
                            
                            if ui.button(magic_btn_text).clicked() {
                                self.show_magic_compose = !self.show_magic_compose;
                            }

                            if self.show_magic_compose {
                                ui.group(|ui| {
                                    ui.label(egui::RichText::new("Magic Message").heading());
                                    ui.label("Write a prompt for the AI to generate a message:");
                                    ui.text_edit_multiline(&mut self.magic_compose_prompt);
                                    
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt("format").selected_text(&self.magic_compose_format).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.magic_compose_format, "Email".to_string(), "Email");
                                            ui.selectable_value(&mut self.magic_compose_format, "Paragraph".to_string(), "Paragraph");
                                            ui.selectable_value(&mut self.magic_compose_format, "Bullet Points".to_string(), "Bullet Points");
                                        });
                                        
                                        egui::ComboBox::from_id_salt("tone").selected_text(&self.magic_compose_tone).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.magic_compose_tone, "Friendly".to_string(), "Friendly");
                                            ui.selectable_value(&mut self.magic_compose_tone, "Professional".to_string(), "Professional");
                                            ui.selectable_value(&mut self.magic_compose_tone, "Casual".to_string(), "Casual");
                                            ui.selectable_value(&mut self.magic_compose_tone, "Persuasive".to_string(), "Persuasive");
                                        });
                                        
                                        egui::ComboBox::from_id_salt("length").selected_text(&self.magic_compose_length).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.magic_compose_length, "Short".to_string(), "Short");
                                            ui.selectable_value(&mut self.magic_compose_length, "Medium".to_string(), "Medium");
                                            ui.selectable_value(&mut self.magic_compose_length, "Long".to_string(), "Long");
                                        });
                                    });
                                    
                                    if ui.button(egui::RichText::new("✨ Generate").strong().color(egui::Color32::from_rgb(0, 200, 255))).clicked() {
                                        self.generate_magic_message();
                                    }
                                });
                                ui.separator();
                            }

                            ui.label("To:");
                            let to_response = ui.text_edit_singleline(&mut self.compose_to);
                            // Contact autocomplete dropdown.
                            if to_response.changed() {
                                let query = self.compose_to.split(',').last().unwrap_or("").trim().to_string();
                                if query.len() >= 2 {
                                    self.contact_suggestions = self.runtime
                                        .block_on(self.email.autocomplete_contacts(&query, 8))
                                        .unwrap_or_default();
                                } else {
                                    self.contact_suggestions.clear();
                                }
                            }
                            if !self.contact_suggestions.is_empty() {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    let mut picked = None;
                                    for contact in &self.contact_suggestions {
                                        let label = if let Some(name) = &contact.display_name {
                                            format!("{name} <{}>", contact.email)
                                        } else {
                                            contact.email.clone()
                                        };
                                        if ui.selectable_label(false, &label).clicked() {
                                            picked = Some(contact.email.clone());
                                        }
                                    }
                                    if let Some(email) = picked {
                                        // Append to compose_to, replacing the last partial token.
                                        let parts: Vec<&str> = self.compose_to.split(',').collect();
                                        if parts.len() > 1 {
                                            let prefix = parts[..parts.len() - 1].join(",");
                                            self.compose_to = format!("{prefix}, {email}, ");
                                        } else {
                                            self.compose_to = format!("{email}, ");
                                        }
                                        self.contact_suggestions.clear();
                                    }
                                });
                            }
                            ui.label("Subject:");
                            ui.text_edit_singleline(&mut self.compose_subject);
                            ui.label("Message:");
                            ui.text_edit_multiline(&mut self.compose_body);
                            
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.attachment_path);
                                if ui.button("Add Attachment").clicked()
                                    && !self.attachment_path.trim().is_empty()
                                {
                                    self.attachment_paths.push(self.attachment_path.clone());
                                    self.attachment_path.clear();
                                }
                            });
                            
                            if !self.attachment_paths.is_empty() {
                                ui.label("Pending attachments:");
                                let mut remove_index = None;
                                for (index, path) in self.attachment_paths.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(path);
                                        if ui.button("Remove").clicked() {
                                            remove_index = Some(index);
                                        }
                                    });
                                }
                                if let Some(index) = remove_index {
                                    self.attachment_paths.remove(index);
                                }
                            }
                            
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                let send_btn = ui.button(egui::RichText::new("Send Now").strong().size(16.0).color(egui::Color32::WHITE));
                                if send_btn.clicked() {
                                    self.send_compose();
                                    close_window = true;
                                }
                                // Send Later: schedule for a future time.
                                egui::ComboBox::from_id_salt("send_later")
                                    .selected_text("Send Later")
                                    .show_ui(ui, |ui| {
                                        let now = Utc::now();
                                        for (label, when) in [
                                            ("In 1 hour", now + Duration::hours(1)),
                                            ("In 2 hours", now + Duration::hours(2)),
                                            ("Tomorrow 9 AM", {
                                                let h = now.hour() as i64;
                                                let offset = if h < 9 { 9 - h } else { 24 - h + 9 };
                                                now + Duration::hours(offset)
                                            }),
                                            ("Monday 9 AM", now + Duration::days(
                                                ((8 - now.weekday().num_days_from_monday() as i64) % 7).max(1) as u64 as i64
                                            )),
                                        ] {
                                            if ui.button(label).clicked() {
                                                self.status = format!("Scheduled for {}", when.format("%b %d %H:%M"));
                                                // TODO: Queue the message for scheduled sending.
                                                close_window = true;
                                            }
                                        }
                                    });
                            });
                        });
                }
                
                if close_window {
                    show_compose = false;
                }
                self.show_compose_window = show_compose;

                // Undo send banner.
                if self.undo_send_message.is_some() {
                    egui::TopBottomPanel::bottom("undo_send").frame(
                        egui::Frame::default().fill(egui::Color32::from_rgb(50, 50, 60)).inner_margin(8.0)
                    ).show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Message sent.").color(egui::Color32::WHITE));
                            if ui.button("Undo").clicked() {
                                self.undo_send_message = None;
                                self.status = "Send canceled.".to_string();
                            }
                            if let Some((_, _, _, sent_at)) = &self.undo_send_message {
                                let remaining = 5u64.saturating_sub(sent_at.elapsed().as_secs());
                                ui.label(egui::RichText::new(format!("{remaining}s")).color(egui::Color32::LIGHT_GRAY));
                            }
                        });
                    });
                }

                // Snooze dialog.
                if let Some(msg_id) = self.pending_snooze {
                    let mut close_snooze = false;
                    egui::Window::new("Snooze Message")
                        .collapsible(false)
                        .default_width(260.0)
                        .show(ctx, |ui| {
                            ui.label("Snooze until:");
                            let now = Utc::now();
                            let hours_to_9am = {
                                let h = now.hour() as i64;
                                if h < 9 { 9 - h } else { 24 - h + 9 }
                            };
                            for (label, duration) in [
                                ("Later today (3h)", chrono::Duration::hours(3)),
                                ("Tomorrow morning", chrono::Duration::hours(hours_to_9am)),
                                ("Next week", chrono::Duration::days(7)),
                            ] {
                                if ui.button(label).clicked() {
                                    let until = now + duration;
                                    let _ = self.runtime.block_on(self.email.snooze_message(msg_id, until));
                                    close_snooze = true;
                                    self.status = format!("Snoozed until {}", until.format("%b %d %H:%M"));
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close_snooze = true;
                            }
                        });
                    if close_snooze {
                        self.pending_snooze = None;
                        self.load_thread_messages();
                    }
                }

                // Command palette.
                if self.show_command_palette {
                    egui::Window::new("Command Palette")
                        .collapsible(false)
                        .title_bar(false)
                        .fixed_pos(egui::pos2(
                            ctx.screen_rect().width() / 2.0 - 200.0,
                            100.0,
                        ))
                        .default_width(400.0)
                        .show(ctx, |ui| {
                            let response = ui.text_edit_singleline(&mut self.command_query);
                            response.request_focus();
                            ui.add_space(4.0);

                            let commands: Vec<(&str, &str)> = vec![
                                ("Compose new message", "compose"),
                                ("Sync all accounts", "sync"),
                                ("Toggle unified inbox", "unified"),
                                ("Go to Inbox", "inbox"),
                                ("Go to Calendar", "calendar"),
                                ("Go to Tasks", "tasks"),
                                ("Go to Chat", "chat"),
                                ("Go to AI", "ai"),
                                ("Go to Security", "security"),
                                ("Go to Settings", "settings"),
                                ("Reload accounts", "reload"),
                            ];

                            let query_lower = self.command_query.to_lowercase();
                            let filtered: Vec<_> = commands.iter()
                                .filter(|(label, _)| query_lower.is_empty() || label.to_lowercase().contains(&query_lower))
                                .collect();

                            for (label, action) in &filtered {
                                if ui.selectable_label(false, *label).clicked() {
                                    match *action {
                                        "compose" => self.show_compose_window = true,
                                        "sync" => self.run_sync_now(),
                                        "unified" => {
                                            self.unified_inbox = !self.unified_inbox;
                                            self.load_threads();
                                        }
                                        "inbox" => self.view = View::Inbox,
                                        "calendar" => self.view = View::Calendar,
                                        "tasks" => self.view = View::Tasks,
                                        "chat" => self.view = View::Chat,
                                        "ai" => self.view = View::Ai,
                                        "security" => self.view = View::Security,
                                        "settings" => self.view = View::Settings,
                                        "reload" => self.reload_accounts(),
                                        _ => {}
                                    }
                                    self.show_command_palette = false;
                                    self.command_query.clear();
                                }
                            }
                        });
                }
            }
            View::Chat => {
                ui.horizontal(|ui| {
                    if ui.button("Refresh Contacts").clicked() {
                        self.load_chat_contacts();
                    }
                });

                ui.add_space(8.0);

                let available_height = ui.available_height();
                
                egui::SidePanel::left("contacts_panel")
                    .resizable(true)
                    .default_width(300.0)
                    .width_range(200.0..=500.0)
                    .frame(egui::Frame::default().inner_margin(8.0))
                    .show_inside(ui, |ui| {
                        ui.heading(egui::RichText::new("Recent Contacts").strong());
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(available_height - 20.0)
                            .show(ui, |ui| {
                                let mut next_contact = None;
                                for contact in &self.chat_contacts {
                                    let is_selected = self.selected_chat_contact.as_deref() == Some(&contact.email_address);
                                    let name = contact.display_name.as_deref().unwrap_or(&contact.email_address);
                                    
                                    let mut frame = egui::Frame::window(&ctx.style())
                                        .inner_margin(12.0)
                                        .corner_radius(8.0);
                                        
                                    if is_selected {
                                        frame = frame.fill(ui.visuals().selection.bg_fill);
                                    }
                                    
                                    ui.add_space(4.0);
                                    let response = frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        let text_color = if is_selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
                                        let title_color = if is_selected { text_color } else { ui.visuals().strong_text_color() };
                                        
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(name).strong().color(title_color).size(15.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if contact.unread_count > 0 {
                                                    ui.label(egui::RichText::new(format!("{} unread", contact.unread_count)).color(ui.visuals().warn_fg_color).size(12.0));
                                                } else {
                                                    ui.label(egui::RichText::new(format!("{} msgs", contact.message_count)).color(text_color).size(12.0));
                                                }
                                            });
                                        });
                                        ui.add_space(2.0);
                                        ui.label(egui::RichText::new(&contact.latest_subject).color(text_color).size(13.0));
                                    }).response;
                                    
                                    if response.interact(egui::Sense::click()).clicked() {
                                        next_contact = Some(contact.email_address.clone());
                                    }
                                }
                                if let Some(contact_email) = next_contact {
                                    self.selected_chat_contact = Some(contact_email);
                                    self.load_chat_messages();
                                }
                            });
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::default().inner_margin(16.0))
                    .show_inside(ui, |ui| {
                        ui.heading(egui::RichText::new("Conversation").strong());
                        ui.add_space(4.0);
                        
                        let chat_height = available_height - 120.0;
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(chat_height)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                let my_email = self.account().map(|a| a.email_address.clone()).unwrap_or_default();
                                for message in &self.chat_messages {
                                    let is_me = message.from.first().map(|addr| addr.address.eq_ignore_ascii_case(&my_email)).unwrap_or(false);
                                    
                                    ui.horizontal(|ui| {
                                        if is_me {
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                                let frame = egui::Frame::default()
                                                    .fill(ui.visuals().selection.bg_fill)
                                                    .inner_margin(egui::Margin::symmetric(12, 8))
                                                    .corner_radius(12.0);
                                                frame.show(ui, |ui| {
                                                    ui.label(egui::RichText::new(ag_chat_bubble_text(message)).color(ui.visuals().selection.stroke.color).size(14.0));
                                                });
                                            });
                                        } else {
                                            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                                                let frame = egui::Frame::window(&ctx.style())
                                                    .inner_margin(egui::Margin::symmetric(12, 8))
                                                    .corner_radius(12.0);
                                                frame.show(ui, |ui| {
                                                    ui.label(egui::RichText::new(ag_chat_bubble_text(message)).size(14.0));
                                                });
                                            });
                                        }
                                    });
                                    ui.add_space(8.0);
                                }
                            });

                        ui.separator();
                        ui.horizontal(|ui| {
                            let text_edit = egui::TextEdit::multiline(&mut self.chat_compose_body)
                                .desired_width(ui.available_width() - 80.0)
                                .margin(egui::Margin::symmetric(12, 8));
                            ui.add(text_edit);
                            if ui.button(egui::RichText::new("Send").strong().size(15.0)).clicked() {
                                self.send_chat_reply();
                            }
                        });
                    });
            }
            View::Calendar => {
                ui.heading("Calendar");

                if let Some(account_id) = self.selected_account {
                    let now = Utc::now();
                    let start = now - Duration::days(7);
                    let end = now + Duration::days(30);

                    match self.runtime.block_on(self.storage.list_calendar_events(account_id, start, end)) {
                        Ok(events) => {
                            if events.is_empty() {
                                ui.label("No calendar events found. Try syncing first.");
                            }
                            let mut rsvp_change: Option<(Uuid, cove_core::RsvpStatus)> = None;

                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for event in &events {
                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(&event.title).strong().size(15.0));
                                            if event.all_day {
                                                ui.label(egui::RichText::new("All Day").size(11.0).italics());
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(
                                                format!("{} → {}",
                                                    event.starts_at.format("%b %d %H:%M"),
                                                    event.ends_at.format("%H:%M"))
                                            ).size(13.0));
                                            if let Some(loc) = &event.location {
                                                ui.label(egui::RichText::new(format!("@ {loc}")).size(13.0));
                                            }
                                        });
                                        // Recurrence display
                                        if let Some(rrule) = &event.recurrence_rule {
                                            ui.label(egui::RichText::new(format!("Repeats: {rrule}")).size(11.0).italics());
                                        }
                                        // Attendees
                                        if !event.attendees.is_empty() {
                                            ui.label(egui::RichText::new(
                                                format!("Attendees: {}", event.attendees.join(", "))
                                            ).size(11.0));
                                        }
                                        // RSVP buttons
                                        if event.organizer.is_some() && !event.attendees.is_empty() {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new(format!("RSVP: {:?}", event.rsvp_status)).size(12.0));
                                                if ui.small_button("Accept").clicked() {
                                                    rsvp_change = Some((event.id, cove_core::RsvpStatus::Accepted));
                                                }
                                                if ui.small_button("Tentative").clicked() {
                                                    rsvp_change = Some((event.id, cove_core::RsvpStatus::Tentative));
                                                }
                                                if ui.small_button("Decline").clicked() {
                                                    rsvp_change = Some((event.id, cove_core::RsvpStatus::Declined));
                                                }
                                            });
                                        }
                                    });
                                    ui.add_space(4.0);
                                }
                            });

                            if let Some((event_id, new_status)) = rsvp_change {
                                let _ = self.runtime.block_on(
                                    self.storage.update_rsvp_status(event_id, &new_status)
                                );
                                self.status = format!("RSVP updated to {:?}", new_status);
                            }
                        }
                        Err(err) => {
                            ui.label(format!("Calendar load failed: {err}"));
                        }
                    }
                } else {
                    ui.label("Select an account to view calendar events.");
                }
            }
            View::Notes => {
                ui.heading("Notes Workspace");
                ui.add_space(8.0);
                ui.label("A dedicated space for rich-text notes synced across accounts.");
            }
            View::Tasks => {
                ui.heading("Tasks");
                if let Some(account) = self.account() {
                    let account_id = account.id;
                    // Priority view toggle (using priority-sorted query).
                    match self.runtime.block_on(self.storage.list_tasks_by_priority(account_id)) {
                        Ok(tasks) => {
                            if tasks.is_empty() {
                                ui.label("No tasks. Try syncing first.");
                            }
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for task in &tasks {
                                    let priority_color = match task.priority {
                                        cove_core::TaskPriority::Critical => egui::Color32::from_rgb(220, 50, 50),
                                        cove_core::TaskPriority::High => egui::Color32::from_rgb(220, 150, 50),
                                        cove_core::TaskPriority::Normal => egui::Color32::from_rgb(150, 150, 220),
                                        cove_core::TaskPriority::Low => egui::Color32::from_rgb(120, 120, 120),
                                    };
                                    let completed = task.completed_at.is_some();
                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(format!("[{:?}]", task.priority))
                                                .size(11.0).color(priority_color));
                                            let title_text = egui::RichText::new(&task.title).size(14.0);
                                            if completed {
                                                ui.label(title_text.strikethrough());
                                            } else {
                                                ui.label(title_text);
                                            }
                                            if let Some(due) = task.due_at {
                                                ui.label(egui::RichText::new(
                                                    format!("Due: {}", due.format("%b %d %H:%M"))
                                                ).size(11.0));
                                            }
                                        });

                                        // Subtasks
                                        if let Ok(subtasks) = self.runtime.block_on(
                                            self.storage.list_subtasks(task.id)
                                        ) {
                                            if !subtasks.is_empty() {
                                                ui.indent(task.id, |ui| {
                                                    for sub in &subtasks {
                                                        let sub_done = sub.completed_at.is_some();
                                                        let sub_text = egui::RichText::new(format!("↳ {}", sub.title)).size(12.0);
                                                        if sub_done {
                                                            ui.label(sub_text.strikethrough());
                                                        } else {
                                                            ui.label(sub_text);
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    });
                                }
                            });
                        }
                        Err(err) => {
                            ui.label(format!("load tasks failed: {err}"));
                        }
                    }
                }
            }
            View::Ai => {
                ui.heading("Global AI Settings");
                ui.horizontal(|ui| {
                    ui.label("AI Mode:");
                    egui::ComboBox::from_id_salt("GlobalAiMode").selected_text(format!("{:?}", self.ai_mode)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ai_mode, AiMode::Local, "Local");
                        ui.selectable_value(&mut self.ai_mode, AiMode::Cloud, "Cloud");
                    });
                    
                    if self.ai_mode == AiMode::Cloud {
                        ui.label("Provider:");
                        egui::ComboBox::from_id_salt("GlobalCloudProvider").selected_text(format!("{:?}", self.ai_cloud_provider.as_ref().unwrap())).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::OpenAi), "OpenAI");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::Anthropic), "Anthropic");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::Gemini), "Gemini");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::Mistral), "Mistral");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::Groq), "Groq");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::Grok), "Grok");
                            ui.selectable_value(&mut self.ai_cloud_provider, Some(CloudAiProvider::OpenRouter), "OpenRouter");
                        });
                    }
                });
                
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.label(egui::RichText::new("AI Opt-In Toggles").strong());
                    ui.label(egui::RichText::new("Enable specific features to use Cloud APIs. Note: Enabling cloud processing sends raw text content to external providers.").size(11.0).color(ui.visuals().warn_fg_color));
                    let mut b1 = true; let mut b2 = true; let mut b3 = false;
                    ui.checkbox(&mut b1, "Enable AI Thread Summaries");
                    ui.checkbox(&mut b2, "Enable AI Draft Suggestions");
                    ui.checkbox(&mut b3, "Enable AI Inbox Categorization (Experimental)");
                });
                
                ui.separator();

                ui.heading("Test AI");
                ui.text_edit_singleline(&mut self.ai_subject);
                ui.text_edit_multiline(&mut self.ai_body);
                if ui.button("Summarize").clicked() {
                    self.summarize_ai();
                }
                ui.separator();
                ui.label(&self.ai_output);
                
                ui.separator();
                ui.heading("Cloud AI Keys");
                
                let mut ai_keys = vec![
                    ("OpenAI", "openai", &mut self.openai_key),
                    ("Anthropic", "anthropic", &mut self.anthropic_key),
                    ("Gemini", "gemini", &mut self.gemini_key),
                    ("Mistral", "mistral", &mut self.mistral_key),
                    ("Groq", "groq", &mut self.groq_key),
                    ("Grok", "grok", &mut self.grok_key),
                    ("OpenRouter", "openrouter", &mut self.openrouter_key),
                ];
                
                for (label_text, id, key_val) in ai_keys.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(*label_text);
                        ui.text_edit_singleline(*key_val);
                        if ui.button("Save").clicked() {
                            match set_secret_guarded(
                                &self.secrets,
                                SecretKey {
                                    namespace: "ai_api_key".to_string(),
                                    id: id.to_string(),
                                },
                                key_val,
                            ) {
                                Ok(()) => self.status = format!("{label_text} key saved"),
                                Err(err) => self.status = format!("{label_text} save err: {err}"),
                            }
                        }
                    });
                }
            }
            View::Security => {
                ui.heading("Security + OAuth");
                ui.label("Secrets are stored in OS keychain namespaces.");
                ui.separator();
                
                ui.heading("Settings Export / Import");
                ui.label("Encrypt and backup your configuration, accounts, and secrets.");

                ui.separator();
                ui.heading("Advanced Privacy Controls");
                ui.label("Data Provenance & Local-First Guarantees:");
                ui.add_space(8.0);
                
                ui.horizontal(|ui| {
                    ui.label("Email Syncing:");
                    ui.label(egui::RichText::new("Local SQLite (SQLCipher encryption on-disk)").color(ui.visuals().warn_fg_color));
                });
                ui.horizontal(|ui| {
                    ui.label("Search/Indexing:");
                    ui.label(egui::RichText::new("Locally executed offline indexes").color(ui.visuals().warn_fg_color));
                });
                ui.horizontal(|ui| {
                    ui.label("Cloud AI features:");
                    ui.label(egui::RichText::new("Requires opt-in. Raw email texts are sent to configured LLM APIs.").color(ui.visuals().warn_fg_color));
                });
                ui.horizontal(|ui| {
                    ui.label("Analytics/Telemetry:");
                    ui.label(egui::RichText::new("0 Telemetry. No remote crash reporting.").color(ui.visuals().warn_fg_color));
                });
                
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let mut b1 = true;
                    ui.checkbox(&mut b1, "Block remote trackers/pixels automatically");
                });
                
                ui.add_space(8.0);
                ui.heading("Account Purge");
                if ui.button(egui::RichText::new("Purge Local Cache & Secrets Data for Selected Account").color(egui::Color32::RED)).clicked() {
                    self.status = "Data purging is implemented via CLI at this time.".to_string();
                }

                ui.separator();
                
                ui.horizontal(|ui| {
                    ui.label("Export Password:");
                    ui.add(egui::TextEdit::singleline(&mut self.export_password).password(true));
                    if ui.button("Export Settings").clicked() {
                        if self.export_password.is_empty() {
                            self.status = "Export password cannot be empty.".to_string();
                        } else if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("backup.age")
                            .add_filter("Age Encrypted Backup", &["age"])
                            .save_file()
                        {
                            self.status = "Gathering settings...".to_string();
                            let mut accounts_export = Vec::new();
                            for account in &self.accounts {
                                let protocol_json = match self.runtime.block_on(self.storage.account_protocol_settings(account.id)) {
                                    Ok(json) => json,
                                    Err(err) => {
                                        self.status = format!("Export error: {}", err);
                                        break; // In real app, might just skip or fail gracefully
                                    }
                                };
                                
                                let mut secrets = BTreeMap::new();
                                // Best effort extraction of associated secrets.
                                for ns in ["account_password", "oauth_refresh_token", "oauth_access_token"] {
                                    let key = SecretKey {
                                        namespace: ns.to_string(),
                                        id: account.id.to_string()
                                    };
                                    if let Ok(Some(secret_val)) = self.secrets.get(&key) {
                                        secrets.insert(ns.to_string(), secret_val);
                                    }
                                }
                                
                                accounts_export.push(export::AccountExport {
                                    account: account.clone(),
                                    protocol_settings_json: protocol_json,
                                    secrets,
                                });
                            }
                            
                            let payload = export::ExportPayload {
                                config: self.config.clone(),
                                sqlcipher_key: None, // Simplified for this implementation
                                accounts: accounts_export,
                            };
                            
                            match export::export_settings(&payload, &self.export_password, &path) {
                                Ok(()) => {
                                    self.status = format!("Exported successfully to {}", path.display());
                                    self.export_password.clear();
                                }
                                Err(err) => {
                                    self.status = format!("Export failed: {}", err);
                                }
                            }
                        }
                    }
                });
                
                ui.horizontal(|ui| {
                    ui.label("Import Password:");
                    ui.add(egui::TextEdit::singleline(&mut self.import_password).password(true));
                    if ui.button("Import Settings").clicked() {
                        if self.import_password.is_empty() {
                            self.status = "Import password cannot be empty.".to_string();
                        } else if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Age Encrypted Backup", &["age"])
                            .pick_file()
                        {
                            self.status = "Importing settings...".to_string();
                            let password_clone = self.import_password.clone();
                            match export::import_settings(&password_clone, &path) {
                                Ok(payload) => {
                                    self.config = payload.config;
                                    // Save config to disk
                                    if let Err(err) = self.config_manager.save(&self.config) {
                                        self.status = format!("Failed to save imported config: {}", err);
                                    } else {
                                        // Restore accounts and secrets
                                        let mut import_success = true;
                                        for acc_export in payload.accounts {
                                            if let Err(err) = self.runtime.block_on(self.storage.upsert_account(&acc_export.account)) {
                                                self.status = format!("Failed to import account {}: {}", acc_export.account.id, err);
                                                import_success = false;
                                                break;
                                            }
                                            
                                            // Restore protocol settings
                                            if let Some(settings_json) = acc_export.protocol_settings_json {
                                                if let Err(err) = self.runtime.block_on(self.storage.upsert_account_protocol_settings(acc_export.account.id, &settings_json)) {
                                                    self.status = format!("Failed to import settings for account {}: {}", acc_export.account.id, err);
                                                    import_success = false;
                                                    break;
                                                }
                                            }
                                            
                                            // Restore secrets
                                            for (ns, val) in acc_export.secrets {
                                                let key = SecretKey {
                                                    namespace: ns,
                                                    id: acc_export.account.id.to_string(),
                                                };
                                                if let Err(err) = set_secret_guarded(&self.secrets, key, &val) {
                                                     self.status = format!("Failed to import secret for account {}: {}", acc_export.account.id, err);
                                                     import_success = false;
                                                     break;
                                                }
                                            }
                                        }
                                        
                                        if import_success {
                                            self.status = format!("Imported successfully from {}", path.display());
                                            self.import_password.clear();
                                            self.reload_accounts(); // Refresh UI State
                                        }
                                    }
                                }
                                Err(err) => {
                                    self.status = format!("Import failed: {}", err);
                                }
                            }
                        }
                    }
                });
                
                ui.separator();
                egui::ComboBox::from_label("Provider")
                    .selected_text(format!("{:?}", self.oauth.provider))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.oauth.provider, Provider::Gmail, "Gmail");
                        ui.selectable_value(&mut self.oauth.provider, Provider::Outlook, "Outlook");
                        ui.selectable_value(&mut self.oauth.provider, Provider::Exchange, "Exchange");
                    });
                ui.horizontal(|ui| {
                    ui.label("Email");
                    ui.text_edit_singleline(&mut self.oauth.email);
                });
                ui.horizontal(|ui| {
                    ui.label("Display");
                    ui.text_edit_singleline(&mut self.oauth.display_name);
                });
                ui.horizontal(|ui| {
                    ui.label("Client ID");
                    ui.text_edit_singleline(&mut self.oauth.client_id);
                });
                ui.horizontal(|ui| {
                    ui.label("Redirect");
                    ui.text_edit_singleline(&mut self.oauth.redirect_url);
                });
                ui.horizontal(|ui| {
                    let label = match self.oauth.sync_limit {
                        cove_core::OfflineSyncLimit::All => "All Time".to_string(),
                        cove_core::OfflineSyncLimit::Days(d) => format!("Last {d} Days"),
                    };
                    egui::ComboBox::from_label("Offline Sync Limit")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.oauth.sync_limit, cove_core::OfflineSyncLimit::Days(30), "Last 30 Days");
                            ui.selectable_value(&mut self.oauth.sync_limit, cove_core::OfflineSyncLimit::Days(90), "Last 90 Days");
                            ui.selectable_value(&mut self.oauth.sync_limit, cove_core::OfflineSyncLimit::All, "All Time");
                        });
                });
                if ui.button("Begin OAuth").clicked() {
                    self.begin_oauth();
                }
                ui.label(&self.oauth.auth_url);
                ui.horizontal(|ui| {
                    ui.label("State");
                    ui.text_edit_singleline(&mut self.oauth.csrf_state);
                });
                ui.horizontal(|ui| {
                    ui.label("Code");
                    ui.text_edit_singleline(&mut self.oauth.code);
                });
                if ui.button("Complete OAuth").clicked() {
                    self.complete_oauth();
                }
            }
            View::Settings => {
                ui.heading("Settings");
                ui.add_space(8.0);

                // -- Signatures --
                egui::CollapsingHeader::new(egui::RichText::new("Email Signatures").heading())
                    .default_open(false)
                    .show(ui, |ui| {
                        let sigs = self.runtime.block_on(self.storage.list_signatures(self.selected_account))
                            .unwrap_or_default();
                        if sigs.is_empty() {
                            ui.label("No signatures configured.");
                        }
                        let mut delete_sig = None;
                        for sig in &sigs {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&sig.name).strong());
                                    if sig.is_default {
                                        ui.label(egui::RichText::new("(default)").italics());
                                    }
                                    if ui.small_button("Delete").clicked() {
                                        delete_sig = Some(sig.id);
                                    }
                                });
                                ui.label(&sig.body_text);
                            });
                        }
                        if let Some(sig_id) = delete_sig {
                            let _ = self.runtime.block_on(self.storage.delete_signature(sig_id));
                        }
                    });

                ui.add_space(8.0);

                // -- Templates --
                egui::CollapsingHeader::new(egui::RichText::new("Email Templates").heading())
                    .default_open(false)
                    .show(ui, |ui| {
                        let templates = self.runtime.block_on(self.storage.list_templates())
                            .unwrap_or_default();
                        if templates.is_empty() {
                            ui.label("No templates configured.");
                        }
                        let mut delete_tmpl = None;
                        for tmpl in &templates {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&tmpl.name).strong());
                                    if ui.small_button("Use").clicked() {
                                        self.compose_subject = tmpl.subject.clone();
                                        self.compose_body = tmpl.body_text.clone();
                                        self.show_compose_window = true;
                                    }
                                    if ui.small_button("Delete").clicked() {
                                        delete_tmpl = Some(tmpl.id);
                                    }
                                });
                                ui.label(format!("Subject: {}", tmpl.subject));
                            });
                        }
                        if let Some(tmpl_id) = delete_tmpl {
                            let _ = self.runtime.block_on(self.storage.delete_template(tmpl_id));
                        }
                    });

                ui.add_space(8.0);

                // -- Rules / Filters --
                egui::CollapsingHeader::new(egui::RichText::new("Mail Rules / Filters").heading())
                    .default_open(false)
                    .show(ui, |ui| {
                        let rules = self.runtime.block_on(self.storage.list_rules())
                            .unwrap_or_default();
                        if rules.is_empty() {
                            ui.label("No rules configured.");
                        }
                        let mut delete_rule = None;
                        for rule in &rules {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    let status = if rule.enabled { "ON" } else { "OFF" };
                                    ui.label(egui::RichText::new(&rule.name).strong());
                                    ui.label(egui::RichText::new(format!("[{status}]")).size(11.0));
                                    if ui.small_button("Delete").clicked() {
                                        delete_rule = Some(rule.id);
                                    }
                                });
                                let cond_text: Vec<String> = rule.conditions.iter().map(|c| {
                                    format!("{:?} {:?} '{}'", c.field, c.operator, c.value)
                                }).collect();
                                ui.label(format!("If {}: {}", if rule.match_all { "ALL" } else { "ANY" }, cond_text.join(", ")));
                                let action_text: Vec<String> = rule.actions.iter().map(|a| format!("{a:?}")).collect();
                                ui.label(format!("Then: {}", action_text.join(", ")));
                            });
                        }
                        if let Some(rule_id) = delete_rule {
                            let _ = self.runtime.block_on(self.storage.delete_rule(rule_id));
                        }
                    });

                ui.add_space(8.0);

                // -- Follow-Up Tracking Controls --
                egui::CollapsingHeader::new(egui::RichText::new("Follow-Up Tracking").heading())
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label("Configure how the app reminds you of unanswered emails:");
                        ui.horizontal(|ui| {
                            let mut tracking_enabled = true;
                            ui.checkbox(&mut tracking_enabled, "Enable Follow-Up Tracking");
                        });
                        ui.horizontal(|ui| {
                            let mut days = 3;
                            ui.label("Remind after:");
                            ui.add(egui::DragValue::new(&mut days).range(1..=30).suffix(" days"));
                        });
                        ui.label(egui::RichText::new("Note: Notifications will be triggered for emails sent where no reply was received.").size(11.0).italics());
                    });

                ui.add_space(8.0);

                // -- Search operators help --
                egui::CollapsingHeader::new(egui::RichText::new("Search Operators").heading())
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label("Use search operators in the mail search bar:");
                        ui.add_space(4.0);
                        for (op, desc) in [
                            ("from:alice@example.com", "Messages from a specific sender"),
                            ("to:bob@example.com", "Messages to a specific recipient"),
                            ("subject:meeting", "Messages with subject containing 'meeting'"),
                            ("has:attachment", "Messages with attachments"),
                            ("is:unread", "Unread messages"),
                            ("is:pinned", "Pinned messages"),
                            ("before:2025-01-01", "Messages before a date"),
                            ("after:2025-06-01", "Messages after a date"),
                            ("label:important", "Messages with a label"),
                        ] {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(op).monospace().strong());
                                ui.label(desc);
                            });
                        }
                    });
            }
            View::SetupWizard => {
                ui.heading("Setup Wizard");
                ui.label("Setup your accounts here.");
            }
            View::Contacts => {
                ui.heading("Contacts Management");
                ui.add_space(8.0);
                ui.label("CRM Lite: Aggregate communication history and manage contact templates.");
            }
            View::Rules => {
                ui.heading("Local Rules Engine");
                ui.add_space(8.0);
                ui.label("Robust local filtering and actions. Configure your Thunderbird-class rules here.");
            }
            View::Analytics => {
                ui.heading("Analytics & Read Status");
                ui.add_space(8.0);
                ui.label("Superhuman-class Response-time and Workflow dashbaords.");
            }
            View::Integrations => {
                ui.heading("App Integrations");
                ui.add_space(8.0);
                ui.label("Connect Slack, WhatsApp, Asana and other external services (Mailbird parity).");
            }
        });

        // Process pending attachment save/open after UI draw.
        if let Some((att_id, file_name)) = self.pending_attachment_save.take() {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&file_name)
                .save_file()
            {
                match self.runtime.block_on(self.email.get_attachment_content(att_id)) {
                    Ok(Some(data)) => {
                        if let Err(e) = std::fs::write(&path, &data) {
                            self.status = format!("Save failed: {e}");
                        } else {
                            self.status = format!("Saved to {}", path.display());
                        }
                    }
                    Ok(None) => {
                        self.status = "Attachment content not available offline.".to_string();
                    }
                    Err(e) => {
                        self.status = format!("Error loading attachment: {e}");
                    }
                }
            }
        }
        if let Some((att_id, file_name)) = self.pending_attachment_open.take() {
            match self.runtime.block_on(self.email.get_attachment_content(att_id)) {
                Ok(Some(data)) => {
                    let tmp_dir = std::env::temp_dir().join("cove-attachments");
                    let _ = std::fs::create_dir_all(&tmp_dir);
                    let tmp_path = tmp_dir.join(&file_name);
                    match std::fs::write(&tmp_path, &data) {
                        Ok(_) => {
                            let _ = open::that(&tmp_path);
                        }
                        Err(e) => {
                            self.status = format!("Failed to write temp file: {e}");
                        }
                    }
                }
                Ok(None) => {
                    self.status = "Attachment content not available offline.".to_string();
                }
                Err(e) => {
                    self.status = format!("Error loading attachment: {e}");
                }
            }
        }
    }
}

fn merge_folder_lists(target: &mut Vec<MailFolder>, remote: Vec<MailFolder>) {
    let mut by_path = std::collections::BTreeMap::new();
    for folder in target.drain(..) {
        by_path.insert(folder.path.clone(), folder);
    }
    for folder in remote {
        by_path.insert(folder.path.clone(), folder);
    }
    *target = by_path.into_values().collect();
}

fn ag_chat_bubble_text(message: &MailMessage) -> String {
    let text = message.body_text.as_deref().unwrap_or(&message.preview);
    
    // Attempt to strip out quoted replies and signatures heuristically.
    let lines = text.lines();
    let mut clean_lines = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('>') || trimmed.starts_with("On ") && trimmed.ends_with("wrote:") {
            break;
        }
        if trimmed == "-- " || trimmed == "--" || trimmed.starts_with("Sent from my") {
            break;
        }
        clean_lines.push(line);
    }
    
    let clean_text = clean_lines.join("\n").trim().to_string();
    if clean_text.is_empty() {
        text.trim().to_string()
    } else {
        clean_text
    }
}

fn set_secret_guarded(secrets: &SecretStore, key: SecretKey, value: &str) -> Result<(), String> {
    validate_secret_key(&key, value)?;
    secrets.set(&key, value).map_err(|err| err.to_string())
}

fn validate_secret_key(key: &SecretKey, value: &str) -> Result<(), String> {
    if key.namespace.trim().is_empty() || key.id.trim().is_empty() {
        return Err("secret namespace and id are required".to_string());
    }
    if value.trim().is_empty() {
        return Err("secret value cannot be empty".to_string());
    }
    if value.len() > 16_384 {
        return Err("secret value is too long".to_string());
    }
    if key.id.len() > 128
        || !key.id.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == ':'
        })
    {
        return Err("secret id contains invalid characters".to_string());
    }

    match key.namespace.as_str() {
        "account_password" | "oauth_refresh_token" | "oauth_access_token" => Ok(()),
        _ => Err("namespace is not allowed".to_string()),
    }
}

fn ai_runtime_from_config(config: &AppConfig) -> AiRuntimeConfig {
    let mut cloud = BTreeMap::new();

    for provider in [
        CloudAiProvider::OpenAi,
        CloudAiProvider::Anthropic,
        CloudAiProvider::Gemini,
        CloudAiProvider::Mistral,
    ] {
        let key = format!("{provider:?}").to_ascii_lowercase();
        if let Some(provider_cfg) = config.ai.cloud.providers.get(&key) {
            cloud.insert(
                provider,
                CloudProviderRuntime {
                    enabled: provider_cfg.enabled,
                    model: provider_cfg.model.clone(),
                    endpoint: provider_cfg.api_base.clone(),
                },
            );
        }
    }

    let cloud_feature_opt_in = config
        .ai
        .cloud
        .feature_opt_in
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    AiRuntimeConfig {
        local: LocalRuntime {
            enabled: config.ai.local.enabled,
            llama_cpp_binary: config.ai.local.llama_cpp_binary.clone(),
            model_path: config.ai.local.model_path.clone(),
            max_tokens: config.ai.local.context_tokens,
            temperature: 0.2,
        },
        cloud_enabled: config.ai.cloud.enabled,
        cloud_feature_opt_in,
        cloud,
    }
}

fn parse_domain_settings<T>(
    raw: &serde_json::Value,
    domain_key: &str,
) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned,
{
    if let Some(nested) = raw.get(domain_key) {
        serde_json::from_value(nested.clone())
    } else {
        serde_json::from_value(raw.clone())
    }
}

fn hydrate_email_secrets(account_id: Uuid, secrets: &SecretStore, settings: &mut ProtocolSettings) {
    if settings.password.is_none() {
        settings.password = secrets
            .get(&SecretKey {
                namespace: "account_password".to_string(),
                id: account_id.to_string(),
            })
            .ok()
            .flatten();
    }

    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .ok()
            .flatten();
    }
}

fn hydrate_calendar_secrets(
    account_id: Uuid,
    secrets: &SecretStore,
    settings: &mut CalendarSettings,
) {
    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .ok()
            .flatten();
    }
}

fn hydrate_task_secrets(account_id: Uuid, secrets: &SecretStore, settings: &mut TaskSettings) {
    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .ok()
            .flatten();
    }
}

fn oauth_profile_for_provider(
    provider: Provider,
    client_id: &str,
    redirect_url: &str,
) -> Result<cove_core::OAuthProfile, String> {
    let (auth_url, token_url, scopes) = match provider {
        Provider::Gmail => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://oauth2.googleapis.com/token",
            vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
                "https://mail.google.com/".to_string(),
                "https://www.googleapis.com/auth/calendar".to_string(),
                "https://www.googleapis.com/auth/tasks".to_string(),
            ],
        ),
        _ => return Err("Only Gmail OAuth is enabled in native shell".to_string()),
    };

    Ok(cove_core::OAuthProfile {
        client_id: client_id.to_string(),
        auth_url: auth_url
            .parse::<url::Url>()
            .map_err(|err| err.to_string())?,
        token_url: token_url
            .parse::<url::Url>()
            .map_err(|err| err.to_string())?,
        redirect_url: redirect_url
            .parse::<url::Url>()
            .map_err(|err| err.to_string())?,
        scopes,
    })
}

/// Extract an HTTP(S) unsubscribe URL from a `List-Unsubscribe` header value.
/// The header typically contains one or more URIs in angle brackets, e.g.
/// `<https://example.com/unsub>, <mailto:unsub@example.com>`.
/// We prefer the first `https://` or `http://` URL found.
fn extract_unsubscribe_url(header: &str) -> Option<String> {
    for part in header.split(',') {
        let trimmed = part.trim().trim_start_matches('<').trim_end_matches('>');
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return Some(trimmed.to_string());
        }
    }
    None
}
