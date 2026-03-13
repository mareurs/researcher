pub mod duckduckgo;
pub mod google;
pub mod searxng;

use anyhow::Result;
use reqwest::Client;
use searxng::SearchResult;
use tracing::warn;

/// Search with SearXNG, falling back to DuckDuckGo lite if it fails.
pub async fn search_with_fallback(
    http: &Client,
    searxng_url: &str,
    google_api_key: &str,
    google_cse_id: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    // 1. SearXNG
    match searxng::search(http, searxng_url, query, num_results).await {
        Ok(results) if !results.is_empty() => return Ok(results),
        Ok(_) => warn!(%query, "SearXNG returned empty results"),
        Err(e) => warn!(%e, %query, "SearXNG failed"),
    }

    // 2. Google Custom Search (if configured)
    if !google_api_key.is_empty() && !google_cse_id.is_empty() {
        match google::search(http, google_api_key, google_cse_id, query, num_results).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => warn!(%query, "Google CSE returned empty results, falling back to DuckDuckGo"),
            Err(e) => warn!(%e, %query, "Google CSE failed, falling back to DuckDuckGo"),
        }
    }

    // 3. DuckDuckGo
    duckduckgo::search(http, query, num_results).await
}
