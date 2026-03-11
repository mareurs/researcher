use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info};

use crate::config::Config;

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

/// Thin OpenAI-compatible client. Works against llama.cpp server, vLLM,
/// Ollama (/v1), or api.openai.com — just swap LLM_BASE_URL.
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    pub base_url: String,
    pub api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    strip_thinking: bool,
    disable_thinking: bool,
}

impl LlmClient {
pub fn new(cfg: &Config) -> Self {
        info!(
            backend = "heavy",
            url = %cfg.llm_base_url,
            model = %cfg.llm_model,
            "LlmClient::new"
        );
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client"),
            base_url: cfg.llm_base_url.trim_end_matches('/').to_string(),
            api_key: cfg.llm_api_key.clone(),
            model: cfg.llm_model.clone(),
            max_tokens: cfg.llm_max_tokens,
            temperature: cfg.llm_temperature,
            strip_thinking: cfg.strip_thinking_tokens,
            disable_thinking: false,
        }
    }

    /// Build a client for the fast/lightweight LLM backend.
    /// Falls back to the heavy backend if `LLM_FAST_BASE_URL` is empty.
    pub fn new_fast(cfg: &Config) -> Self {
        let use_fast = !cfg.llm_fast_base_url.is_empty();

        let base_url = if use_fast {
            &cfg.llm_fast_base_url
        } else {
            &cfg.llm_base_url
        };

        let api_key = if use_fast && !cfg.llm_fast_api_key.is_empty() {
            &cfg.llm_fast_api_key
        } else {
            &cfg.llm_api_key
        };

        let (model, max_tokens) = if use_fast {
            (&cfg.llm_fast_model, cfg.llm_fast_max_tokens)
        } else {
            (&cfg.llm_model, cfg.llm_max_tokens)
        };

        info!(
            backend = if use_fast { "fast" } else { "heavy (fallback)" },
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
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.clone(),
            model: model.clone(),
            max_tokens,
            temperature: cfg.llm_temperature,
            strip_thinking: cfg.strip_thinking_tokens,
            disable_thinking: use_fast,
        }
    }


    /// Stream completion tokens to `tx`, accumulate and return the full text.
pub async fn stream(
        &self,
        messages: Vec<ChatMessage>,
        tx: tokio::sync::mpsc::Sender<String>,
    ) -> Result<String> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()?;
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


    /// Send a non-streaming chat completion and return the assistant text.
pub async fn complete(&self, messages: Vec<ChatMessage>) -> Result<String> {
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

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .context("LLM HTTP request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
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
        Ok(if self.strip_thinking { strip_thinking(&text) } else { text })
    }
}

/// Remove `<think>...</think>` blocks emitted by Qwen3/thinking models.
fn strip_thinking(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find("<think>") {
            None => { out.push_str(rest); break; }
            Some(start) => {
                out.push_str(&rest[..start]);
                match rest[start..].find("</think>") {
                    None => break, // malformed — drop the rest
                    Some(end) => rest = &rest[start + end + 8..],
                }
            }
        }
    }
    out.trim().to_string()
}

