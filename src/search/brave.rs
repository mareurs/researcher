use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use super::searxng::SearchResult;

#[derive(Deserialize)]
struct BraveResponse {
    #[serde(default)]
    web: WebResults,
}

#[derive(Deserialize, Default)]
struct WebResults {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

pub async fn search(
    http: &Client,
    api_key: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "Brave Search");

    // Brave API hard-caps at 20 results
    let count = num_results.min(20);

    let resp = http
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &count.to_string())])
        .send()
        .await
        .context("Brave Search request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Brave Search error {status}: {body}");
    }

    let body: BraveResponse = resp.json().await.context("Brave Search JSON parse")?;

    let results = body
        .web
        .results
        .into_iter()
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.description.chars().take(300).collect(),
            content: None,
        })
        .collect();

    Ok(results)
}
