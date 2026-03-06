use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    #[allow(dead_code)]
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

/// Query SearXNG's JSON API. SearXNG acts as a privacy-preserving meta-search
/// engine — it fans out to Google, Bing, DuckDuckGo, etc. without API keys.
pub async fn search(
    http: &Client,
    searxng_url: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    let url = format!("{}/search", searxng_url.trim_end_matches('/'));

    debug!(%query, "SearXNG search");

    let resp = http
        .get(&url)
        .query(&[
            ("q", query),
            ("format", "json"),
            ("language", "en"),
            ("categories", "general"),
        ])
        .send()
        .await
        .context("SearXNG request")?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("SearXNG error {status}");
    }

    let body: SearxngResponse = resp.json().await.context("SearXNG JSON parse")?;

    let results = body
        .results
        .into_iter()
        .take(num_results)
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.content,
        })
        .collect();

    Ok(results)
}
