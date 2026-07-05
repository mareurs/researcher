use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Serialize)]
struct EmbedRequest {
    inputs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EmbedResponse {
    /// TEI returns a flat Vec<Vec<f32>> for batch requests
    Batch(Vec<Vec<f32>>),
    /// OpenAI-compat format (vLLM embedding endpoint)
    OpenAI { data: Vec<EmbedObject> },
}

#[derive(Deserialize)]
struct EmbedObject {
    embedding: Vec<f32>,
}

/// Client for HuggingFace Text Embeddings Inference (TEI).
/// TEI exposes `/embed` for batch embedding — same model used by gpt-researcher.
#[derive(Clone)]
pub struct EmbedClient {
    http: Client,
    base_url: String,
}

impl EmbedClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Embed a batch of texts. Returns one vector per input.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        debug!(count = texts.len(), "embedding batch");

        let url = format!("{}/embed", self.base_url);
        let req = EmbedRequest { inputs: texts.to_vec() };

        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("embed request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("TEI embed error {status}: {body}");
        }

        let body: EmbedResponse = resp.json().await.context("embed parse")?;
        let vecs = match body {
            EmbedResponse::Batch(v) => v,
            EmbedResponse::OpenAI { data } => data.into_iter().map(|o| o.embedding).collect(),
        };

        Ok(vecs)
    }

    /// Convenience: embed a single text.
    #[allow(dead_code)]
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut batch = self.embed(&[text.to_string()]).await?;
        batch.pop().context("empty embed response")
    }
}
