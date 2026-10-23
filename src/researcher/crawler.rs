use anyhow::Result;
use futures::future::join_all;
use reqwest::Client;
use tracing::{debug, warn};

use crate::config::Config;
use crate::scraper::html::fetch_and_extract;

/// Retrieval provenance describes the observed snapshot, not the page's publication time.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SourceProvenance {
    pub origin: SourceOrigin,
    pub retrieved_at: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    FetchedPage,
    SearchSnippet,
    ProviderExtract,
    #[default]
    Unknown,
}

impl SourceProvenance {
    pub fn observed(origin: SourceOrigin, truncated: bool) -> Self {
        Self {
            origin,
            retrieved_at: chrono::Utc::now().to_rfc3339(),
            truncated,
        }
    }
    pub fn description(&self) -> String {
        let origin = serde_json::to_value(self.origin).unwrap();
        format!("Origin: {}\nRetrieved at: {} (observation time, not publication time)\nTruncated: {}\nFreshness: {}",
            origin.as_str().unwrap(), self.retrieved_at, self.truncated,
            match self.origin {
                SourceOrigin::SearchSnippet => "A search snippet does not verify current price or stock.",
                SourceOrigin::ProviderExtract => "Provider extraction has unverified cache age; current price or stock is unverified.",
                SourceOrigin::FetchedPage => "Fetched page snapshot; only explicit quoted price/stock wording is evidence at retrieval time.",
                SourceOrigin::Unknown => "Unknown retrieval origin and freshness; current price or stock is unverified.",
            })
    }
}

#[derive(Debug, Clone)]
pub struct ScrapedSource {
    pub url: String,
    pub title: String,
    pub query: String,
    pub content: String,
    pub provenance: SourceProvenance,
    pub domain: String,
    pub word_count: usize,
    pub raw_html_len: usize,
    pub link_count: usize,
    pub ad_link_count: usize,
    pub has_headings: bool,
    pub has_lists: bool,
    pub has_code_blocks: bool,
    pub paywall_detected: bool,
}

fn domain_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

/// Domains that consistently return off-topic results and should never be scraped.
/// e.g. "Rust" the game showing up in Rust programming language searches.
const DOMAIN_BLACKLIST: &[&str] = &[
    // Games / off-topic
    "rust.facepunch.com",
    "store.steampowered.com",
    "city-data.com",
    "facepunch.com",
    // AI chat front-ends (not research content)
    "chatgpt.com",
    "chatgpt.org",
    "claude.ai",
    "gemini.google.com",
    "copilot.microsoft.com",
    "character.ai",
    // Google utility pages (no research content)
    "support.google.com",
    "accounts.google.com",
    "translate.google.com",
    "policies.google.com",
    // English-grammar Q&A / language forums (noise for queries with common words)
    "ell.stackexchange.com",
    "wordreference.com",
    // Retail / unrelated sites that match on generic words ("running", "best", etc.)
    "rei.com",
    "amazon.com",
    "ebay.com",
    // Education platforms that surface on queries with words like "benchmarks", "top"
    "tophat.com",
    "coursera.org",
    "udemy.com",
    "khanacademy.org",
];

