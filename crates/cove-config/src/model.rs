use cove_core::{AiMode, CloudAiProvider};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub profile_name: String,
    pub privacy: PrivacyConfig,
    pub database: DatabaseConfig,
    pub sync: SyncConfig,
    pub ai: AiConfig,
    pub ui: UiConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    pub telemetry_enabled: bool,
    pub analytics_enabled: bool,
    pub block_untrusted_remote_content: bool,
    pub default_ai_mode: AiMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub file_name: String,
    pub sqlcipher_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub email_poll_interval_secs: u64,
    pub calendar_poll_interval_secs: u64,
    pub task_poll_interval_secs: u64,
    pub max_parallel_jobs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub local: LocalAiConfig,
    pub cloud: CloudAiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiConfig {
    pub enabled: bool,
    pub llama_cpp_binary: Option<String>,
    pub model_path: Option<String>,
    pub context_tokens: usize,
    pub gpu_layers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudAiConfig {
    pub enabled: bool,
    pub per_feature_opt_in: bool,
    pub feature_opt_in: Vec<String>,
    pub default_provider: Option<CloudAiProvider>,
    pub providers: BTreeMap<String, CloudProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudProviderConfig {
    pub enabled: bool,
    pub model: String,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub compact_density: bool,
    pub default_start_page: String,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub new_mail_enabled: bool,
    pub new_mail_sound: bool,
    pub reminder_enabled: bool,
    pub reminder_minutes_before: Vec<i64>,
    pub quiet_hours_enabled: bool,
    pub quiet_hours_start: String,
    pub quiet_hours_end: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            new_mail_enabled: true,
            new_mail_sound: false,
            reminder_enabled: true,
            reminder_minutes_before: vec![15, 5],
            quiet_hours_enabled: false,
            quiet_hours_start: "22:00".to_string(),
            quiet_hours_end: "08:00".to_string(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        providers.insert(
            "openai".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "gpt-4o-mini".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "anthropic".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "claude-3-5-sonnet-latest".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "gemini".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "gemini-1.5-pro".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "mistral".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "mistral-large-latest".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "groq".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "llama-3.3-70b-versatile".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "grok".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "grok-beta".to_string(),
                api_base: None,
            },
        );
        providers.insert(
            "openrouter".to_string(),
            CloudProviderConfig {
                enabled: false,
                model: "openrouter/auto".to_string(),
                api_base: None,
            },
        );

        Self {
            version: 1,
            profile_name: "default".to_string(),
            privacy: PrivacyConfig {
                telemetry_enabled: false,
                analytics_enabled: false,
                block_untrusted_remote_content: true,
                default_ai_mode: AiMode::Local,
            },
            database: DatabaseConfig {
                file_name: "covemail.sqlite3".to_string(),
                sqlcipher_enabled: false,
            },
            sync: SyncConfig {
                email_poll_interval_secs: 120,
                calendar_poll_interval_secs: 300,
                task_poll_interval_secs: 300,
                max_parallel_jobs: 4,
            },
            ai: AiConfig {
                local: LocalAiConfig {
                    enabled: true,
                    llama_cpp_binary: None,
                    model_path: None,
                    context_tokens: 4096,
                    gpu_layers: 32,
                },
                cloud: CloudAiConfig {
                    enabled: false,
                    per_feature_opt_in: true,
                    feature_opt_in: Vec::new(),
                    default_provider: None,
                    providers,
                },
            },
            ui: UiConfig {
                compact_density: false,
                default_start_page: "inbox".to_string(),
                timezone: None,
            },
            notifications: NotificationConfig::default(),
        }
    }
}
