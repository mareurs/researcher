use anyhow::Result;
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::debug;

/// Enriched extraction result with content quality metadata.
pub struct ExtractedPage {
    pub text: String,
    pub final_url: String,
    pub truncated: bool,
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

    let final_url = resp.url().to_string();
    let html = resp.text().await?;
    let mut page = extract_text(&html, max_chars);
    page.final_url = final_url;

    if page.text.len() < 100 {
        anyhow::bail!("extracted text too short ({})", page.text.len());
    }

    Ok(page)
}

fn extract_text(html: &str, max_chars: usize) -> ExtractedPage {
    let document = Html::parse_document(html);
    // Use the first available semantic root, rather than a selector union whose
    // generic li/p alternatives also scan the entire navigation before main.
    let roots = [
        "main, [role='main']",
        "article",
        ".docblock",
        ".post-content, .entry-content, #content, #main, .content",
    ]
    .iter()
    .find_map(|selector| {
        let selector = Selector::parse(selector).expect("static selector");
        let selected: Vec<_> = document.select(&selector).collect();
        (!selected.is_empty()).then_some(selected)
    });
    let content_selector =
        Selector::parse("p, h1, h2, h3, h4, h5, h6, li, table, pre, blockquote, dt, dd")
            .expect("static selector");
    let mut parts = Vec::new();
    let mut total = 0usize;
    let mut truncated = false;
    let mut has_headings = false;
    let mut has_lists = false;
    let mut has_code_blocks = false;

    for el in document.select(&content_selector) {
        let ancestors: Vec<_> = el
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .collect();
        if ancestors.iter().any(|a| {
            matches!(
                a.value().name(),
                "script" | "style" | "nav" | "header" | "footer" | "aside" | "form"
            )
        }) {
            continue;
        }
        if roots.as_ref().is_some_and(|roots| {
            !roots
                .iter()
                .any(|root| ancestors.iter().any(|a| a.id() == root.id()))
        }) {
            continue;
        }
        // A selected outer block already includes its nested text. Counting both
        // can consume the budget before later safety qualifiers are reached.
        if ancestors.iter().any(|a| {
            matches!(
                a.value().name(),
                "p" | "li" | "table" | "pre" | "blockquote"
            )
        }) {
            continue;
        }
        let text = el.text().collect::<Vec<_>>().join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let separator = usize::from(!parts.is_empty());
        let remaining = max_chars.saturating_sub(total + separator);
        if remaining == 0 {
            truncated = true;
            break;
        }
        match el.value().name() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => has_headings = true,
            "li" => has_lists = true,
            "pre" | "table" | "blockquote" => has_code_blocks = true,
            _ => {}
        }
        let chunk: String = text.chars().take(remaining).collect();
        total += separator + chunk.chars().count();
        truncated = chunk.len() < text.len();
        parts.push(chunk);
        if truncated {
            break;
        }
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
        final_url: String::new(),
        truncated,
        raw_html_len: html.len(),
        link_count,
        ad_link_count,
        has_headings,
        has_lists,
        has_code_blocks,
        paywall_detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_navigation_and_keeps_short_safety_qualifiers() {
        let html = format!(
            "<nav><ul><li>{}</li></ul></nav><main><h2>Cancellation safety</h2>             <p>The following methods are cancellation safe:</p><ul><li>recv</li></ul>             <p>The following methods are not cancellation safe:</p>             <ul><li>read_exact</li><li>write_all</li></ul></main>",
            "unrelated navigation ".repeat(100)
        );
        let page = extract_text(&html, 800);
        assert!(!page.text.contains("navigation"), "{}", page.text);
        for expected in [
            "Cancellation safety",
            "recv",
            "not cancellation safe",
            "read_exact",
            "write_all",
        ] {
            assert!(
                page.text.contains(expected),
                "missing {expected}: {}",
                page.text
            );
        }
    }

    #[test]
    fn drops_noise_descendants_even_when_their_joined_text_is_long() {
        let html = "<main><aside><p>Cookie settings <b>and other irrelevant banners</b></p></aside>             <h2>Limits</h2><p>Only read and write, not read_exact or write_all, are safe here.</p></main>";
        let page = extract_text(html, 800);
        assert!(!page.text.contains("Cookie"), "{}", page.text);
        assert!(page.text.contains("not read_exact or write_all"));
        assert!(page.text.contains("Limits"));
    }

    #[test]
    fn max_chars_is_a_unicode_character_budget() {
        let page = extract_text("<main><p>șțâîă漢字abcdefghijk</p></main>", 8);
        assert_eq!(page.text, "șțâîă漢字a");
    }
}
