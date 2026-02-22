use crate::state::{AppState, PendingOAuthSession};
use cove_calendar::CalendarSettings;
use cove_config::AppConfig;
use cove_core::{
    Account, AccountProtocol, AiMode, CloudAiProvider, DataProvenance, OAuthProfile, Provider,
    SearchResult, SyncDomain, SyncJob, SyncStatus,
};
use cove_email::{OutgoingMail, ProtocolSettings};
use cove_security::{OAuthWorkflow, SecretKey, SecretStore};
use cove_storage::Storage;
use cove_tasks::NaturalTaskInput;
use chrono::{Duration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{collections::{HashMap, BTreeMap}, sync::Arc};
use tauri::{Emitter, State};
use tauri_plugin_notification::NotificationExt;
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct BootstrapResponse {
    pub config: AppConfig,
    pub accounts: Vec<Account>,
    pub pending_sync_jobs: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncRunSummary {
    pub completed_jobs: usize,
    pub failed_jobs: usize,
    pub retried_jobs: usize,
    pub email_messages_synced: usize,
    pub calendar_events_synced: usize,
    pub tasks_synced: usize,
}

impl SyncRunSummary {
    fn merge(&mut self, other: SyncRunSummary) {
        self.completed_jobs += other.completed_jobs;
        self.failed_jobs += other.failed_jobs;
        self.retried_jobs += other.retried_jobs;
        self.email_messages_synced += other.email_messages_synced;
        self.calendar_events_synced += other.calendar_events_synced;
        self.tasks_synced += other.tasks_synced;
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveAccountPayload {
    pub account: Account,
    pub protocol_settings: Value,
    pub password: Option<String>,
    pub oauth_refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QueueSyncPayload {
    pub account_id: Uuid,
    pub domain: SyncDomain,
    pub payload: Option<Value>,
    pub run_after_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchPayload {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ListMailPayload {
    pub account_id: Uuid,
    pub folder: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListThreadsPayload {
    pub account_id: Uuid,
    pub folder: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ThreadMessagesPayload {
    pub account_id: Uuid,
    pub thread_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ListFoldersPayload {
    pub account_id: Uuid,
    pub refresh_remote: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NaturalTaskPayload {
    pub account_id: Uuid,
    pub list_id: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportIcsPayload {
    pub account_id: Uuid,
    pub calendar_id: String,
    pub ics_payload: String,
}

#[derive(Debug, Deserialize)]
pub struct ExportIcsPayload {
    pub account_id: Uuid,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct AiPromptPayload {
    pub subject: String,
    pub body: String,
    pub mode: AiMode,
    pub cloud_provider: Option<CloudAiProvider>,
}

#[derive(Debug, Deserialize)]
pub struct AiActionPayload {
    pub body: String,
    pub mode: AiMode,
    pub cloud_provider: Option<CloudAiProvider>,
}

#[derive(Debug, Deserialize)]
pub struct AiActionToTasksPayload {
    pub account_id: Uuid,
    pub list_id: String,
    pub body: String,
    pub mode: AiMode,
    pub cloud_provider: Option<CloudAiProvider>,
}

#[derive(Debug, Serialize)]
pub struct AiResult {
    pub output: String,
    pub provenance: DataProvenance,
}

#[derive(Debug, Serialize)]
pub struct AiTaskExtractionResult {
    pub created: Vec<cove_core::ReminderTask>,
    pub provenance: DataProvenance,
}

#[derive(Debug, Deserialize)]
pub struct SetSecretPayload {
    pub namespace: String,
    pub id: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct SendMailPayload {
    pub account_id: Uuid,
    pub outgoing: OutgoingMail,
}

#[derive(Debug, Deserialize)]
pub struct BeginOAuthPayload {
    pub provider: Provider,
    pub email_address: String,
    pub display_name: Option<String>,
    pub client_id: String,
    pub redirect_url: String,
}

#[derive(Debug, Serialize)]
pub struct BeginOAuthResponse {
    pub session_id: Uuid,
    pub authorization_url: String,
}

#[derive(Debug, Deserialize)]
pub struct CompleteOAuthPayload {
    pub session_id: Uuid,
    pub csrf_state: String,
    pub code: String,
    pub calendar_id: Option<String>,
    pub task_list_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CompleteOAuthResponse {
    pub account: Account,
}

#[derive(Debug, Deserialize)]
pub struct ValidateLocalAiRuntimePayload {
    pub llama_cpp_binary: Option<String>,
    pub model_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidateLocalAiRuntimeResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}
#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapResponse, String> {
    let config = state.config().await;
    let accounts = state
        .storage
        .list_accounts()
        .await
        .map_err(to_error_string)?;
    let pending_sync_jobs = state
        .storage
        .pending_sync_jobs_count()
        .await
        .map_err(to_error_string)?;

    Ok(BootstrapResponse {
        config,
        accounts,
        pending_sync_jobs,
    })
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config().await)
}

#[tauri::command]
pub async fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<(), String> {
    state.set_config(config).await.map_err(to_error_string)
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    state.storage.list_accounts().await.map_err(to_error_string)
}

#[tauri::command]
pub async fn save_account(
    state: State<'_, AppState>,
    payload: SaveAccountPayload,
) -> Result<(), String> {
    state
        .storage
        .upsert_account(&payload.account)
        .await
        .map_err(to_error_string)?;

    let mut settings = payload.protocol_settings;
    extract_protocol_secrets(payload.account.id, &mut settings, &state.secrets)?;

    state
        .storage
        .upsert_account_protocol_settings(payload.account.id, &settings)
        .await
        .map_err(to_error_string)?;

    if let Some(password) = payload.password {
        set_secret_guarded(
            &state.secrets,
            SecretKey {
                namespace: "account_password".to_string(),
                id: payload.account.id.to_string(),
            },
            &password,
        )?;
    }

    if let Some(refresh_token) = payload.oauth_refresh_token {
        set_secret_guarded(
            &state.secrets,
            SecretKey {
                namespace: "oauth_refresh_token".to_string(),
                id: payload.account.id.to_string(),
            },
            &refresh_token,
        )?;
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_account(state: State<'_, AppState>, account_id: Uuid) -> Result<(), String> {
    state
        .storage
        .delete_account(account_id)
        .await
        .map_err(to_error_string)?;

    let secrets = [
        "account_password",
        "oauth_refresh_token",
        "oauth_access_token",
    ];
    for namespace in secrets {
        state
            .secrets
            .delete(&SecretKey {
                namespace: namespace.to_string(),
                id: account_id.to_string(),
            })
            .map_err(to_error_string)?;
    }

    Ok(())
}

#[tauri::command]
pub fn set_secret(state: State<'_, AppState>, payload: SetSecretPayload) -> Result<(), String> {
    let key = SecretKey {
        namespace: payload.namespace,
        id: payload.id,
    };

    set_secret_guarded(&state.secrets, key, &payload.value)
}

#[tauri::command]
pub async fn begin_oauth_pkce(
    state: State<'_, AppState>,
    payload: BeginOAuthPayload,
) -> Result<BeginOAuthResponse, String> {
    if payload.client_id.trim().is_empty() {
        return Err("client_id is required".to_string());
    }

    let profile = oauth_profile_for_provider(
        payload.provider.clone(),
        payload.client_id.trim(),
        payload.redirect_url.trim(),
    )?;

    let workflow = OAuthWorkflow::new(profile.clone()).map_err(to_error_string)?;
    let session = workflow.begin_pkce_session().map_err(to_error_string)?;

    let display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| payload.email_address.clone());

    let session_id = Uuid::new_v4();
    {
        let mut guard = state.oauth_sessions.write().await;
        guard.insert(
            session_id,
            PendingOAuthSession {
                provider: payload.provider,
                email_address: payload.email_address,
                display_name,
                oauth_profile: profile,
                csrf_state: session.csrf_state.clone(),
                pkce_verifier: session.pkce_verifier,
                created_at: Utc::now(),
            },
        );
    }

    Ok(BeginOAuthResponse {
        session_id,
        authorization_url: session.authorization_url,
    })
}
#[tauri::command]
pub async fn complete_oauth_pkce(
    state: State<'_, AppState>,
    payload: CompleteOAuthPayload,
) -> Result<CompleteOAuthResponse, String> {
    let session = {
        let guard = state.oauth_sessions.read().await;
        guard
            .get(&payload.session_id)
            .cloned()
            .ok_or_else(|| "OAuth session not found".to_string())?
    };

    if Utc::now() - session.created_at > Duration::minutes(15) {
        return Err("OAuth session expired; start again".to_string());
    }

    if payload.csrf_state != session.csrf_state {
        return Err("OAuth state mismatch".to_string());
    }

    let workflow = OAuthWorkflow::new(session.oauth_profile.clone()).map_err(to_error_string)?;
    let token = workflow
        .exchange_code(payload.code.trim(), &session.pkce_verifier)
        .await
        .map_err(to_error_string)?;

    let now = Utc::now();
    let account = Account {
        id: Uuid::new_v4(),
        provider: session.provider.clone(),
        protocols: protocols_for_provider(&session.provider),
        display_name: session.display_name,
        email_address: session.email_address.clone(),
        oauth_profile: Some(session.oauth_profile.clone()),
        created_at: now,
        updated_at: now,
    };

    state
        .storage
        .upsert_account(&account)
        .await
        .map_err(to_error_string)?;

    let settings = default_settings_for_provider(
        &session.provider,
        &session.email_address,
        payload.calendar_id,
        payload.task_list_id,
    )?;

    state
        .storage
        .upsert_account_protocol_settings(account.id, &settings)
        .await
        .map_err(to_error_string)?;

    set_secret_guarded(
        &state.secrets,
        SecretKey {
            namespace: "oauth_access_token".to_string(),
            id: account.id.to_string(),
        },
        &token.access_token,
    )?;

    if let Some(refresh_token) = token.refresh_token {
        set_secret_guarded(
            &state.secrets,
            SecretKey {
                namespace: "oauth_refresh_token".to_string(),
                id: account.id.to_string(),
            },
            &refresh_token,
        )?;
    }

    {
        let mut guard = state.oauth_sessions.write().await;
        guard.remove(&payload.session_id);
    }

    Ok(CompleteOAuthResponse { account })
}

#[tauri::command]
pub async fn queue_sync_job(
    state: State<'_, AppState>,
    payload: QueueSyncPayload,
) -> Result<SyncJob, String> {
    let now = Utc::now();
    let run_after_secs = payload.run_after_secs.unwrap_or(0).max(0);
    let run_after = now + Duration::seconds(run_after_secs);
    let job = SyncJob {
        id: Uuid::new_v4(),
        account_id: payload.account_id,
        domain: payload.domain,
        status: SyncStatus::Queued,
        payload_json: payload.payload.unwrap_or_else(|| serde_json::json!({})),
        attempt_count: 0,
        max_attempts: 5,
        run_after,
        last_error: None,
        created_at: now,
        updated_at: now,
    };

    state
        .storage
        .enqueue_sync_job(&job)
        .await
        .map_err(to_error_string)?;

    Ok(job)
}
#[tauri::command]
pub async fn run_sync_queue(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<SyncRunSummary, String> {
    let max_parallel_jobs = state.config().await.sync.max_parallel_jobs.clamp(1, 40);
    let max_jobs_per_account = 2_usize;
    let max_jobs_per_account_domain = 1_usize;
    let accounts = state
        .storage
        .list_accounts()
        .await
        .map_err(to_error_string)?;
    let jobs = state
        .storage
        .fetch_due_sync_jobs(40)
        .await
        .map_err(to_error_string)?;

    let mut summary = SyncRunSummary::default();
    if jobs.is_empty() {
        return Ok(summary);
    }

    let context = SyncExecutionContext {
        storage: state.storage.clone(),
        email: state.email.clone(),
        calendar: state.calendar.clone(),
        tasks: state.tasks.clone(),
        secrets: state.secrets.clone(),
        accounts: Arc::new(
            accounts
                .into_iter()
                .map(|account| (account.id, account))
                .collect::<HashMap<Uuid, Account>>(),
        ),
    };

    let mut jobs = jobs;
    let mut workers = JoinSet::new();
    let mut active_by_account: HashMap<Uuid, usize> = HashMap::new();
    let mut active_by_account_domain: HashMap<(Uuid, u8), usize> = HashMap::new();

    for _ in 0..max_parallel_jobs {
        let Some(job) = take_next_eligible_job(
            &mut jobs,
            &active_by_account,
            &active_by_account_domain,
            max_jobs_per_account,
            max_jobs_per_account_domain,
        ) else {
            break;
        };

        let account_id = job.account_id;
        let domain_slot = sync_domain_slot(&job.domain);
        *active_by_account.entry(account_id).or_insert(0) += 1;
        *active_by_account_domain
            .entry((account_id, domain_slot))
            .or_insert(0) += 1;

        let context_for_worker = context.clone();
        workers.spawn(async move {
            let worker_summary = run_sync_job(context_for_worker, job).await?;
            Ok::<(SyncRunSummary, Uuid, u8), String>((worker_summary, account_id, domain_slot))
        });
    }

    while let Some(joined) = workers.join_next().await {
        let (worker_summary, account_id, domain_slot) = joined
            .map_err(|error| format!("sync worker join failed: {error}"))??;

        if let Some(active) = active_by_account.get_mut(&account_id) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                active_by_account.remove(&account_id);
            }
        }
        if let Some(active) = active_by_account_domain.get_mut(&(account_id, domain_slot)) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                active_by_account_domain.remove(&(account_id, domain_slot));
            }
        }

        summary.merge(worker_summary);

        if let Some(job) = take_next_eligible_job(
            &mut jobs,
            &active_by_account,
            &active_by_account_domain,
            max_jobs_per_account,
            max_jobs_per_account_domain,
        ) {
            let account_id = job.account_id;
            let domain_slot = sync_domain_slot(&job.domain);
            *active_by_account.entry(account_id).or_insert(0) += 1;
            *active_by_account_domain
                .entry((account_id, domain_slot))
                .or_insert(0) += 1;

            let context_for_worker = context.clone();
            workers.spawn(async move {
                let worker_summary = run_sync_job(context_for_worker, job).await?;
                Ok::<(SyncRunSummary, Uuid, u8), String>((worker_summary, account_id, domain_slot))
            });
        }
    }

    if summary.completed_jobs > 0 || summary.failed_jobs > 0 {
        let _ = app_handle.emit("sync://summary", &summary);
        send_sync_notification(&app_handle, &summary);
    }

    Ok(summary)
}

fn take_next_eligible_job(
    jobs: &mut Vec<SyncJob>,
    active_by_account: &HashMap<Uuid, usize>,
    active_by_account_domain: &HashMap<(Uuid, u8), usize>,
    max_jobs_per_account: usize,
    max_jobs_per_account_domain: usize,
) -> Option<SyncJob> {
    let index = jobs.iter().position(|job| {
        let account_active = active_by_account.get(&job.account_id).copied().unwrap_or(0);
        if account_active >= max_jobs_per_account {
            return false;
        }

        let domain_slot = sync_domain_slot(&job.domain);
        let domain_active = active_by_account_domain
            .get(&(job.account_id, domain_slot))
            .copied()
            .unwrap_or(0);
        domain_active < max_jobs_per_account_domain
    })?;

    Some(jobs.remove(index))
}

fn sync_domain_slot(domain: &SyncDomain) -> u8 {
    match domain {
        SyncDomain::Email => 0,
        SyncDomain::Calendar => 1,
        SyncDomain::Tasks => 2,
    }
}
#[tauri::command]
pub async fn search_mail(
    state: State<'_, AppState>,
    payload: SearchPayload,
) -> Result<SearchResult<cove_core::MailMessage>, String> {
    state
        .storage
        .search_mail(&payload.query, payload.limit.unwrap_or(50))
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn list_mail(
    state: State<'_, AppState>,
    payload: ListMailPayload,
) -> Result<SearchResult<cove_core::MailMessage>, String> {
    state
        .storage
        .list_mail_messages(
            payload.account_id,
            payload.folder.as_deref(),
            payload.limit.unwrap_or(50),
            payload.offset.unwrap_or(0),
        )
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn list_mail_folders(
    state: State<'_, AppState>,
    payload: ListFoldersPayload,
) -> Result<Vec<cove_core::MailFolder>, String> {
    let account = state
        .storage
        .list_accounts()
        .await
        .map_err(to_error_string)?
        .into_iter()
        .find(|account| account.id == payload.account_id)
        .ok_or_else(|| "account not found".to_string())?;

    let mut cached = state
        .storage
        .list_mail_folders(payload.account_id)
        .await
        .map_err(to_error_string)?;

    if payload.refresh_remote.unwrap_or(false) {
        if let Some(raw) = state
            .storage
            .account_protocol_settings(account.id)
            .await
            .map_err(to_error_string)?
        {
            let mut settings: ProtocolSettings =
                parse_domain_settings(&raw, "email").map_err(to_error_string)?;
            hydrate_email_secrets(account.id, &state.secrets, &mut settings)?;
            let remote = state
                .email
                .sync_folders(&account, &settings)
                .await
                .map_err(to_error_string)?;
            merge_folders(&mut cached, remote);
        }
    }

    Ok(cached)
}

#[tauri::command]
pub async fn list_mail_threads(
    state: State<'_, AppState>,
    payload: ListThreadsPayload,
) -> Result<Vec<cove_core::MailThreadSummary>, String> {
    state
        .email
        .list_threads(
            payload.account_id,
            payload.folder.as_deref(),
            payload.limit.unwrap_or(120),
            payload.offset.unwrap_or(0),
        )
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn list_thread_messages(
    state: State<'_, AppState>,
    payload: ThreadMessagesPayload,
) -> Result<Vec<cove_core::MailMessage>, String> {
    state
        .storage
        .list_thread_messages(payload.account_id, &payload.thread_id)
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn get_mail_message(
    state: State<'_, AppState>,
    message_id: Uuid,
) -> Result<Option<cove_core::MailMessage>, String> {
    state
        .storage
        .get_mail_message(message_id)
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn send_mail(state: State<'_, AppState>, payload: SendMailPayload) -> Result<(), String> {
    let account = state
        .storage
        .list_accounts()
        .await
        .map_err(to_error_string)?
        .into_iter()
        .find(|account| account.id == payload.account_id)
        .ok_or_else(|| "account not found".to_string())?;

    let settings = state
        .storage
        .account_protocol_settings(account.id)
        .await
        .map_err(to_error_string)?
        .ok_or_else(|| "protocol settings missing".to_string())?;

    let mut settings: ProtocolSettings =
        parse_domain_settings(&settings, "email").map_err(to_error_string)?;
    hydrate_email_secrets(account.id, &state.secrets, &mut settings)?;

    state
        .email
        .send(&account, &settings, &payload.outgoing)
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn list_tasks(
    state: State<'_, AppState>,
    account_id: Uuid,
) -> Result<Vec<cove_core::ReminderTask>, String> {
    state
        .storage
        .list_tasks(account_id)
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn create_task_from_text(
    state: State<'_, AppState>,
    payload: NaturalTaskPayload,
) -> Result<cove_core::ReminderTask, String> {
    state
        .tasks
        .create_from_natural_language(NaturalTaskInput {
            text: payload.text,
            account_id: payload.account_id,
            list_id: payload.list_id,
        })
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn import_calendar_ics(
    state: State<'_, AppState>,
    payload: ImportIcsPayload,
) -> Result<Vec<cove_core::CalendarEvent>, String> {
    state
        .calendar
        .import_ics(
            payload.account_id,
            &payload.calendar_id,
            &payload.ics_payload,
        )
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn export_calendar_ics(
    state: State<'_, AppState>,
    payload: ExportIcsPayload,
) -> Result<String, String> {
    let from = chrono::DateTime::parse_from_rfc3339(&payload.from)
        .map_err(to_error_string)?
        .with_timezone(&Utc);
    let to = chrono::DateTime::parse_from_rfc3339(&payload.to)
        .map_err(to_error_string)?
        .with_timezone(&Utc);

    let events = state
        .storage
        .list_calendar_events(payload.account_id, from, to)
        .await
        .map_err(to_error_string)?;

    Ok(state.calendar.export_ics(&events))
}
#[tauri::command]
pub async fn ai_summarize_email(
    state: State<'_, AppState>,
    payload: AiPromptPayload,
) -> Result<AiResult, String> {
    let ai = state.ai.read().await;
    let (response, provenance) = ai
        .summarize_email(
            &payload.subject,
            &payload.body,
            payload.mode,
            payload.cloud_provider,
        )
        .await
        .map_err(to_error_string)?;

    Ok(AiResult {
        output: response.output,
        provenance,
    })
}

#[tauri::command]
pub async fn ai_suggest_reply(
    state: State<'_, AppState>,
    payload: AiPromptPayload,
) -> Result<AiResult, String> {
    let ai = state.ai.read().await;
    let (response, provenance) = ai
        .suggest_reply(
            &payload.subject,
            &payload.body,
            payload.mode,
            payload.cloud_provider,
        )
        .await
        .map_err(to_error_string)?;

    Ok(AiResult {
        output: response.output,
        provenance,
    })
}

#[tauri::command]
pub async fn ai_extract_action_items(
    state: State<'_, AppState>,
    payload: AiActionPayload,
) -> Result<(Vec<String>, DataProvenance), String> {
    let ai = state.ai.read().await;
    ai.extract_action_items(&payload.body, payload.mode, payload.cloud_provider)
        .await
        .map_err(to_error_string)
}

#[tauri::command]
pub async fn ai_create_tasks_from_email(
    state: State<'_, AppState>,
    payload: AiActionToTasksPayload,
) -> Result<AiTaskExtractionResult, String> {
    let ai = state.ai.read().await;
    let (items, provenance) = ai
        .extract_action_items(&payload.body, payload.mode, payload.cloud_provider)
        .await
        .map_err(to_error_string)?;
    drop(ai);

    let mut created = Vec::new();
    for line in items {
        if line.trim().is_empty() {
            continue;
        }

        let task = state
            .tasks
            .create_from_natural_language(NaturalTaskInput {
                text: line,
                account_id: payload.account_id,
                list_id: payload.list_id.clone(),
            })
            .await
            .map_err(to_error_string)?;
        created.push(task);
    }

    Ok(AiTaskExtractionResult {
        created,
        provenance,
    })
}

#[tauri::command]
pub async fn set_ai_api_key(
    state: State<'_, AppState>,
    provider: CloudAiProvider,
    api_key: String,
) -> Result<(), String> {
    let id = format!("{provider:?}").to_ascii_lowercase();
    set_secret_guarded(
        &state.secrets,
        SecretKey {
            namespace: "ai_api_key".to_string(),
            id,
        },
        &api_key,
    )
}

#[tauri::command]
pub fn validate_local_ai_runtime(
    payload: ValidateLocalAiRuntimePayload,
) -> ValidateLocalAiRuntimeResponse {
    let mut errors = Vec::new();

    match payload.llama_cpp_binary {
        Some(path) if path.trim().is_empty() => {
            errors.push("llama.cpp binary path is empty".to_string());
        }
        Some(path) => {
            let path = std::path::Path::new(path.trim());
            if !path.exists() {
                errors.push("llama.cpp binary path does not exist".to_string());
            } else if !path.is_file() {
                errors.push("llama.cpp binary path is not a file".to_string());
            }
        }
        None => errors.push("llama.cpp binary path is required".to_string()),
    }

    match payload.model_path {
        Some(path) if path.trim().is_empty() => {
            errors.push("GGUF model path is empty".to_string());
        }
        Some(path) => {
            let path = std::path::Path::new(path.trim());
            if !path.exists() {
                errors.push("GGUF model path does not exist".to_string());
            } else if !path.is_file() {
                errors.push("GGUF model path is not a file".to_string());
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| !ext.eq_ignore_ascii_case("gguf"))
                .unwrap_or(true)
            {
                errors.push("model file must use .gguf extension".to_string());
            }
        }
        None => errors.push("GGUF model path is required".to_string()),
    }

    ValidateLocalAiRuntimeResponse {
        valid: errors.is_empty(),
        errors,
    }
}

#[tauri::command]
pub async fn ai_fetch_available_models(
    state: State<'_, AppState>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let ai = state.ai.read().await;
    ai.fetch_available_models().await.map_err(to_error_string)
}

enum SyncDomainResult {
    Email(usize),
    Calendar(usize),
    Tasks(usize),
}

enum JobFailureDisposition {
    Retried,
    Failed,
}

#[derive(Clone)]
struct SyncExecutionContext {
    storage: Storage,
    email: cove_email::EmailService,
    calendar: cove_calendar::CalendarService,
    tasks: cove_tasks::TaskService,
    secrets: SecretStore,
    accounts: Arc<HashMap<Uuid, Account>>,
}

async fn run_sync_job(
    context: SyncExecutionContext,
    job: SyncJob,
) -> Result<SyncRunSummary, String> {
    let mut summary = SyncRunSummary::default();

    context
        .storage
        .update_sync_job_status(job.id, SyncStatus::Running, None, Some(job.attempt_count))
        .await
        .map_err(to_error_string)?;

    let account = match context.accounts.get(&job.account_id) {
        Some(account) => account.clone(),
        None => {
            match fail_or_retry_job(&context.storage, &job, "account not found".to_string())
                .await?
            {
                JobFailureDisposition::Retried => summary.retried_jobs += 1,
                JobFailureDisposition::Failed => summary.failed_jobs += 1,
            }
            return Ok(summary);
        }
    };

    let settings = match context
        .storage
        .account_protocol_settings(account.id)
        .await
        .map_err(to_error_string)?
    {
        Some(value) => value,
        None => {
            match fail_or_retry_job(
                &context.storage,
                &job,
                "protocol settings missing".to_string(),
            )
            .await?
            {
                JobFailureDisposition::Retried => summary.retried_jobs += 1,
                JobFailureDisposition::Failed => summary.failed_jobs += 1,
            }
            return Ok(summary);
        }
    };

    let result: Result<SyncDomainResult, String> = match job.domain {
        SyncDomain::Email => {
            let mut protocol: ProtocolSettings =
                parse_domain_settings(&settings, "email").map_err(to_error_string)?;
            hydrate_email_secrets(account.id, &context.secrets, &mut protocol)?;
            context
                .email
                .sync_recent_mail(&account, &protocol, "INBOX", 100)
                .await
                .map(SyncDomainResult::Email)
                .map_err(to_error_string)
        }
        SyncDomain::Calendar => {
            let mut calendar_settings: CalendarSettings =
                parse_domain_settings(&settings, "calendar").map_err(to_error_string)?;
            hydrate_calendar_secrets(account.id, &context.secrets, &mut calendar_settings)?;
            context
                .calendar
                .sync_range(
                    &account,
                    &calendar_settings,
                    Utc::now() - Duration::days(30),
                    Utc::now() + Duration::days(365),
                )
                .await
                .map(|events| SyncDomainResult::Calendar(events.len()))
                .map_err(to_error_string)
        }
        SyncDomain::Tasks => {
            let mut task_settings: cove_tasks::TaskSettings =
                parse_domain_settings(&settings, "tasks").map_err(to_error_string)?;
            hydrate_task_secrets(account.id, &context.secrets, &mut task_settings)?;
            context
                .tasks
                .sync_tasks(&account, &task_settings)
                .await
                .map(|tasks| SyncDomainResult::Tasks(tasks.len()))
                .map_err(to_error_string)
        }
    };

    match result {
        Ok(domain_result) => {
            context
                .storage
                .update_sync_job_status(job.id, SyncStatus::Completed, None, Some(job.attempt_count))
                .await
                .map_err(to_error_string)?;
            summary.completed_jobs += 1;
            match domain_result {
                SyncDomainResult::Email(count) => summary.email_messages_synced += count,
                SyncDomainResult::Calendar(count) => summary.calendar_events_synced += count,
                SyncDomainResult::Tasks(count) => summary.tasks_synced += count,
            }
        }
        Err(error) => match fail_or_retry_job(&context.storage, &job, error).await? {
            JobFailureDisposition::Retried => summary.retried_jobs += 1,
            JobFailureDisposition::Failed => summary.failed_jobs += 1,
        },
    }

    Ok(summary)
}

async fn fail_or_retry_job(
    storage: &Storage,
    job: &SyncJob,
    error: String,
) -> Result<JobFailureDisposition, String> {
    let next_attempt = job.attempt_count.saturating_add(1);
    if next_attempt >= job.max_attempts {
        storage
            .update_sync_job_status(job.id, SyncStatus::Failed, Some(error), Some(next_attempt))
            .await
            .map_err(to_error_string)?;
        return Ok(JobFailureDisposition::Failed);
    }

    let delay_secs = backoff_secs(next_attempt);
    let now = Utc::now();
    let retry = SyncJob {
        id: job.id,
        account_id: job.account_id,
        domain: job.domain.clone(),
        status: SyncStatus::Queued,
        payload_json: job.payload_json.clone(),
        attempt_count: next_attempt,
        max_attempts: job.max_attempts,
        run_after: now + Duration::seconds(delay_secs as i64),
        last_error: Some(error),
        created_at: job.created_at,
        updated_at: now,
    };

    storage
        .enqueue_sync_job(&retry)
        .await
        .map_err(to_error_string)?;

    Ok(JobFailureDisposition::Retried)
}

fn send_sync_notification(app_handle: &tauri::AppHandle, summary: &SyncRunSummary) {
    let title = if summary.failed_jobs > 0 {
        "Cove Mail sync needs attention"
    } else {
        "Cove Mail sync complete"
    };

    let mut parts = Vec::new();
    if summary.email_messages_synced > 0 {
        parts.push(format!("{} email items", summary.email_messages_synced));
    }
    if summary.calendar_events_synced > 0 {
        parts.push(format!(
            "{} calendar events",
            summary.calendar_events_synced
        ));
    }
    if summary.tasks_synced > 0 {
        parts.push(format!("{} tasks", summary.tasks_synced));
    }
    if summary.retried_jobs > 0 {
        parts.push(format!("{} retried jobs", summary.retried_jobs));
    }
    if summary.failed_jobs > 0 {
        parts.push(format!("{} failed jobs", summary.failed_jobs));
    }
    if parts.is_empty() {
        parts.push(format!("{} completed jobs", summary.completed_jobs));
    }

    let body = parts.join(" | ");
    let _ = app_handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

fn backoff_secs(attempt: u32) -> u64 {
    let base = 30_u64;
    let capped = attempt.min(8);
    base.saturating_mul(1_u64 << capped)
}

fn parse_domain_settings<T>(raw: &Value, domain_key: &str) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    if let Some(nested) = raw.get(domain_key) {
        serde_json::from_value(nested.clone())
    } else {
        serde_json::from_value(raw.clone())
    }
}

fn hydrate_email_secrets(
    account_id: Uuid,
    secrets: &SecretStore,
    settings: &mut ProtocolSettings,
) -> Result<(), String> {
    if settings.password.is_none() {
        settings.password = secrets
            .get(&SecretKey {
                namespace: "account_password".to_string(),
                id: account_id.to_string(),
            })
            .map_err(to_error_string)?;
    }

    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .map_err(to_error_string)?;
    }

    Ok(())
}

fn hydrate_calendar_secrets(
    account_id: Uuid,
    secrets: &SecretStore,
    settings: &mut CalendarSettings,
) -> Result<(), String> {
    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .map_err(to_error_string)?;
    }

    Ok(())
}

fn hydrate_task_secrets(
    account_id: Uuid,
    secrets: &SecretStore,
    settings: &mut cove_tasks::TaskSettings,
) -> Result<(), String> {
    if settings.access_token.is_none() {
        settings.access_token = secrets
            .get(&SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            })
            .map_err(to_error_string)?;
    }

    Ok(())
}

fn extract_protocol_secrets(
    account_id: Uuid,
    settings: &mut Value,
    secrets: &SecretStore,
) -> Result<(), String> {
    let mut password = None;
    let mut access_token = None;
    strip_secrets_from_value(settings, &mut password, &mut access_token);

    if let Some(password) = password {
        set_secret_guarded(
            secrets,
            SecretKey {
                namespace: "account_password".to_string(),
                id: account_id.to_string(),
            },
            &password,
        )?;
    }

    if let Some(token) = access_token {
        set_secret_guarded(
            secrets,
            SecretKey {
                namespace: "oauth_access_token".to_string(),
                id: account_id.to_string(),
            },
            &token,
        )?;
    }

    Ok(())
}

fn strip_secrets_from_value(
    value: &mut Value,
    password_out: &mut Option<String>,
    access_token_out: &mut Option<String>,
) {
    match value {
        Value::Object(map) => {
            if let Some(secret) = map.remove("password") {
                if let Some(secret) = secret.as_str() {
                    *password_out = Some(secret.to_string());
                }
            }
            if let Some(secret) = map.remove("access_token") {
                if let Some(secret) = secret.as_str() {
                    *access_token_out = Some(secret.to_string());
                }
            }

            for nested in map.values_mut() {
                strip_secrets_from_value(nested, password_out, access_token_out);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_secrets_from_value(item, password_out, access_token_out);
            }
        }
        _ => {}
    }
}
fn oauth_profile_for_provider(
    provider: Provider,
    client_id: &str,
    redirect_url: &str,
) -> Result<OAuthProfile, String> {
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
        Provider::Outlook | Provider::Exchange => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            vec![
                "offline_access".to_string(),
                "User.Read".to_string(),
                "Mail.ReadWrite".to_string(),
                "Mail.Send".to_string(),
                "Calendars.ReadWrite".to_string(),
                "Tasks.ReadWrite".to_string(),
            ],
        ),
        _ => return Err("OAuth flow is implemented for Gmail and Microsoft providers".to_string()),
    };

    Ok(OAuthProfile {
        client_id: client_id.to_string(),
        auth_url: auth_url.parse().map_err(to_error_string)?,
        token_url: token_url.parse().map_err(to_error_string)?,
        redirect_url: redirect_url.parse().map_err(to_error_string)?,
        scopes,
    })
}

fn protocols_for_provider(provider: &Provider) -> Vec<AccountProtocol> {
    match provider {
        Provider::Gmail => vec![
            AccountProtocol::ImapSmtp,
            AccountProtocol::GoogleCalendar,
            AccountProtocol::GoogleTasks,
        ],
        Provider::Outlook | Provider::Exchange => vec![
            AccountProtocol::ImapSmtp,
            AccountProtocol::MicrosoftGraph,
            AccountProtocol::MicrosoftTodo,
        ],
        _ => vec![AccountProtocol::ImapSmtp],
    }
}

fn default_settings_for_provider(
    provider: &Provider,
    email_address: &str,
    calendar_id: Option<String>,
    task_list_id: Option<String>,
) -> Result<Value, String> {
    match provider {
        Provider::Gmail => Ok(serde_json::json!({
            "email": {
                "imap_host": "imap.gmail.com",
                "imap_port": 993,
                "smtp_host": "smtp.gmail.com",
                "smtp_port": 465,
                "endpoint": null,
                "username": email_address,
                "password": null,
                "access_token": null
            },
            "calendar": {
                "endpoint": "https://www.googleapis.com/calendar/v3",
                "access_token": null,
                "calendar_id": calendar_id.unwrap_or_else(|| "primary".to_string())
            },
            "tasks": {
                "endpoint": "https://tasks.googleapis.com/tasks/v1",
                "access_token": null,
                "list_id": task_list_id.unwrap_or_else(|| "@default".to_string())
            }
        })),
        Provider::Outlook | Provider::Exchange => Ok(serde_json::json!({
            "email": {
                "imap_host": "outlook.office365.com",
                "imap_port": 993,
                "smtp_host": "smtp.office365.com",
                "smtp_port": 587,
                "endpoint": null,
                "username": email_address,
                "password": null,
                "access_token": null
            },
            "calendar": {
                "endpoint": "https://graph.microsoft.com/v1.0",
                "access_token": null,
                "calendar_id": calendar_id.unwrap_or_else(|| "primary".to_string())
            },
            "tasks": {
                "endpoint": "https://graph.microsoft.com/v1.0",
                "access_token": null,
                "list_id": task_list_id.unwrap_or_else(|| "Tasks".to_string())
            }
        })),
        _ => Err("Provider defaults not implemented".to_string()),
    }
}

fn merge_folders(target: &mut Vec<cove_core::MailFolder>, remote: Vec<cove_core::MailFolder>) {
    let mut by_path = std::collections::BTreeMap::new();

    for folder in target.drain(..) {
        by_path.insert(folder.path.clone(), folder);
    }
    for folder in remote {
        by_path.insert(folder.path.clone(), folder);
    }

    *target = by_path.into_values().collect();
}

fn set_secret_guarded(secrets: &SecretStore, key: SecretKey, value: &str) -> Result<(), String> {
    validate_secret_key(&key, value)?;
    secrets.set(&key, value).map_err(to_error_string)
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
        "account_password" | "oauth_refresh_token" | "oauth_access_token" => {}
        "ai_api_key" => {
            let allowed = [
                "openai",
                "anthropic",
                "gemini",
                "mistral",
                "groq",
                "grok",
                "openrouter",
            ];
            if !allowed.contains(&key.id.as_str()) {
                return Err("invalid AI provider id".to_string());
            }
        }
        "database" => {
            if key.id != "sqlcipher_key" {
                return Err("invalid database secret id".to_string());
            }
            if value.len() < 16 {
                return Err("sqlcipher key must be at least 16 characters".to_string());
            }
        }
        _ => return Err("namespace is not allowed".to_string()),
    }

    Ok(())
}

fn to_error_string<E>(error: E) -> String
where
    E: std::fmt::Display,
{
    error.to_string()
}
