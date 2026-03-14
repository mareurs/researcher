pub mod brave;
pub mod duckduckgo;
pub mod exa;
pub mod searxng;
pub mod tavily;

use anyhow::Result;
use reqwest::Client;
use tracing::warn;

pub use searxng::SearchResult;

/// Search with profile-based routing.
///
/// Primary backend is chosen by `domain_profile`:
///   "news"                     → Tavily (real-time web crawl)
///   "academic"                 → Exa    (neural/semantic index)
///   "tech-news" | "llm-news"   → Brave  (full web index)
///   "shopping-ro" | "travel"   → Brave
///   None / unknown             → Brave
///
/// If the primary backend's key is empty, falls through to SearXNG → DuckDuckGo.
pub async fn search_with_fallback(
    http: &Client,
    searxng_url: &str,
    brave_api_key: &str,
    tavily_api_key: &str,
    exa_api_key: &str,
    domain_profile: Option<&str>,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    // 1. Profile-selected primary backend
    match domain_profile {
        Some("news") if !tavily_api_key.is_empty() => {
            match tavily::search(http, tavily_api_key, query, num_results).await {
                Ok(results) if !results.is_empty() => return Ok(results),
                Ok(_) => warn!(%query, "Tavily returned empty results, falling back"),
                Err(e) => warn!(%e, %query, "Tavily failed, falling back"),
            }
        }
        Some("academic") if !exa_api_key.is_empty() => {
            match exa::search(http, exa_api_key, query, num_results).await {
                Ok(results) if !results.is_empty() => return Ok(results),
                Ok(_) => warn!(%query, "Exa returned empty results, falling back"),
                Err(e) => warn!(%e, %query, "Exa failed, falling back"),
            }
        }
        _ if !brave_api_key.is_empty() => {
            match brave::search(http, brave_api_key, query, num_results).await {
                Ok(results) if !results.is_empty() => return Ok(results),
                Ok(_) => warn!(%query, "Brave returned empty results, falling back"),
                Err(e) => warn!(%e, %query, "Brave failed, falling back"),
            }
        }
        _ => {}
    }

    // 2. SearXNG
    match searxng::search(http, searxng_url, query, num_results).await {
        Ok(results) if !results.is_empty() => return Ok(results),
        Ok(_) => warn!(%query, "SearXNG returned empty results"),
        Err(e) => warn!(%e, %query, "SearXNG failed"),
    }

    // 3. DuckDuckGo
    duckduckgo::search(http, query, num_results).await
}
