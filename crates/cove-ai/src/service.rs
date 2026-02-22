use crate::AiError;
use cove_core::{AiMode, AiResponse, CloudAiProvider, DataProvenance};
use cove_security::{SecretKey, SecretStore};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRuntime {
    pub enabled: bool,
    pub llama_cpp_binary: Option<String>,
    pub model_path: Option<String>,
    pub max_tokens: usize,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudProviderRuntime {
    pub enabled: bool,
    pub model: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRuntimeConfig {
    pub local: LocalRuntime,
    pub cloud_enabled: bool,
    pub cloud_feature_opt_in: BTreeSet<String>,
    pub cloud: BTreeMap<CloudAiProvider, CloudProviderRuntime>,
}

impl Default for AiRuntimeConfig {
    fn default() -> Self {
        let mut cloud = BTreeMap::new();
        cloud.insert(
            CloudAiProvider::OpenAi,
            CloudProviderRuntime {
                enabled: false,
                model: "gpt-4o-mini".to_string(),
                endpoint: Some("https://api.openai.com/v1/chat/completions".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::Anthropic,
            CloudProviderRuntime {
                enabled: false,
                model: "claude-3-5-sonnet-latest".to_string(),
                endpoint: Some("https://api.anthropic.com/v1/messages".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::Gemini,
            CloudProviderRuntime {
                enabled: false,
                model: "gemini-1.5-pro".to_string(),
                endpoint: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::Mistral,
            CloudProviderRuntime {
                enabled: false,
                model: "mistral-large-latest".to_string(),
                endpoint: Some("https://api.mistral.ai/v1/chat/completions".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::Groq,
            CloudProviderRuntime {
                enabled: false,
                model: "llama-3.3-70b-versatile".to_string(),
                endpoint: Some("https://api.groq.com/openai/v1/chat/completions".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::Grok,
            CloudProviderRuntime {
                enabled: false,
                model: "grok-beta".to_string(),
                endpoint: Some("https://api.x.ai/v1/chat/completions".to_string()),
            },
        );
        cloud.insert(
            CloudAiProvider::OpenRouter,
            CloudProviderRuntime {
                enabled: false,
                model: "openrouter/auto".to_string(),
                endpoint: Some("https://openrouter.ai/api/v1/chat/completions".to_string()),
            },
        );

        Self {
            local: LocalRuntime {
                enabled: true,
                llama_cpp_binary: None,
                model_path: None,
                max_tokens: 512,
                temperature: 0.2,
            },
            cloud_enabled: false,
            cloud_feature_opt_in: BTreeSet::new(),
            cloud,
        }
    }
}

#[derive(Clone)]
pub struct AiService {
    config: AiRuntimeConfig,
    secrets: SecretStore,
    http: reqwest::Client,
}

impl AiService {
    pub fn new(config: AiRuntimeConfig, secrets: SecretStore) -> Self {
        Self {
            config,
            secrets,
            http: reqwest::Client::new(),
        }
    }

    pub fn update_config(&mut self, config: AiRuntimeConfig) {
        self.config = config;
    }

    pub async fn summarize_email(
        &self,
        subject: &str,
        body: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        let feature = "email_summarization";
        let prompt =
            format!("Summarize this email in 4 bullet points. Subject: {subject}\nBody:\n{body}");

        self.run_feature(feature, &prompt, mode, cloud_provider)
            .await
    }

    /// Summarize an entire email thread (multiple messages).
    pub async fn summarize_thread(
        &self,
        messages: &[(String, String, String)], // (sender, subject, body_snippet)
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        let feature = "email_summarization";
        let mut prompt = String::from("Summarize this email thread in 3-5 bullet points:\n\n");
        for (i, (sender, subject, body)) in messages.iter().enumerate() {
            let snippet: String = body.chars().take(500).collect();
            prompt.push_str(&format!("Message {}: From: {sender}, Subject: {subject}\n{snippet}\n\n", i + 1));
        }
        self.run_feature(feature, &prompt, mode, cloud_provider).await
    }

    /// Suggest a draft reply to the latest message.
    pub async fn draft_reply_suggestion(
        &self,
        sender: &str,
        subject: &str,
        body: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        let feature = "suggested_reply";
        let snippet: String = body.chars().take(1000).collect();
        let prompt = format!(
            "Draft a brief, professional reply to this email. Only output the reply body.\n\
             From: {sender}\nSubject: {subject}\n\n{snippet}"
        );
        self.run_feature(feature, &prompt, mode, cloud_provider).await
    }

    pub async fn generate_message(
        &self,
        prompt: &str,
        format: &str,
        tone: &str,
        length: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        let feature = "magic_compose";
        let system_prompt = format!(
            "Generate a message based on the user's prompt.\n\
             Enforce the following constraints strictly:\n\
             - Format: {format}\n\
             - Tone: {tone}\n\
             - Length: {length}\n\n\
             User Prompt: {prompt}"
        );

        self.run_feature(feature, &system_prompt, mode, cloud_provider)
            .await
    }

    pub async fn suggest_reply(
        &self,
        subject: &str,
        body: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        let feature = "suggested_reply";
        let prompt = format!(
            "Draft a concise, polite reply. Never send automatically. Subject: {subject}\nBody:\n{body}"
        );

        self.run_feature(feature, &prompt, mode, cloud_provider)
            .await
    }

    pub async fn extract_action_items(
        &self,
        body: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(Vec<String>, DataProvenance), AiError> {
        let feature = "action_extraction";
        let prompt =
            format!("Extract action items from this email as one short line each:\n{body}");

        let (response, provenance) = self
            .run_feature(feature, &prompt, mode, cloud_provider)
            .await?;
        let lines = response
            .output
            .lines()
            .map(|line| line.trim().trim_start_matches('-').trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();

        Ok((lines, provenance))
    }

    pub fn importance_score(&self, subject: &str, body: &str) -> u8 {
        let text = format!("{subject} {body}").to_ascii_lowercase();
        let high = [
            "urgent",
            "asap",
            "deadline",
            "invoice",
            "security",
            "production",
        ];
        let medium = ["meeting", "review", "action", "follow up"];

        let high_hits = high.iter().filter(|word| text.contains(**word)).count();
        let medium_hits = medium.iter().filter(|word| text.contains(**word)).count();

        let score = (high_hits * 30 + medium_hits * 15).clamp(0, 100);
        score as u8
    }

    pub fn classify_email(&self, subject: &str, body: &str) -> Vec<String> {
        let text = format!("{subject} {body}").to_ascii_lowercase();
        let mut labels = Vec::new();

        if text.contains("invoice") || text.contains("payment") {
            labels.push("finance".to_string());
        }
        if text.contains("meeting") || text.contains("calendar") {
            labels.push("calendar".to_string());
        }
        if text.contains("todo") || text.contains("action") || text.contains("follow up") {
            labels.push("action-required".to_string());
        }
        if labels.is_empty() {
            labels.push("general".to_string());
        }

        labels
    }

    pub fn natural_language_search_to_query(&self, prompt: &str) -> String {
        let normalized = prompt.to_ascii_lowercase();
        let from = Regex::new(r"from\s+([a-z0-9._%+-]+@[a-z0-9.-]+)")
            .ok()
            .and_then(|re| re.captures(&normalized))
            .and_then(|cap| cap.get(1).map(|m| m.as_str().to_string()));

        let mut parts = Vec::new();
        if let Some(from) = from {
            parts.push(format!("from:{from}"));
        }
        if normalized.contains("last week") {
            parts.push("received:last_7_days".to_string());
        }
        if normalized.contains("budget") {
            parts.push("budget".to_string());
        }

        if parts.is_empty() {
            normalized
        } else {
            parts.join(" AND ")
        }
    }

    async fn run_feature(
        &self,
        feature: &str,
        prompt: &str,
        mode: AiMode,
        cloud_provider: Option<CloudAiProvider>,
    ) -> Result<(AiResponse, DataProvenance), AiError> {
        match mode {
            AiMode::Local => {
                let output = self.run_local(prompt).await?;
                Ok((
                    AiResponse {
                        feature: feature.to_string(),
                        output,
                        mode: AiMode::Local,
                        provider: "llama.cpp".to_string(),
                        confidence: 0.66,
                    },
                    DataProvenance {
                        feature: feature.to_string(),
                        mode: AiMode::Local,
                        destination: "local_device".to_string(),
                        reason: "Local model inference".to_string(),
                    },
                ))
            }
            AiMode::Cloud => {
                self.ensure_cloud_allowed(feature)?;
                let provider = cloud_provider
                    .ok_or_else(|| AiError::Config("cloud provider is required".to_string()))?;
                let output = self.run_cloud(prompt, provider.clone()).await?;

                Ok((
                    AiResponse {
                        feature: feature.to_string(),
                        output,
                        mode: AiMode::Cloud,
                        provider: format!("{provider:?}"),
                        confidence: 0.72,
                    },
                    DataProvenance {
                        feature: feature.to_string(),
                        mode: AiMode::Cloud,
                        destination: format!("{:?} API", provider),
                        reason: "User opted in for cloud inference".to_string(),
                    },
                ))
            }
        }
    }

    fn ensure_cloud_allowed(&self, feature: &str) -> Result<(), AiError> {
        if !self.config.cloud_enabled {
            return Err(AiError::CloudOptInRequired(
                "cloud AI is globally disabled".to_string(),
            ));
        }

        if !self.config.cloud_feature_opt_in.contains(feature) {
            return Err(AiError::CloudOptInRequired(format!(
                "feature `{feature}` is not opted in for cloud"
            )));
        }

        Ok(())
    }

    async fn run_local(&self, prompt: &str) -> Result<String, AiError> {
        if !self.config.local.enabled {
            return Err(AiError::Config("local AI is disabled".to_string()));
        }

        let binary = self
            .config
            .local
            .llama_cpp_binary
            .as_ref()
            .ok_or_else(|| AiError::Config("llama.cpp binary path is missing".to_string()))?;
        let model = self
            .config
            .local
            .model_path
            .as_ref()
            .ok_or_else(|| AiError::Config("GGUF model path is missing".to_string()))?;

        let output = Command::new(binary)
            .arg("-m")
            .arg(model)
            .arg("-n")
            .arg(self.config.local.max_tokens.to_string())
            .arg("--temp")
            .arg(self.config.local.temperature.to_string())
            .arg("-p")
            .arg(prompt)
            .output()
            .await?;

        if !output.status.success() {
            return Err(AiError::Inference(format!(
                "llama.cpp returned status {}",
                output.status
            )));
        }

        let text = String::from_utf8(output.stdout)?;
        Ok(text.trim().to_string())
    }

    async fn run_cloud(&self, prompt: &str, provider: CloudAiProvider) -> Result<String, AiError> {
        let provider_cfg =
            self.config.cloud.get(&provider).ok_or_else(|| {
                AiError::Config(format!("missing provider config for {provider:?}"))
            })?;

        if !provider_cfg.enabled {
            return Err(AiError::Config(format!(
                "provider {provider:?} is disabled"
            )));
        }

        let api_key = self
            .secrets
            .get(&SecretKey {
                namespace: "ai_api_key".to_string(),
                id: format!("{provider:?}").to_ascii_lowercase(),
            })?
            .ok_or_else(|| AiError::Config(format!("missing API key for {provider:?}")))?;

        match provider {
            CloudAiProvider::OpenAi
            | CloudAiProvider::Mistral
            | CloudAiProvider::Groq
            | CloudAiProvider::Grok
            | CloudAiProvider::OpenRouter => {
                let endpoint = provider_cfg
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| AiError::Config(format!("missing endpoint for {provider:?}")))?;

                let response = self
                    .http
                    .post(endpoint)
                    .bearer_auth(api_key)
                    .json(&serde_json::json!({
                        "model": provider_cfg.model,
                        "messages": [
                            {"role": "system", "content": "You are Cove Mail's assistant."},
                            {"role": "user", "content": prompt}
                        ]
                    }))
                    .send()
                    .await?
                    .error_for_status()?;

                let json: serde_json::Value = response.json().await?;
                Ok(json
                    .pointer("/choices/0/message/content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string())
            }
            CloudAiProvider::Anthropic => {
                let endpoint = provider_cfg
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| AiError::Config("missing endpoint for Anthropic".to_string()))?;

                let response = self
                    .http
                    .post(endpoint)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&serde_json::json!({
                        "model": provider_cfg.model,
                        "max_tokens": 512,
                        "messages": [{"role":"user", "content": prompt}]
                    }))
                    .send()
                    .await?
                    .error_for_status()?;

                let json: serde_json::Value = response.json().await?;
                Ok(json
                    .pointer("/content/0/text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string())
            }
            CloudAiProvider::Gemini => {
                let base = provider_cfg
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| AiError::Config("missing endpoint for Gemini".to_string()))?;
                let endpoint = format!(
                    "{base}/models/{}:generateContent?key={}",
                    provider_cfg.model, api_key
                );

                let response = self
                    .http
                    .post(endpoint)
                    .json(&serde_json::json!({
                        "contents": [{"parts": [{"text": prompt}]}]
                    }))
                    .send()
                    .await?
                    .error_for_status()?;

                let json: serde_json::Value = response.json().await?;
                Ok(json
                    .pointer("/candidates/0/content/parts/0/text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string())
            }
        }
    }

    /// Fetches all available models from all enabled providers (Cloud and active Local).
    pub async fn fetch_available_models(&self) -> Result<BTreeMap<String, Vec<String>>, AiError> {
        let mut all_models = BTreeMap::new();

        // Check local models
        if let Ok(local_models) = self.discover_local_models().await {
            if !local_models.is_empty() {
                all_models.insert("Local".to_string(), local_models);
            }
        }

        // Check enabled cloud providers
        for (provider, cfg) in &self.config.cloud {
            if !cfg.enabled {
                continue;
            }

            if let Ok(key) = self.secrets.get(&SecretKey {
                namespace: "ai_api_key".to_string(),
                id: format!("{provider:?}").to_ascii_lowercase(),
            }) {
                if let Some(key) = key {
                    match self.fetch_cloud_models(provider, &key, cfg.endpoint.as_deref()).await {
                        Ok(models) => {
                            all_models.insert(format!("{provider:?}"), models);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch models for {provider:?}: {e}");
                        }
                    }
                }
            }
        }

        Ok(all_models)
    }

    async fn discover_local_models(&self) -> Result<Vec<String>, AiError> {
        let mut models = std::collections::BTreeSet::new();

        // 1. Check Ollama default port (11434)
        let ollama_url = "http://localhost:11434/api/tags";
        if let Ok(res) = self.http.get(ollama_url).timeout(std::time::Duration::from_millis(500)).send().await {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(arr) = json.pointer("/models").and_then(|v| v.as_array()) {
                    for m in arr {
                        if let Some(name) = m.pointer("/name").and_then(|v| v.as_str()) {
                            models.insert(format!("ollama/{name}"));
                        }
                    }
                }
            }
        }

        // 2. Check LM Studio default port (1234)
        let lm_studio_url = "http://localhost:1234/v1/models";
        if let Ok(res) = self.http.get(lm_studio_url).timeout(std::time::Duration::from_millis(500)).send().await {
             if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(arr) = json.pointer("/data").and_then(|v| v.as_array()) {
                    for m in arr {
                        if let Some(id) = m.pointer("/id").and_then(|v| v.as_str()) {
                            models.insert(format!("lm-studio/{id}"));
                        }
                    }
                }
            }
        }

        Ok(models.into_iter().collect())
    }

    async fn fetch_cloud_models(
        &self,
        provider: &CloudAiProvider,
        api_key: &str,
        default_endpoint: Option<&str>,
    ) -> Result<Vec<String>, AiError> {
        // Many use the standard openai /v1/models endpoint shape
        match provider {
            CloudAiProvider::OpenAi 
            | CloudAiProvider::Groq 
            | CloudAiProvider::Grok 
            | CloudAiProvider::Mistral
            | CloudAiProvider::OpenRouter => {
                let base_url = default_endpoint.and_then(|e| {
                    if e.ends_with("/chat/completions") {
                        Some(e.replace("/chat/completions", "/models"))
                    } else if e.ends_with("/messages") { // catch-all
                        Some(e.replace("/messages", "/models"))
                    } else {
                        Some(format!("{}/models", e.trim_end_matches('/')))
                    }
                }).unwrap_or_else(|| "https://api.openai.com/v1/models".to_string());

                let res = self.http.get(&base_url)
                    .bearer_auth(api_key)
                    .send()
                    .await?
                    .error_for_status()?;
                
                let json: serde_json::Value = res.json().await?;
                let mut models = Vec::new();
                if let Some(data) = json.pointer("/data").and_then(|v| v.as_array()) {
                    for item in data {
                        if let Some(id) = item.pointer("/id").and_then(|v| v.as_str()) {
                            models.push(id.to_string());
                        }
                    }
                }
                models.sort();
                Ok(models)
            },
            CloudAiProvider::Anthropic => {
                Ok(vec![
                    "claude-3-5-sonnet-latest".to_string(),
                    "claude-3-5-haiku-latest".to_string(),
                    "claude-3-opus-latest".to_string(),
                ])
            },
            CloudAiProvider::Gemini => {
               // The Gemini API requires the key in the query string and the endpoint is different.
               // We will just return known models for Gemini for now as `/v1beta/models` acts slightly differently
               Ok(vec![
                   "gemini-1.5-flash".to_string(),
                   "gemini-1.5-pro".to_string(),
                   "gemini-2.0-flash-exp".to_string(),
               ])
            }
        }
    }
}
