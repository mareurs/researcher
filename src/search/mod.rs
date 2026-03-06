pub mod duckduckgo;
pub mod searxng;

use anyhow::Result;
use reqwest::Client;
use searxng::SearchResult;
use tracing::warn;

/// Search with SearXNG, falling back to DuckDuckGo lite if it fails.
pub async fn search_with_fallback(
    http: &Client,
    searxng_url: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    match searxng::search(http, searxng_url, query, num_results).await {
        Ok(results) if !results.is_empty() => Ok(results),
        Ok(_) => {
            warn!(%query, "SearXNG returned empty results, trying DuckDuckGo");
            duckduckgo::search(http, query, num_results).await
        }
        Err(e) => {
            warn!(%e, %query, "SearXNG failed, trying DuckDuckGo");
            duckduckgo::search(http, query, num_results).await
        }
    }
}
