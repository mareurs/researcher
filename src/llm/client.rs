use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::config::{Config, LlmProvider};

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content: String,
}

const DEFAULT_OPENAI_BASE_URL: &str = "http://localhost:8080/v1";
const DEFAULT_VERTEX_BASE_URL: &str = "https://aiplatform.googleapis.com/v1beta1";

/// Thin multi-provider client. OpenAI-compatible providers use `/chat/completions`;
/// Vertex uses `generateContent` / `streamGenerateContent` with ADC auth.
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    provider: LlmProvider,
    pub base_url: String,
    pub api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    strip_thinking: bool,
    disable_thinking: bool,
    vertex_project: String,
    vertex_location: String,
}

impl LlmClient {
    pub fn new(cfg: &Config) -> Self {
        let provider = LlmProvider::parse(&cfg.llm_provider);
        let base_url = resolve_base_url(&provider, &cfg.llm_base_url);

        info!(
            backend = "heavy",
            provider = ?provider,
            url = %base_url,
            model = %cfg.llm_model,
            "LlmClient::new"
        );

        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client"),
            provider,
            base_url,
            api_key: cfg.llm_api_key.clone(),
            model: cfg.llm_model.clone(),
            max_tokens: cfg.llm_max_tokens,
            temperature: cfg.llm_temperature,
            strip_thinking: cfg.strip_thinking_tokens,
            disable_thinking: false,
            vertex_project: cfg.vertex_project.clone(),
            vertex_location: cfg.vertex_location.clone(),
        }
    }

    /// Build a client for the fast/lightweight LLM backend.
    /// Falls back to the heavy backend if `LLM_FAST_BASE_URL` is empty.
    pub fn new_fast(cfg: &Config) -> Self {
        let use_fast = !cfg.llm_fast_base_url.is_empty() || !cfg.llm_fast_provider.is_empty();
        let provider = resolved_fast_provider(cfg, use_fast);

        let base_url = if use_fast && !cfg.llm_fast_base_url.is_empty() {
            resolve_base_url(&provider, &cfg.llm_fast_base_url)
        } else {
            resolve_base_url(&provider, &cfg.llm_base_url)
        };

        let api_key = if use_fast && !cfg.llm_fast_api_key.is_empty() {
            cfg.llm_fast_api_key.clone()
        } else {
            cfg.llm_api_key.clone()
        };

        let (model, max_tokens) = if use_fast {
            (cfg.llm_fast_model.clone(), cfg.llm_fast_max_tokens)
        } else {
            (cfg.llm_model.clone(), cfg.llm_max_tokens)
        };

        info!(
            backend = if use_fast { "fast" } else { "heavy (fallback)" },
            provider = ?provider,
            url = %base_url,
            model = %model,
            disable_thinking = use_fast,
            "LlmClient::new_fast"
        );

        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client"),
            provider,
            base_url,
            api_key,
            model,
            max_tokens,
            temperature: cfg.llm_temperature,
            strip_thinking: cfg.strip_thinking_tokens,
            disable_thinking: use_fast,
            vertex_project: cfg.vertex_project.clone(),
            vertex_location: cfg.vertex_location.clone(),
        }
    }

    fn ensure_vertex_ready(&self) -> Result<()> {
        if self.provider == LlmProvider::Vertex && self.vertex_project.trim().is_empty() {
            anyhow::bail!("Vertex provider requires VERTEX_PROJECT to be set");
        }
        Ok(())
    }

    /// Stream completion tokens to `tx`, accumulate and return the full text.
    pub async fn stream(
        &self,
        messages: Vec<ChatMessage>,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<String> {
        self.ensure_vertex_ready()?;

        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()?;

        match self.provider {
            LlmProvider::OpenAi => {
                crate::llm::stream::stream_completion(
                    &http,
                    &self.base_url,
                    &self.api_key,
                    &self.model,
                    self.max_tokens,
                    self.temperature,
                    self.strip_thinking,
                    messages,
                    tx,
                )
                .await
            }
            LlmProvider::Vertex => {
                let text = crate::llm::vertex::stream(
                    &http,
                    &self.base_url,
                    &self.vertex_project,
                    &self.vertex_location,
                    &self.model,
                    self.max_tokens,
                    self.temperature,
                    messages,
                    tx,
                )
                .await?;
                Ok(if self.strip_thinking { strip_thinking(&text) } else { text })
            }
        }
    }

    /// Send a non-streaming chat completion and return the assistant text.
    /// Retries up to 3 times on transient errors (5xx, 429, network failures).
    pub async fn complete(&self, messages: Vec<ChatMessage>) -> Result<String> {
        self.ensure_vertex_ready()?;

        match self.provider {
            LlmProvider::Vertex => {
                let text = crate::llm::vertex::complete(
                    &self.http,
                    &self.base_url,
                    &self.vertex_project,
                    &self.vertex_location,
                    &self.model,
                    self.max_tokens,
                    self.temperature,
                    messages,
                )
                .await?;
                return Ok(if self.strip_thinking { strip_thinking(&text) } else { text });
            }
            LlmProvider::OpenAi => {}
        }

        let url = format!("{}/chat/completions", self.base_url);

        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            stream: false,
            chat_template_kwargs: if self.disable_thinking {
                Some(json!({"enable_thinking": false}))
            } else {
                None
            },
        };

        debug!(url, model = %self.model, disable_thinking = self.disable_thinking, "LLM request");

        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0u32..3 {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << (attempt - 1));
                warn!(attempt, delay_secs = delay.as_secs(), "LLM transient error, retrying");
                tokio::time::sleep(delay).await;
            }

            let resp = match self
                .http
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&req)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("LLM HTTP request failed: {e}"));
                    continue;
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                if status.is_server_error() || status.as_u16() == 429 {
                    last_err = Some(anyhow::anyhow!("LLM error {status}: {body}"));
                    continue;
                }
                anyhow::bail!("LLM error {status}: {body}");
            }

            let chat: ChatResponse = resp.json().await.context("LLM response parse")?;
            let text = chat
                .choices
                .into_iter()
                .next()
                .map(|c| c.message.content)
                .unwrap_or_default();

            let text = text.trim().to_string();
            return Ok(if self.strip_thinking { strip_thinking(&text) } else { text });
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM request failed after 3 attempts")))
    }
}

