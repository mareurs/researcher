use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use super::searxng::SearchResult;

#[derive(Deserialize)]
struct GoogleResponse {
    #[serde(default)]
    items: Vec<GoogleItem>,
}

#[derive(Deserialize)]
struct GoogleItem {
    title: String,
    link: String,
    #[serde(default)]
    snippet: String,
}

pub async fn search(
    http: &Client,
    api_key: &str,
    cse_id: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "Google Custom Search");

    // Google CSE returns at most 10 results per request
    let num = num_results.min(10).to_string();

    let resp = http
        .get("https://www.googleapis.com/customsearch/v1")
        .query(&[("key", api_key), ("cx", cse_id), ("q", query), ("num", &num)])
        .send()
        .await
        .context("Google CSE request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Google CSE error {status}: {body}");
    }

    let body: GoogleResponse = resp.json().await.context("Google CSE JSON parse")?;

    Ok(body
        .items
        .into_iter()
        .map(|item| SearchResult {
            title: item.title,
            url: item.link,
            snippet: item.snippet,
        })
        .collect())
}
