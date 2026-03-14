pub mod duckduckgo;
pub mod searxng;
pub mod vertex;

use anyhow::Result;
use reqwest::Client;
use tracing::warn;

pub use searxng::SearchResult;

/// Search with SearXNG, falling back to Vertex AI Search then DuckDuckGo lite.
pub async fn search_with_fallback(
    http: &Client,
    searxng_url: &str,
    gcloud_path: &str,
    vertex_project: &str,
    vertex_engine_id: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    // 1. SearXNG
    match searxng::search(http, searxng_url, query, num_results).await {
        Ok(results) if !results.is_empty() => return Ok(results),
        Ok(_) => warn!(%query, "SearXNG returned empty results"),
        Err(e) => warn!(%e, %query, "SearXNG failed"),
    }

    // 2. Vertex AI Search (if configured)
    if !vertex_project.is_empty() && !vertex_engine_id.is_empty() {
        match vertex::search(http, gcloud_path, vertex_project, vertex_engine_id, query, num_results).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => warn!(%query, "Vertex AI Search returned empty results, falling back to DuckDuckGo"),
            Err(e) => warn!(%e, %query, "Vertex AI Search failed, falling back to DuckDuckGo"),
        }
    }

    // 3. DuckDuckGo
    duckduckgo::search(http, query, num_results).await
}