/// For a single sub-question: search → deduplicate URLs → scrape in parallel.
pub async fn crawl_query(
    http: &Client,
    cfg: &Config,
    query: &str,
    visited_urls: &mut std::collections::HashSet<String>,
) -> Result<Vec<ScrapedSource>> {
    // 1. Search — resolve effective domain list for native API filtering (Tavily/Exa)
    let mut domains: Vec<String> = cfg
        .domain_profile
        .as_deref()
        .and_then(|p| cfg.profiles.get(p))
        .cloned()
        .unwrap_or_default();
    for d in &cfg.cli_domains {
        if !domains.contains(d) {
            domains.push(d.clone());
        }
    }

    let results = crate::search::search_with_fallback(
        http,
        &cfg.searxng_url,
        &cfg.brave_api_key,
        &cfg.tavily_api_key,
        &cfg.exa_api_key,
        &domains,
        cfg.domain_profile.as_deref(),
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
            let host = domain_from_url(&r.url);
            if DOMAIN_BLACKLIST
                .iter()
                .any(|b| host == *b || host.ends_with(&format!(".{b}")))
            {
                debug!(url = %r.url, "skipping blacklisted domain");
                return false;
            }
            let is_new = !visited_urls.contains(&r.url);
            if is_new {
                visited_urls.insert(r.url.clone());
            }
            is_new
        })
        .take(cfg.max_sources_per_query)
        .collect();

    debug!(count = fresh.len(), %query, "scraping URLs");

    // 3. Partition: pre-extracted content (Tavily/Exa) vs needs scraping
    let (pre_extracted, needs_scraping): (Vec<_>, Vec<_>) = fresh
        .into_iter()
        .partition(|r| r.content.as_deref().is_some_and(|c| !c.is_empty()));

    // Build ScrapedSource for pre-extracted results — no HTTP fetch needed
    let mut sources: Vec<ScrapedSource> = pre_extracted
        .into_iter()
        .map(|result| {
            let truncated = result.content.as_ref().unwrap().chars().count() > cfg.max_page_chars;
            let content: String = result
                .content
                .unwrap()
                .chars()
                .take(cfg.max_page_chars)
                .collect();
            let domain = domain_from_url(&result.url);
            let word_count = content.split_whitespace().count();
            ScrapedSource {
                url: result.url,
                title: result.title,
                query: query.to_string(),
                domain,
                word_count,
                raw_html_len: 0,
                link_count: 0,
                ad_link_count: 0,
                has_headings: false,
                has_lists: false,
                has_code_blocks: false,
                paywall_detected: false,
                content,
                provenance: SourceProvenance::observed(SourceOrigin::ProviderExtract, truncated),
            }
        })
        .collect();

    // Scrape remaining results concurrently
    let futs = needs_scraping.iter().map(|result| {
        let http = http.clone();
        let url = result.url.clone();
        let max_chars = cfg.max_page_chars;
        let cookie: Option<String> = {
            let host = url
                .split("://")
                .nth(1)
                .unwrap_or("")
                .split('/')
                .next()
                .unwrap_or("");
            cfg.auth.cookie_for_host(host).map(|c| c.to_string())
        };
        async move { fetch_and_extract(&http, &url, max_chars, cookie.as_deref()).await }
    });

    let scraped = join_all(futs).await;

    sources.extend(needs_scraping.into_iter().zip(scraped).map(
        |(result, content)| match content {
            Ok(page) => {
                let domain = domain_from_url(&page.final_url);
                let word_count = page.text.split_whitespace().count();
                ScrapedSource {
                    url: page.final_url,
                    title: result.title,
                    query: query.to_string(),
                    domain,
                    word_count,
                    raw_html_len: page.raw_html_len,
                    link_count: page.link_count,
                    ad_link_count: page.ad_link_count,
                    has_headings: page.has_headings,
                    has_lists: page.has_lists,
                    has_code_blocks: page.has_code_blocks,
                    paywall_detected: page.paywall_detected,
                    content: page.text,
                    provenance: SourceProvenance::observed(
                        SourceOrigin::FetchedPage,
                        page.truncated,
                    ),
                }
            }
            Err(e) => {
                warn!(%e, url = %result.url, "scrape failed, using snippet");
                let domain = domain_from_url(&result.url);
                let word_count = result.snippet.split_whitespace().count();
                ScrapedSource {
                    url: result.url,
                    title: result.title,
                    query: query.to_string(),
                    domain,
                    word_count,
                    raw_html_len: 0,
                    link_count: 0,
                    ad_link_count: 0,
                    has_headings: false,
                    has_lists: false,
                    has_code_blocks: false,
                    paywall_detected: false,
                    content: result.snippet,
                    provenance: SourceProvenance::observed(SourceOrigin::SearchSnippet, false),
                }
            }
        },
    ));

    let mut final_urls = std::collections::HashSet::new();
    sources.retain(|source| {
        crate::search::domain_allowed(&source.url, &domains)
            && !DOMAIN_BLACKLIST.iter().any(|blocked| {
                source.domain == *blocked || source.domain.ends_with(&format!(".{blocked}"))
            })
            && final_urls.insert(source.url.clone())
    });
    Ok(sources)
}

