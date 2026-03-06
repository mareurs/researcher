use anyhow::Result;
use futures::future::join_all;
use reqwest::Client;
use tracing::{debug, warn};

use crate::config::Config;
use crate::scraper::html::fetch_and_extract;


#[derive(Debug, Clone)]
pub struct ScrapedSource {
    pub url: String,
    pub title: String,
    pub query: String,
    pub content: String,
}

/// For a single sub-question: search → deduplicate URLs → scrape in parallel.
pub async fn crawl_query(
    http: &Client,
    cfg: &Config,
    query: &str,
    visited_urls: &mut std::collections::HashSet<String>,
) -> Result<Vec<ScrapedSource>> {
    // 1. Search (SearXNG with DuckDuckGo fallback)
    let results = crate::search::search_with_fallback(
        http,
        &cfg.searxng_url,
        query,
        cfg.search_results_per_query,
    )
    .await
    .unwrap_or_else(|e| {
        warn!(%e, %query, "all search backends failed");
        vec![]
    });

    // 2. Deduplicate against globally visited URLs
    let fresh: Vec<_> = results
        .into_iter()
        .filter(|r| {
            let is_new = !visited_urls.contains(&r.url);
            if is_new {
                visited_urls.insert(r.url.clone());
            }
            is_new
        })
        .take(cfg.max_sources_per_query)
        .collect();

    debug!(count = fresh.len(), %query, "scraping URLs");

    // 3. Scrape all fresh URLs concurrently
    let futs = fresh.iter().map(|result| {
        let http = http.clone();
        let url = result.url.clone();
        let max_chars = cfg.max_page_chars;
        let cookie: Option<String> = {
            let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
            cfg.auth.cookie_for_host(host).map(|c| c.to_string())
        };
        async move { fetch_and_extract(&http, &url, max_chars, cookie.as_deref()).await }
    });

    let scraped = join_all(futs).await;

    let sources = fresh
        .into_iter()
        .zip(scraped)
        .filter_map(|(result, content)| match content {
            Ok(text) => Some(ScrapedSource {
                url: result.url,
                title: result.title,
                query: query.to_string(),
                content: text,
            }),
            Err(e) => {
                warn!(%e, url = %result.url, "scrape failed");
                None
            }
        })
        .collect();

    Ok(sources)
}

/// Run all sub-queries in parallel, collecting deduplicated sources.
pub async fn crawl_all(
    http: &Client,
    cfg: &Config,
    queries: &[String],
) -> Vec<ScrapedSource> {
    let mut visited = std::collections::HashSet::new();
    let mut all_sources = Vec::new();

    // We run queries sequentially here to share the visited_urls deduplication.
    // Within each query, URL fetches are concurrent (join_all above).
    for query in queries {
        match crawl_query(http, cfg, query, &mut visited).await {
            Ok(sources) => all_sources.extend(sources),
            Err(e) => warn!(%e, %query, "crawl_query failed"),
        }
    }

    all_sources
}
