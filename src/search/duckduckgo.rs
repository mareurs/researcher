use anyhow::Result;
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::debug;

use super::searxng::SearchResult;

/// DuckDuckGo Lite — no JS, no API key, scrapes the lite HTML endpoint.
/// Used as a fallback when SearXNG is unavailable.
pub async fn search(
    http: &Client,
    query: &str,
    num_results: usize,
) -> Result<Vec<SearchResult>> {
    debug!(%query, "DuckDuckGo lite search");

    let resp = http
        .post("https://lite.duckduckgo.com/lite/")
        .header("User-Agent", "Mozilla/5.0 (compatible; Researcher/0.1)")
        .form(&[("q", query)])
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("DDG error {}", resp.status());
    }

    let html = resp.text().await?;
    let document = Html::parse_document(&html);

    // DDG lite result structure: <a class="result-link"> titles, <td class="result-snippet"> snippets
    let link_sel = Selector::parse("a.result-link").unwrap();
    let snippet_sel = Selector::parse("td.result-snippet").unwrap();

    let links: Vec<_> = document.select(&link_sel).collect();
    let snippets: Vec<_> = document.select(&snippet_sel).collect();

    let results = links
        .into_iter()
        .zip(snippets.into_iter())
        .filter_map(|(link, snippet)| {
            let url = link.value().attr("href")?.to_string();
            // Skip DuckDuckGo internal links
            if url.starts_with("//duckduckgo") || url.starts_with("javascript") {
                return None;
            }
            let title = link.text().collect::<String>().trim().to_string();
            let snippet_text = snippet.text().collect::<String>().trim().to_string();
            Some(SearchResult {
                title,
                url,
                snippet: snippet_text,
                content: None,
            })
        })
        .take(num_results)
        .collect();

    Ok(results)
}