/// Run all sub-queries in parallel, collecting deduplicated sources.
pub async fn crawl_all(http: &Client, cfg: &Config, queries: &[String]) -> Vec<ScrapedSource> {
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
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[tokio::test]
    async fn failed_fetch_is_a_search_snippet_and_success_is_a_fetched_snapshot() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base = format!("http://{address}");
        let search_base = base.clone();
        let app = axum::Router::new()
            .route("/search", axum::routing::get(move || {
                let base = search_base.clone();
                async move { axum::Json(serde_json::json!({"results":[
                    {"title":"Page","url":format!("{base}/page"),"content":"search snippet for fetched page"},
                    {"title":"Missing","url":format!("{base}/missing"),"content":"Product price 374.99 RON in a cached search snippet only."}
                ]})) }
            }))
            .route("/page", axum::routing::get(|| async { axum::response::Html(
                "<main><p>Fetched page content with enough readable text to pass extraction. The full page is observed directly rather than inferred from the search snippet returned by an index.</p></main>") }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut cfg = Config::parse_from(["researcher"]);
        cfg.searxng_url = base;
        cfg.brave_api_key.clear();
        cfg.tavily_api_key.clear();
        cfg.exa_api_key.clear();
        cfg.cli_domains = vec!["127.0.0.1".into()];
        cfg.domain_profile = None;
        let sources = crawl_query(
            &Client::new(),
            &cfg,
            "product price",
            &mut Default::default(),
        )
        .await
        .unwrap();
        server.abort();
        assert_eq!(sources.len(), 2);
        let page = sources.iter().find(|s| s.url.ends_with("/page")).unwrap();
        let snippet = sources
            .iter()
            .find(|s| s.url.ends_with("/missing"))
            .unwrap();
        assert_eq!(page.provenance.origin, SourceOrigin::FetchedPage);
        assert_eq!(snippet.provenance.origin, SourceOrigin::SearchSnippet);
        assert!(snippet.content.contains("cached search snippet"));
        assert!(!page.provenance.retrieved_at.is_empty());
        assert!(!snippet.provenance.retrieved_at.is_empty());
    }
    #[tokio::test]
    async fn redirects_use_final_source_urls_and_cannot_escape_requested_domains() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let base = format!("http://{address}");
        let search_base = base.clone();
        let foreign = format!("http://localhost:{}/page", address.port());
        let app = axum::Router::new()
            .route("/search", axum::routing::get(move || {
                let base = search_base.clone();
                async move { axum::Json(serde_json::json!({"results":[
                    {"title":"Moved","url":format!("{base}/moved"),"content":"search result"},
                    {"title":"Off domain redirect","url":format!("{base}/escape"),"content":"search result"}
                ]})) }
            }))
            .route("/moved", axum::routing::get(|| async { axum::response::Redirect::temporary("/page") }))
            .route("/escape", axum::routing::get(move || {
                let foreign = foreign.clone();
                async move { axum::response::Redirect::temporary(&foreign) }
            }))
            .route("/page", axum::routing::get(|| async { axum::response::Html(
                "<main><p>Fetched page content with enough readable text to pass extraction. This is the final destination of the redirect and must be attributed to its actual URL and host.</p></main>") }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut cfg = Config::parse_from(["researcher"]);
        cfg.searxng_url = base;
        cfg.brave_api_key.clear();
        cfg.tavily_api_key.clear();
        cfg.exa_api_key.clear();
        cfg.cli_domains = vec!["127.0.0.1".into()];
        cfg.domain_profile = None;
        let sources = crawl_query(
            &Client::new(),
            &cfg,
            "source redirect",
            &mut Default::default(),
        )
        .await
        .unwrap();
        server.abort();
        assert_eq!(
            sources.len(),
            1,
            "off-domain final URL must not supply page evidence"
        );
        assert!(sources[0].url.ends_with("/page"));
        assert_eq!(sources[0].domain, "127.0.0.1");
    }
}
