use aether_ai::{AiRuntimeConfig, AiService, CloudProviderRuntime, LocalRuntime};
use aether_calendar::CalendarService;
use aether_config::{AppConfig, ConfigManager};
use aether_core::{CloudAiProvider, OAuthProfile, Provider, SyncDomain, SyncJob, SyncStatus};
use aether_email::{EmailService, ProtocolSettings};
use aether_security::{SecretKey, SecretStore};
use aether_storage::Storage;
use aether_tasks::TaskService;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub(crate) struct PendingOAuthSession {
    pub provider: Provider,
    pub email_address: String,
    pub display_name: String,
    pub oauth_profile: OAuthProfile,
    pub csrf_state: String,
    pub pkce_verifier: String,
    pub created_at: DateTime<Utc>,
}

pub struct AppState {
    pub(crate) config_manager: ConfigManager,
    pub(crate) config: RwLock<AppConfig>,
    pub(crate) storage: Storage,
    pub(crate) secrets: SecretStore,
    pub(crate) email: EmailService,
    pub(crate) calendar: CalendarService,
    pub(crate) tasks: TaskService,
    pub(crate) ai: RwLock<AiService>,
    pub(crate) oauth_sessions: RwLock<HashMap<Uuid, PendingOAuthSession>>,
}

impl AppState {
    pub async fn initialize() -> anyhow::Result<Self> {
        let config_manager = ConfigManager::new().context("initialize config manager")?;
        let config = config_manager.load().context("load app config")?;

        let secrets =
            SecretStore::new_with_legacy("io.aegisinbox.desktop", "io.aether.desktop");
        let db_key = secrets
            .get(&SecretKey {
                namespace: "database".to_string(),
                id: "sqlcipher_key".to_string(),
            })
            .context("load sqlcipher key from keychain")?;
        let db_key = validate_sqlcipher_config(&config, db_key)?;

        let db_path = config_manager.data_dir().join(&config.database.file_name);
        let search_path = config_manager.cache_dir().join("mail-index");

        let storage = Storage::connect(&db_path, &search_path, db_key.as_deref())
            .await
            .context("initialize sqlite storage")?;

        let email = EmailService::new(storage.clone());
        let calendar = CalendarService::new(storage.clone());
        let tasks = TaskService::new(storage.clone());

        let ai_config = ai_runtime_from_config(&config);
        let ai = AiService::new(ai_config, secrets.clone());

        Ok(Self {
            config_manager,
            config: RwLock::new(config),
            storage,
            secrets,
            email,
            calendar,
            tasks,
            ai: RwLock::new(ai),
            oauth_sessions: RwLock::new(HashMap::new()),
        })
    }

    pub async fn config(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn set_config(&self, next: AppConfig) -> anyhow::Result<()> {
        let db_key = self
            .secrets
            .get(&SecretKey {
                namespace: "database".to_string(),
                id: "sqlcipher_key".to_string(),
            })
            .context("load sqlcipher key from keychain")?;
        validate_sqlcipher_config(&next, db_key)?;

        self.config_manager.save(&next)?;
        {
            let mut guard = self.config.write().await;
            *guard = next.clone();
        }
        {
            let mut ai = self.ai.write().await;
            ai.update_config(ai_runtime_from_config(&next));
        }

        Ok(())
    }

    pub async fn schedule_sync_jobs(&self) -> anyhow::Result<usize> {
        let config = self.config().await;
        let accounts = self.storage.list_accounts().await?;
        let now = Utc::now();
        let mut scheduled = 0_usize;

        for account in &accounts {
            for domain in [SyncDomain::Email, SyncDomain::Calendar, SyncDomain::Tasks] {
                if self
                    .storage
                    .has_active_sync_job(account.id, domain.clone())
                    .await?
                {
                    continue;
                }

                let has_history = self
                    .storage
                    .has_sync_history(account.id, domain.clone())
                    .await?;
                let run_after = if has_history {
                    now + chrono::Duration::seconds(
                        interval_secs_for_domain(&config, &domain) as i64
                    )
                } else {
                    now
                };

                let job = SyncJob {
                    id: Uuid::new_v4(),
                    account_id: account.id,
                    domain: domain.clone(),
                    status: SyncStatus::Queued,
                    payload_json: serde_json::json!({}),
                    attempt_count: 0,
                    max_attempts: 5,
                    run_after,
                    last_error: None,
                    created_at: now,
                    updated_at: now,
                };

                self.storage.enqueue_sync_job(&job).await?;
                scheduled += 1;
            }
        }

        Ok(scheduled)
    }

    pub async fn prime_idle_listeners(&self) -> anyhow::Result<usize> {
        let accounts = self.storage.list_accounts().await?;
        let mut started = 0_usize;

        for account in accounts {
            let Some(raw) = self.storage.account_protocol_settings(account.id).await? else {
                continue;
            };

            let mut settings: ProtocolSettings = match parse_domain_settings(&raw, "email") {
                Ok(settings) => settings,
                Err(_) => continue,
            };
            hydrate_email_secrets(account.id, &self.secrets, &mut settings)?;

            if self
                .email
                .start_idle(&account, &settings, "INBOX")
                .await
                .is_ok()
            {
                started += 1;
            }
        }

        Ok(started)
    }
}

fn ai_runtime_from_config(config: &AppConfig) -> AiRuntimeConfig {
    let mut cloud = BTreeMap::new();

    for provider in [
        CloudAiProvider::OpenAi,
        CloudAiProvider::Anthropic,
        CloudAiProvider::Gemini,
        CloudAiProvider::Mistral,
        CloudAiProvider::Groq,
        CloudAiProvider::Grok,
        CloudAiProvider::OpenRouter,
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

fn interval_secs_for_domain(config: &AppConfig, domain: &SyncDomain) -> u64 {
    match domain {
        SyncDomain::Email => config.sync.email_poll_interval_secs,
        SyncDomain::Calendar => config.sync.calendar_poll_interval_secs,
        SyncDomain::Tasks => config.sync.task_poll_interval_secs,
    }
}

fn parse_domain_settings<T>(
    raw: &serde_json::Value,
    domain_key: &str,
) -> Result<T, serde_json::Error>
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
) -> anyhow::Result<()> {
    if settings.password.is_none() {
        settings.password = secrets.get(&SecretKey {
            namespace: "account_password".to_string(),
            id: account_id.to_string(),
        })?;
    }

    if settings.access_token.is_none() {
        settings.access_token = secrets.get(&SecretKey {
            namespace: "oauth_access_token".to_string(),
            id: account_id.to_string(),
        })?;
    }

    Ok(())
}

fn validate_sqlcipher_config(
    config: &AppConfig,
    db_key: Option<String>,
) -> anyhow::Result<Option<String>> {
    if !config.database.sqlcipher_enabled {
        return Ok(None);
    }

    let key = db_key
        .context("sqlcipher is enabled but keychain secret `database/sqlcipher_key` is missing")?;
    if key.len() < 16 {
        anyhow::bail!("sqlcipher key must be at least 16 characters");
    }

    Ok(Some(key))
}
