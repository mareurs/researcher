use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    #[allow(dead_code)]
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

/// Query SearXNG's JSON API. SearXNG acts as a privacy-preserving meta-search
/// engine — it fans out to Google, Bing, DuckDuckGo, etc. without API keys.
pub async fn search(
    http: &Client,
    searxng_url: &str,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    let url = format!("{}/search", searxng_url.trim_end_matches('/'));

    debug!(%query, "SearXNG search");

    let resp = http
        .get(&url)
        .query(&[
            ("q", query),
            ("format", "json"),
            ("language", "en"),
            ("categories", "general"),
        ])
        .send()
        .await
        .context("SearXNG request")?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("SearXNG error {status}");
    }

    let body: SearxngResponse = resp.json().await.context("SearXNG JSON parse")?;

    let results = body
        .results
        .into_iter()
        .filter(|r| !is_non_english_domain(&r.url))
        .take(num_results)
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.content,
        })
        .collect();

    Ok(results)
}

/// Filter out domains that predominantly host non-English content.
fn is_non_english_domain(url: &str) -> bool {
    // Extract hostname (without www. prefix)
    let after_scheme = url.split("://").nth(1).unwrap_or("");
    let host = after_scheme.split('/').next().unwrap_or("").trim_start_matches("www.");
    let hostname = host.split(':').next().unwrap_or(host); // strip port

    // Explicit non-English domain blocklist
    const NON_ENGLISH_DOMAINS: &[&str] = &[
        // Chinese
        "zhihu.com", "baidu.com", "csdn.net", "cnblogs.com",
        "163.com", "sina.com.cn", "weibo.com", "bilibili.com",
        "juejin.cn", "segmentfault.com", "tieba.baidu.com",
        // French
        "commentcamarche.net", "lesnumeriques.com", "clubic.com", "01net.com",
    ];
    if NON_ENGLISH_DOMAINS.iter().any(|d| hostname == *d || hostname.ends_with(&format!(".{d}"))) {
        return true;
    }

    // Country-code TLDs that consistently produce non-English results.
    // Does NOT include .io/.ai/.co/.me/.tv/.app which are used extensively for English sites.
    const NON_ENGLISH_TLDS: &[&str] = &[
        ".fr", ".de", ".ru", ".jp", ".cn", ".it", ".es", ".pt",
        ".nl", ".pl", ".cz", ".ro", ".bg", ".hu", ".sk", ".hr",
        ".si", ".dk", ".se", ".no", ".fi", ".at", ".be",
        ".gr", ".tr", ".ua", ".by", ".kz",
    ];
    if NON_ENGLISH_TLDS.iter().any(|tld| hostname.ends_with(tld)) {
        return true;
    }

    // URL path language prefixes — e.g. chatgpt.org/fr?... or example.com/de/docs
    let path = after_scheme.splitn(2, '/').nth(1).unwrap_or("");
    // first path segment before / or ?
    let first_seg = path.split('/').next().unwrap_or("").split('?').next().unwrap_or("");
    const PATH_LANG_CODES: &[&str] = &[
        "fr", "de", "es", "it", "pt", "nl", "pl",
        "ru", "ja", "zh", "ko", "ar", "tr", "sv", "da", "fi", "no",
    ];
    PATH_LANG_CODES.iter().any(|code| first_seg == *code)
}
