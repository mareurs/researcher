use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use super::searxng::SearchResult;

#[derive(Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyResult>,
}

#[derive(Deserialize)]
struct TavilyResult {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

pub async fn search(
    http: &Client,
    api_key: &str,
    domains: &[String],
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "Tavily Search");

    let mut body = serde_json::json!({
        "query": query,
        "max_results": num_results,
        "search_depth": "basic"
    });
    if !domains.is_empty() {
        body["include_domains"] = serde_json::json!(domains);
    }

    let resp = http
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("Tavily Search request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Tavily Search error {status}: {body}");
    }

    let body: TavilyResponse = resp.json().await.context("Tavily Search JSON parse")?;

    let results = body
        .results
        .into_iter()
        .filter_map(|r| {
            if r.url.is_empty() {
                return None;
            }
            let snippet: String = r.content.chars().take(300).collect();
            Some(SearchResult {
                title: r.title,
                url: r.url,
                snippet,
                content: Some(r.content),
            })
        })
        .collect();

    Ok(results)
}
