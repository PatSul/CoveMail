mod export;

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
use chrono::{Duration, Utc};
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use uuid::Uuid;

fn main() -> anyhow::Result<()> {
    let options = eframe::NativeOptions::default();
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
        let Some(account_id) = self.selected_account else {
            return;
        };
        let Some(thread_id) = self.selected_thread.clone() else {
            return;
        };

        match self
            .runtime
            .block_on(self.storage.list_thread_messages(account_id, &thread_id))
        {
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
                self.status = format!(
                    "Sync failed: email={:?}, calendar={:?}, tasks={:?}",
                    mail.err(),
                    calendar.err(),
                    tasks.err()
                );
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
        match self
            .runtime
            .block_on(self.storage.search_mail(self.mail_query.trim(), 100))
        {
            Ok(result) => {
                self.selected_thread = None;
                self.selected_message = result.items.last().map(|message| message.id);
                self.thread_messages = result.items;
                self.status = format!("Search returned {} message(s)", self.thread_messages.len());
            }
            Err(err) => self.status = format!("search failed: {err}"),
        }
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

        match self
            .runtime
            .block_on(self.email.send(&account, &settings, &outgoing))
        {
            Ok(()) => {
                self.status = "Draft sent".to_string();
                self.compose_subject.clear();
                self.compose_body.clear();
                self.attachment_paths.clear();
            }
            Err(err) => self.status = format!("send failed: {err}"),
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
                let workflow = OAuthWorkflow::new(profile);
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

        let workflow = OAuthWorkflow::new(profile);
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
                                        });
                                });
                            });
                            
                            if provider_changed {
                                // Default redirect URL for desktop clients
                                self.oauth.redirect_url = "http://127.0.0.1:8765/oauth/callback".to_string();
                            }
                            
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

        egui::TopBottomPanel::top("top").frame(egui::Frame::default().fill(ctx.style().visuals.panel_fill).inner_margin(8.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (view, label) in [
                    (View::Inbox, "Inbox"),
                    (View::Chat, "Chat"),
                    (View::Calendar, "Calendar"),
                    (View::Tasks, "Tasks"),
                    (View::Ai, "AI"),
                    (View::Security, "Security"),
                ] {
                    if ui.selectable_label(self.view == view, label).clicked() {
                        self.view = view;
                    }
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
                        ui.heading(egui::RichText::new("Message").strong());
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(available_height - 20.0)
                            .show(ui, |ui| {
                                let mut next_message = None;
                                for message in &self.thread_messages {
                                    let selected = self.selected_message == Some(message.id);
                                    let mut frame = egui::Frame::window(&ctx.style())
                                        .inner_margin(16.0)
                                        .corner_radius(8.0);
                                    if selected {
                                        frame = frame.stroke(egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
                                    }
                                    
                                    ui.add_space(8.0);
                                    let response = frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(sender_for_message(message)).strong().size(15.0));
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                ui.label(egui::RichText::new(message.received_at.to_string()).size(12.0));
                                            });
                                        });
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(&message.subject).strong().size(16.0));
                                        ui.add_space(8.0);
                                        
                                        if selected {
                                            if !message.attachments.is_empty() {
                                                ui.label(egui::RichText::new("Attachments:").strong());
                                                for attachment in &message.attachments {
                                                    ui.label(format!("- {} ({} bytes)", attachment.file_name, attachment.size));
                                                }
                                                ui.add_space(8.0);
                                            }
                                            let body = message.body_text.as_deref().unwrap_or(&message.preview);
                                            ui.label(egui::RichText::new(body).size(14.0).line_height(Some(20.0)));
                                        } else {
                                            ui.label(egui::RichText::new(&message.preview).size(13.0));
                                        }
                                    }).response;
                                    
                                    if response.interact(egui::Sense::click()).clicked() {
                                        next_message = Some(message.id);
                                    }
                                }
                                if let Some(message_id) = next_message {
                                    self.selected_message = Some(message_id);
                                }
                            });
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
                            ui.text_edit_singleline(&mut self.compose_to);
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
                            let send_btn = ui.button(egui::RichText::new("Send Draft").strong().size(16.0).color(egui::Color32::WHITE));
                            if send_btn.clicked() {
                                self.send_compose();
                                close_window = true;
                            }
                        });
                }
                
                if close_window {
                    show_compose = false;
                }
                self.show_compose_window = show_compose;
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
                ui.label("Calendar sync uses Google/CalDAV backends from Rust services.");
            }
            View::Tasks => {
                ui.heading("Tasks");
                if let Some(account) = self.account() {
                    match self.runtime.block_on(self.storage.list_tasks(account.id)) {
                        Ok(tasks) => {
                            for task in tasks {
                                ui.label(format!("{} [{:?}]", task.title, task.priority));
                            }
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
            View::SetupWizard => {
                ui.heading("Setup Wizard");
                ui.label("Setup your accounts here.");
            }
        });
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

fn sender_for_message(message: &MailMessage) -> String {
    message
        .from
        .first()
        .map(|from| from.name.clone().unwrap_or_else(|| from.address.clone()))
        .unwrap_or_else(|| "Unknown sender".to_string())
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
