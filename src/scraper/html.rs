use anyhow::Result;
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::debug;

/// Fetch a URL and extract clean readable text from the HTML body.
/// Strips scripts, styles, nav, footer, ads — keeps main content.
pub async fn fetch_and_extract(
    http: &Client,
    url: &str,
    max_chars: usize,
    cookie: Option<&str>,
) -> Result<String> {
    debug!(%url, "fetching page");

    let mut req = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; Researcher/0.1)")
        .timeout(std::time::Duration::from_secs(15));

    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }

    let resp = req.send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} for {}", resp.status(), url);
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if !content_type.contains("html") {
        anyhow::bail!("non-HTML content type: {}", content_type);
    }

    let html = resp.text().await?;
    let text = extract_text(&html, max_chars);

    if text.len() < 100 {
        anyhow::bail!("extracted text too short ({})", text.len());
    }

    Ok(text)
}

fn extract_text(html: &str, max_chars: usize) -> String {
    let document = Html::parse_document(html);

    // Remove noise elements by collecting their text nodes to exclude
    let noise_selector = Selector::parse(
        "script, style, nav, header, footer, aside",
    )
    .unwrap();
    let noise_text: std::collections::HashSet<String> = document
        .select(&noise_selector)
        .flat_map(|el| el.text())
        .map(|t| t.to_string())
        .collect();

    // Prefer semantic content containers
    let content_selector = Selector::parse(
        "article p, main p, [role='main'] p, \
         .content p, .post-content p, .entry-content p, \
         #content p, #main p, \
         article li, main li, \
         article h1, article h2, article h3, \
         main h1, main h2, main h3, \
         p, h1, h2, h3, li",
    )
    .unwrap();

    let mut parts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;

    for el in document.select(&content_selector) {
        let text = el.text().collect::<Vec<_>>().join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

        if text.len() < 20 || seen.contains(&text) || noise_text.contains(&text) {
            continue;
        }
        seen.insert(text.clone());

        let remaining = max_chars.saturating_sub(total);
        if remaining == 0 {
            break;
        }

        let chunk = if text.len() > remaining { &text[..remaining] } else { &text };
        parts.push(chunk.to_string());
        total += chunk.len();
    }

    parts.join("\n")
}