fn resolved_fast_provider(cfg: &Config, use_fast: bool) -> LlmProvider {
    if use_fast && !cfg.llm_fast_provider.trim().is_empty() {
        LlmProvider::parse(&cfg.llm_fast_provider)
    } else {
        LlmProvider::parse(&cfg.llm_provider)
    }
}

fn resolve_base_url(provider: &LlmProvider, configured: &str) -> String {
    match provider {
        LlmProvider::OpenAi => configured.trim_end_matches('/').to_string(),
        LlmProvider::Vertex => {
            if configured.trim().is_empty() || configured == DEFAULT_OPENAI_BASE_URL {
                DEFAULT_VERTEX_BASE_URL.to_string()
            } else {
                configured.trim_end_matches('/').to_string()
            }
        }
    }
}

/// Remove `<think>...</think>` blocks emitted by Qwen3/thinking models.
fn strip_thinking(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find("<think>") {
            None => {
                out.push_str(rest);
                break;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                match rest[start..].find("</think>") {
                    None => break,
                    Some(end) => rest = &rest[start + end + 8..],
                }
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, Config};

    fn base_config() -> Config {
        Config {
            query: None,
            server: false,
            bind_addr: String::new(),
            llm_base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            llm_model: "test-model".into(),
            llm_provider: "openai".into(),
            llm_api_key: "no-key-needed".into(),
            llm_max_tokens: 256,
            llm_temperature: 0.3,
            strip_thinking_tokens: true,
            llm_fast_base_url: String::new(),
            llm_fast_model: "fast-model".into(),
            llm_fast_provider: String::new(),
            llm_fast_api_key: String::new(),
            llm_fast_max_tokens: 128,
            llm_fast_stages: vec![],
            searxng_url: String::new(),
            search_results_per_query: 8,
            vertex_project: String::new(),
            vertex_location: "eu".into(),
            brave_api_key: String::new(),
            tavily_api_key: String::new(),
            exa_api_key: String::new(),
            embed_base_url: String::new(),
            dedup_threshold: 0.92,
            rerank_base_url: String::new(),
            rerank_relevance_weight: 0.7,
            rerank_authority_weight: 0.2,
            rerank_quality_weight: 0.1,
            rerank_min_score: -5.0,
            langfuse_host: String::new(),
            langfuse_public_key: String::new(),
            langfuse_secret_key: String::new(),
            langfuse_max_payload_chars: 50000,
            min_content_words: 100,
            min_text_density: 0.05,
            max_search_queries: 4,
            max_sources_per_query: 4,
            max_page_chars: 8000,
            output: None,
            mode: "report".into(),
            domain_profile: None,
            cli_domains: vec![],
            profiles: Default::default(),
            auth: AuthConfig::default(),
            job_profile: None,
        }
    }

    #[test]
    fn new_defaults_to_openai_provider() {
        let cfg = base_config();
        let client = LlmClient::new(&cfg);
        assert_eq!(client.provider, LlmProvider::OpenAi);
        assert_eq!(client.base_url, DEFAULT_OPENAI_BASE_URL);
    }

    #[test]
    fn new_fast_inherits_primary_provider_when_fast_provider_empty() {
        let mut cfg = base_config();
        cfg.llm_provider = "vertex".into();
        let client = LlmClient::new_fast(&cfg);
        assert_eq!(client.provider, LlmProvider::Vertex);
    }

    #[test]
    fn vertex_client_requires_project() {
        let mut cfg = base_config();
        cfg.llm_provider = "vertex".into();
        let client = LlmClient::new(&cfg);
        assert!(client.ensure_vertex_ready().is_err());
    }

    #[test]
    fn vertex_provider_uses_vertex_default_base_url() {
        let mut cfg = base_config();
        cfg.llm_provider = "vertex".into();
        cfg.vertex_project = "sp-mrv-gemini-dev".into();
        let client = LlmClient::new(&cfg);
        assert_eq!(client.base_url, DEFAULT_VERTEX_BASE_URL);
    }
}
