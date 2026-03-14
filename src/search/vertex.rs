use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use super::searxng::SearchResult;

// Response structs — all fields optional since derivedStructData
// may be absent when a document hasn't been fully crawled.
#[derive(Deserialize, Default)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResultItem>,
}

#[derive(Deserialize, Default)]
struct SearchResultItem {
    #[serde(default)]
    document: Document,
}

#[derive(Deserialize, Default)]
struct Document {
    #[serde(default, rename = "derivedStructData")]
    derived_struct_data: DerivedStructData,
}

#[derive(Deserialize, Default)]
struct DerivedStructData {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

#[derive(Deserialize, Default)]
struct Snippet {
    #[serde(default)]
    snippet: String,
}

pub async fn search(
    http: &Client,
    gcloud_path: &str,
    project: &str,
    engine_id: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "Vertex AI Search");

    // Acquire bearer token via gcloud
    let output = std::process::Command::new(gcloud_path)
        .args(["auth", "print-access-token"])
        .output()
        .context("failed to spawn gcloud")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gcloud auth print-access-token failed: {stderr}");
    }

    let token = String::from_utf8(output.stdout)
        .context("gcloud output not UTF-8")?;
    let token = token.trim();

    // Vertex AI Search caps pageSize at 10
    let page_size = num_results.min(10);

    let url = format!(
        "https://discoveryengine.googleapis.com/v1alpha/projects/{project}/locations/global\
         /collections/default_collection/engines/{engine_id}\
         /servingConfigs/default_search:searchLite"
    );

    let resp = http
        .post(&url)
        .bearer_auth(token)
        .header("x-goog-user-project", project)
        .json(&serde_json::json!({ "query": query, "pageSize": page_size }))
        .send()
        .await
        .context("Vertex AI Search request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Vertex AI Search error {status}: {body}");
    }

    let body: SearchResponse = resp.json().await.context("Vertex AI Search JSON parse")?;

    let results = body
        .results
        .into_iter()
        .filter_map(|item| {
            let data = item.document.derived_struct_data;
            // Skip results with no URL — useless without a link to scrape
            let url = data.link?;
            let title = data.title.unwrap_or_default();
            let snippet = data.snippets.into_iter().next().map(|s| s.snippet).unwrap_or_default();
            Some(SearchResult { title, url, snippet })
        })
        .collect();

    Ok(results)
}
