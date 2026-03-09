use anyhow::Result;
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::debug;

/// Enriched extraction result with content quality metadata.
pub struct ExtractedPage {
    pub text: String,
    pub raw_html_len: usize,
    pub link_count: usize,
    pub ad_link_count: usize,
    pub has_headings: bool,
    pub has_lists: bool,
    pub has_code_blocks: bool,
    pub paywall_detected: bool,
}


/// Fetch a URL and extract clean readable text from the HTML body.
/// Strips scripts, styles, nav, footer, ads — keeps main content.
pub async fn fetch_and_extract(
    http: &Client,
    url: &str,
    max_chars: usize,
    cookie: Option<&str>,
) -> Result<ExtractedPage> {
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
    let page = extract_text(&html, max_chars);

    if page.text.len() < 100 {
        anyhow::bail!("extracted text too short ({})", page.text.len());
    }

    Ok(page)
}

fn extract_text(html: &str, max_chars: usize) -> ExtractedPage {
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

    // Prefer semantic content containers (includes table, pre, blockquote)
    let content_selector = Selector::parse(
        "article p, main p, [role='main'] p, \
         .content p, .post-content p, .entry-content p, \
         #content p, #main p, \
         article li, main li, \
         article h1, article h2, article h3, \
         main h1, main h2, main h3, \
         p, h1, h2, h3, li, table, pre, blockquote",
    )
    .unwrap();

    let mut parts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;
    let mut has_headings = false;
    let mut has_lists = false;
    let mut has_code_blocks = false;

    for el in document.select(&content_selector) {
        let tag = el.value().name();
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => has_headings = true,
            "li" => has_lists = true,
            "pre" | "code" => has_code_blocks = true,
            "table" | "blockquote" => has_code_blocks = true,
            _ => {}
        }

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

    // Count links and detect ad links
    let a_selector = Selector::parse("a[href]").unwrap();
    let mut link_count = 0usize;
    let mut ad_link_count = 0usize;
    let ad_domains = [
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "amazon-adsystem.com",
        "adnxs.com",
        "outbrain.com",
        "taboola.com",
    ];

    for el in document.select(&a_selector) {
        if let Some(href) = el.value().attr("href") {
            link_count += 1;
            if ad_domains.iter().any(|ad| href.contains(ad)) {
                ad_link_count += 1;
            }
        }
    }

    // Paywall detection
    let html_lower = html.to_lowercase();
    let paywall_patterns = [
        "subscribe to continue",
        "subscribe to read",
        "sign in to read",
        "sign in to continue",
        "premium content",
        "paywall",
        "tp-modal",
        "pw-overlay",
        "regwall",
        "metered-content",
    ];
    let paywall_detected = paywall_patterns.iter().any(|p| html_lower.contains(p));

    ExtractedPage {
        text: parts.join("\n"),
        raw_html_len: html.len(),
        link_count,
        ad_link_count,
        has_headings,
        has_lists,
        has_code_blocks,
        paywall_detected,
    }
}
