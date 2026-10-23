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
/// Remove `site:` operators (and adjacent OR/AND) from a query string.
/// Used before sending to Tavily/Exa, which accept native `include_domains` params.
fn strip_site_operators(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|t| !t.starts_with("site:") && *t != "OR" && *t != "AND")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Match a parsed hostname on a label boundary; URL text and paths are not hosts.
pub fn domain_allowed(url: &str, domains: &[String]) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    domains.is_empty()
        || domains.iter().any(|domain| {
            let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
        })
}

#[allow(clippy::too_many_arguments)]
pub async fn search_with_fallback(
    http: &Client,
    searxng_url: &str,
    brave_api_key: &str,
    tavily_api_key: &str,
    exa_api_key: &str,
    domains: &[String],
    domain_profile: Option<&str>,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    // Tavily/Exa use native domain params — strip site: operators from query for them.
    // Brave/SearXNG/DuckDuckGo understand site: in the query string directly.
    let clean_query = strip_site_operators(query);
    let restrict = |results: Vec<SearchResult>| {
        results
            .into_iter()
            .filter(|r| domain_allowed(&r.url, domains))
            .collect::<Vec<_>>()
    };

    // 1. Profile-selected primary backend
    match domain_profile {
        Some("news") if !tavily_api_key.is_empty() => {
            match tavily::search(http, tavily_api_key, domains, &clean_query, num_results).await {
                Ok(results) => {
                    let results = restrict(results);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                    warn!(%query, "backend returned no results within requested domains");
                }

                Err(e) => warn!(%e, %query, "Tavily failed, falling back"),
            }
        }
        Some("academic") if !exa_api_key.is_empty() => {
            match exa::search(http, exa_api_key, domains, &clean_query, num_results).await {
                Ok(results) => {
                    let results = restrict(results);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                    warn!(%query, "backend returned no results within requested domains");
                }

                Err(e) => warn!(%e, %query, "Exa failed, falling back"),
            }
        }
        _ if !brave_api_key.is_empty() => {
            match brave::search(http, brave_api_key, query, num_results).await {
                Ok(results) => {
                    let results = restrict(results);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                    warn!(%query, "backend returned no results within requested domains");
                }

                Err(e) => warn!(%e, %query, "Brave failed, falling back"),
            }
        }
        _ => {}
    }

    // 2. SearXNG
    match searxng::search(http, searxng_url, query, num_results, domains).await {
        Ok(results) => {
            let results = restrict(results);
            if !results.is_empty() {
                return Ok(results);
            }
            warn!(%query, "backend returned no results within requested domains");
        }

        Err(e) => warn!(%e, %query, "SearXNG failed"),
    }

    // 3. DuckDuckGo
    duckduckgo::search(http, query, num_results)
        .await
        .map(restrict)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_domains_keep_romanian_results_and_reject_lookalikes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().fallback(|| async {
            axum::Json(serde_json::json!({"results":[
                {"title":"Romanian shop","url":"https://www.altex.ro/product","content":"requested product price"},
                {"title":"Lookalike","url":"https://altex.ro.evil.test/product","content":"off-domain"},
                {"title":"Unrelated","url":"https://other.example/product","content":"off-domain"}
            ]}))
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let results = search_with_fallback(
            &Client::new(),
            &format!("http://{address}"),
            "",
            "",
            "",
            &["altex.ro".into()],
            None,
            "mouse site:altex.ro",
            10,
        )
        .await
        .unwrap();
        server.abort();
        assert_eq!(
            results.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
            vec!["https://www.altex.ro/product"]
        );
    }
}
