use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use super::searxng::SearchResult;

#[derive(Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<ExaResult>,
}

#[derive(Deserialize)]
struct ExaResult {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    text: String,
}

pub async fn search(
    http: &Client,
    api_key: &str,
    domains: &[String],
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "Exa Search");

    let mut body = serde_json::json!({
        "query": query,
        "numResults": num_results,
        "type": "auto",
        "contents": { "text": true }
    });
    if !domains.is_empty() {
        body["includeDomains"] = serde_json::json!(domains);
    }

    let resp = http
        .post("https://api.exa.ai/search")
        .header("x-api-key", api_key)
        .json(&body)
        .send()
        .await
        .context("Exa Search request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Exa Search error {status}: {body}");
    }

    let body: ExaResponse = resp.json().await.context("Exa Search JSON parse")?;

    let results = body
        .results
        .into_iter()
        .filter_map(|r| {
            if r.url.is_empty() {
                return None;
            }
            let snippet: String = r.text.chars().take(300).collect();
            Some(SearchResult {
                title: r.title,
                url: r.url,
                snippet,
                content: Some(r.text),
            })
        })
        .collect();

    Ok(results)
}
