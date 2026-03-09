# Ranking Improvements Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the minimal bi-encoder-only ranking with a 4-layer system: content quality heuristics, bi-encoder dedup (wider window), cross-encoder rerank, and LLM relevance judge.

**Architecture:** New stages slot into the existing `run()` pipeline between crawl and summarize. Two new modules (`quality.rs`, `reranker.rs`), enriched scraper output, and structured JSON from the summarizer. Everything degrades gracefully when services aren't configured.

**Tech Stack:** Rust, TEI (existing bi-encoder + new cross-encoder), reqwest, serde_json for structured LLM output.

**Note:** This project has no test suite. Verification is `cargo check` then `cargo build --release` per project conventions.

---

### Task 1: Enrich the HTML Scraper — `ExtractedPage` Struct

**Files:**
- Modify: `src/scraper/html.rs:8-50` (`fetch_and_extract`) and `src/scraper/html.rs:52-102` (`extract_text`)

**Context:** Currently `fetch_and_extract` returns `Result<String>` and `extract_text` returns `String`. We need metadata from the HTML parsing pass for quality filtering downstream. The key insight: `extract_text` already walks the DOM — we piggyback on that traversal to collect metadata at near-zero cost.

**Step 1: Add the `ExtractedPage` struct**

Add above `fetch_and_extract` (after imports):

```rust
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
```

**Step 2: Update `extract_text` to return `ExtractedPage`**

Change signature from `fn(html: &str, max_chars: usize) -> String` to `fn(html: &str, max_chars: usize) -> ExtractedPage`.

Key changes inside:
- Add `<table>`, `<pre>`, `<blockquote>` to the `content_selector` CSS (currently missing — drops tabular/code content)
- Track `has_headings`, `has_lists`, `has_code_blocks` booleans during the selector walk
- After the main loop, count links and detect paywalls:

```rust
// Count links
let a_selector = Selector::parse("a[href]").unwrap();
let mut link_count = 0usize;
let mut ad_link_count = 0usize;
let ad_domains = ["doubleclick.net", "googlesyndication.com", "googleadservices.com",
    "amazon-adsystem.com", "adnxs.com", "outbrain.com", "taboola.com"];

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
let paywall_patterns = ["subscribe to continue", "subscribe to read",
    "sign in to read", "sign in to continue", "premium content",
    "paywall", "tp-modal", "pw-overlay", "regwall", "metered-content"];
let paywall_detected = paywall_patterns.iter().any(|p| html_lower.contains(p));
```

Return `ExtractedPage` with all collected fields.

**Step 3: Update `fetch_and_extract` to return `ExtractedPage`**

Change return type from `Result<String>` to `Result<ExtractedPage>`. Pass `html.len()` as `raw_html_len`. The min-length check (`text.len() < 100`) now checks `page.text.len()`.

**Step 4: Verify**

```bash
cargo check
```

**Step 5: Commit**

```bash
git add src/scraper/html.rs
git commit -m "feat: enrich scraper with ExtractedPage metadata

Return structured ExtractedPage from extract_text/fetch_and_extract with
quality signals: link counts, paywall detection, structural elements.
Add table/pre/blockquote selectors for tech content."
```

---

### Task 2: Enrich `ScrapedSource` and Update Crawler

**Files:**
- Modify: `src/researcher/crawler.rs:11-16` (`ScrapedSource`), `src/researcher/crawler.rs:19-97` (`crawl_query`)

**Context:** `ScrapedSource` is the data structure that flows through the entire pipeline. Currently it's just `{url, title, query, content}`. We need `domain`, `word_count`, and a slot for quality metadata. `crawl_query` constructs `ScrapedSource` values — it needs to populate the new fields from `ExtractedPage`.

**Step 1: Add fields to `ScrapedSource`**

```rust
#[derive(Debug, Clone)]
pub struct ScrapedSource {
    pub url: String,
    pub title: String,
    pub query: String,
    pub content: String,
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
```

**Step 2: Add a helper to extract domain from URL**

```rust
fn domain_from_url(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase()
}
```

**Step 3: Update `crawl_query` to use `ExtractedPage`**

The closure that calls `fetch_and_extract` currently returns `Result<String>`. Change it to handle `ExtractedPage`. Update both the `Ok(text)` and snippet fallback arms in the `filter_map`:

For the `Ok` arm — construct `ScrapedSource` from `ExtractedPage` fields:
```rust
Ok(page) => Some(ScrapedSource {
    url: result.url,
    title: result.title,
    query: query.to_string(),
    domain: domain_from_url(&result.url),
    word_count: page.text.split_whitespace().count(),
    raw_html_len: page.raw_html_len,
    link_count: page.link_count,
    ad_link_count: page.ad_link_count,
    has_headings: page.has_headings,
    has_lists: page.has_lists,
    has_code_blocks: page.has_code_blocks,
    paywall_detected: page.paywall_detected,
    content: page.text,
}),
```

For the snippet fallback arm — use sensible defaults (no HTML metadata available):
```rust
Some(ScrapedSource {
    url: result.url,
    title: result.title,
    query: query.to_string(),
    content: result.snippet,
    domain: domain_from_url(&result.url),
    word_count: result.snippet.split_whitespace().count(),
    raw_html_len: 0,
    link_count: 0,
    ad_link_count: 0,
    has_headings: false,
    has_lists: false,
    has_code_blocks: false,
    paywall_detected: false,
})
```

**Step 4: Verify**

```bash
cargo check
```

Expected: errors in `pipeline.rs` and `dedup.rs` where `ScrapedSource` is constructed or pattern-matched — that's fine, we fix those in later tasks.

**Step 5: Commit**

```bash
git add src/researcher/crawler.rs
git commit -m "feat: enrich ScrapedSource with domain, word count, quality metadata

Add domain extraction, word count, and HTML quality signals (link counts,
paywall detection, structural elements) to ScrapedSource for downstream
quality filtering and ranking."
```

---

### Task 3: Content Quality Filter Module

**Files:**
- Create: `src/researcher/quality.rs`
- Modify: `src/researcher/mod.rs` (add `pub mod quality;`)

**Context:** This is a new module that scores and filters sources based on heuristic quality signals. It runs after crawl, before any model calls — so bad sources never waste embedding or LLM tokens.

**Step 1: Create `src/researcher/quality.rs`**

```rust
use tracing::debug;

use super::crawler::ScrapedSource;
use crate::config::Config;
use crate::researcher::pipeline::ResearchTarget;

/// Quality assessment for a scraped source.
#[derive(Debug, Clone)]
pub struct ContentQuality {
    pub text_density: f32,
    pub ad_link_ratio: f32,
    pub has_structure: bool,
    pub domain_authority: f32,
}

/// Domain authority tiers.
const TIER1_DOMAINS: &[&str] = &[
    "wikipedia.org", "github.com", "arxiv.org", "docs.rs",
    "doc.rust-lang.org", "developer.mozilla.org", "w3.org",
];
const TIER2_DOMAINS: &[&str] = &[
    "nytimes.com", "bbc.com", "reuters.com", "bloomberg.com",
    "techcrunch.com", "arstechnica.com", "nature.com", "stackoverflow.com",
];
const TIER3_DOMAINS: &[&str] = &[
    "reddit.com", "medium.com", "quora.com", "dev.to",
    "hackernoon.com", "substack.com",
];

/// Target-specific authority boosts. Returns additional tier-1 domains
/// for the given research target.
fn target_authority_domains(target: &ResearchTarget) -> &'static [&'static str] {
    match target {
        ResearchTarget::Person(_) => &[
            "linkedin.com", "twitter.com", "x.com", "crunchbase.com",
        ],
        ResearchTarget::Company(_) => &[
            "linkedin.com", "crunchbase.com", "glassdoor.com",
            "trustpilot.com", "bloomberg.com",
        ],
        ResearchTarget::Code { .. } => &[
            "docs.rs", "crates.io", "npmjs.com", "pypi.org",
            "pkg.go.dev",
        ],
        ResearchTarget::General => &[],
    }
}

fn domain_authority(domain: &str, target: &ResearchTarget) -> f32 {
    let target_domains = target_authority_domains(target);
    if target_domains.iter().any(|d| domain.contains(d)) {
        return 1.0;
    }
    if TIER1_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 1.0;
    }
    if TIER2_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 0.7;
    }
    if TIER3_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 0.5;
    }
    0.3
}

/// Compute quality assessment for a source.
pub fn assess_quality(source: &ScrapedSource, target: &ResearchTarget) -> ContentQuality {
    let text_density = if source.raw_html_len > 0 {
        source.content.len() as f32 / source.raw_html_len as f32
    } else {
        0.5 // snippet fallback — no HTML available, assume decent
    };

    let ad_link_ratio = if source.link_count > 0 {
        source.ad_link_count as f32 / source.link_count as f32
    } else {
        0.0
    };

    let has_structure = source.has_headings || source.has_lists || source.has_code_blocks;

    ContentQuality {
        text_density,
        ad_link_ratio,
        has_structure,
        domain_authority: domain_authority(&source.domain, target),
    }
}

/// Filter sources by content quality heuristics. Returns sources that pass
/// minimum thresholds, each annotated with a quality score.
pub fn filter_sources(
    sources: Vec<ScrapedSource>,
    target: &ResearchTarget,
    cfg: &Config,
) -> Vec<(ScrapedSource, ContentQuality)> {
    let before = sources.len();

    let result: Vec<(ScrapedSource, ContentQuality)> = sources
        .into_iter()
        .filter_map(|source| {
            let quality = assess_quality(&source, target);

            // Hard filters
            if source.word_count < cfg.min_content_words {
                debug!(url = %source.url, words = source.word_count, "quality: dropping thin content");
                return None;
            }
            if source.paywall_detected {
                debug!(url = %source.url, "quality: dropping paywalled content");
                return None;
            }
            if quality.text_density < cfg.min_text_density {
                debug!(url = %source.url, density = quality.text_density, "quality: dropping low-density page");
                return None;
            }

            Some((source, quality))
        })
        .collect();

    debug!(before, after = result.len(), "quality filter complete");
    result
}

/// Compute a normalized quality score (0.0-1.0) from quality signals.
pub fn quality_score(q: &ContentQuality) -> f32 {
    let mut score = 0.5_f32; // baseline

    // Structure bonus
    if q.has_structure {
        score += 0.2;
    }

    // Ad penalty
    score -= q.ad_link_ratio * 0.3;

    // Density bonus (clamped)
    score += (q.text_density.min(0.5) / 0.5) * 0.1;

    score.clamp(0.0, 1.0)
}
```

**Step 2: Add module to `src/researcher/mod.rs`**

Add `pub mod quality;` after the existing module declarations.

**Step 3: Verify**

```bash
cargo check
```

Expected: may still have errors from `ScrapedSource` field changes in other files — that's fine, addressed in later tasks.

**Step 4: Commit**

```bash
git add src/researcher/quality.rs src/researcher/mod.rs
git commit -m "feat: add content quality filter module

Heuristic quality scoring: word count floor, paywall detection, text
density, ad-link ratio, domain authority tiers with target-specific
boosts (person/company/code research)."
```

---

### Task 4: Config Changes

**Files:**
- Modify: `src/config.rs:32-129` (`Config` struct)
- Modify: `src/mcp_server.rs:417-469` (`config_from_env`)

**Context:** New config fields for reranker URL, score weights, and quality thresholds. Per project rules, both `Config` and `config_from_env()` must be updated in sync.

**Step 1: Add fields to `Config`**

After the existing `dedup_threshold` field (around line 86), add:

```rust
    // ── Reranker ─────────────────────────────────────────────────────────────
    /// TEI cross-encoder reranker base URL (empty = disable reranking)
    #[arg(long, env = "RERANK_BASE_URL", default_value = "")]
    pub rerank_base_url: String,

    /// Weight for cross-encoder relevance score in combined ranking
    #[arg(long, env = "RERANK_RELEVANCE_WEIGHT", default_value = "0.7")]
    pub rerank_relevance_weight: f32,

    /// Weight for domain authority in combined ranking
    #[arg(long, env = "RERANK_AUTHORITY_WEIGHT", default_value = "0.2")]
    pub rerank_authority_weight: f32,

    /// Weight for content quality heuristics in combined ranking
    #[arg(long, env = "RERANK_QUALITY_WEIGHT", default_value = "0.1")]
    pub rerank_quality_weight: f32,

    // ── Quality filter ───────────────────────────────────────────────────────
    /// Minimum word count for a source to pass quality filter
    #[arg(long, env = "MIN_CONTENT_WORDS", default_value = "100")]
    pub min_content_words: usize,

    /// Minimum text/HTML density ratio for quality filter
    #[arg(long, env = "MIN_TEXT_DENSITY", default_value = "0.05")]
    pub min_text_density: f32,
```

**Step 2: Mirror in `config_from_env()`**

In the `Config { ... }` struct literal inside `config_from_env()`, add after `dedup_threshold`:

```rust
        rerank_base_url: env("RERANK_BASE_URL", ""),
        rerank_relevance_weight: env_f32("RERANK_RELEVANCE_WEIGHT", 0.7),
        rerank_authority_weight: env_f32("RERANK_AUTHORITY_WEIGHT", 0.2),
        rerank_quality_weight: env_f32("RERANK_QUALITY_WEIGHT", 0.1),
        min_content_words: env_usize("MIN_CONTENT_WORDS", 100),
        min_text_density: env_f32("MIN_TEXT_DENSITY", 0.05),
```

**Step 3: Verify**

```bash
cargo check
```

**Step 4: Commit**

```bash
git add src/config.rs src/mcp_server.rs
git commit -m "feat: add config fields for reranker and quality filter

New env vars: RERANK_BASE_URL, RERANK_*_WEIGHT, MIN_CONTENT_WORDS,
MIN_TEXT_DENSITY. Both Config and config_from_env() updated."
```

---

### Task 5: Embedding Window Fix and Remove `rank_by_relevance`

**Files:**
- Modify: `src/embeddings/dedup.rs:17-62` (`deduplicate`) and `src/embeddings/dedup.rs:67-100` (`rank_by_relevance`)

**Context:** Two changes: (1) bump the embedding window from 512 chars to 2000 chars — BGE-large-en-v1.5 supports 512 tokens ≈ 2000 chars, so we're currently wasting 75% of the model's capacity. (2) Remove `rank_by_relevance()` entirely — its job is taken over by the cross-encoder reranker. The bi-encoder stays for dedup only.

**Step 1: Bump embedding window in `deduplicate()`**

Change `s.content.chars().take(512)` to `s.content.chars().take(2000)` and update the comment:

```rust
    // Use first ~2000 chars of content as embedding input.
    // BGE-large-en-v1.5 handles 512 tokens (~2000 chars); TEI --auto-truncate handles overshoot.
    let texts: Vec<String> = sources
        .iter()
        .map(|s| s.content.chars().take(2000).collect())
        .collect();
```

**Step 2: Remove `rank_by_relevance()`**

Delete the entire `rank_by_relevance` function (lines 67-100). Its job is now handled by the cross-encoder reranker.

**Step 3: Verify**

```bash
cargo check
```

Expected: error in `pipeline.rs` where `rank_by_relevance` is called — that's expected and fixed in Task 8 (pipeline integration).

**Step 4: Commit**

```bash
git add src/embeddings/dedup.rs
git commit -m "feat: widen embedding window to 2000 chars, remove rank_by_relevance

BGE-large-en-v1.5 supports 512 tokens (~2000 chars). The old 512-char
limit wasted 75% of model capacity. rank_by_relevance is replaced by
cross-encoder reranking."
```

---

### Task 6: Cross-Encoder Reranker Client

**Files:**
- Create: `src/embeddings/reranker.rs`
- Modify: `src/embeddings/mod.rs` (add `pub mod reranker;`)

**Context:** TEI exposes a `/rerank` endpoint when loaded with a cross-encoder model. It takes a query + list of documents and returns relevance scores. This is a simple HTTP client similar to `EmbedClient`.

**Step 1: Create `src/embeddings/reranker.rs`**

```rust
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::researcher::crawler::ScrapedSource;
use crate::researcher::quality::ContentQuality;

#[derive(Serialize)]
struct RerankRequest {
    query: String,
    texts: Vec<String>,
}

#[derive(Deserialize)]
struct RerankResult {
    index: usize,
    score: f32,
}

pub struct RerankerClient {
    http: Client,
    base_url: String,
}

/// A source annotated with ranking scores.
pub struct RankedSource {
    pub source: ScrapedSource,
    pub quality: ContentQuality,
    pub relevance_score: f32,
    pub combined_score: f32,
}

impl RerankerClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Rerank sources by relevance to the query using a cross-encoder.
    /// Returns sources sorted by combined score (descending).
    pub async fn rerank(
        &self,
        query: &str,
        sources: Vec<(ScrapedSource, ContentQuality)>,
        relevance_weight: f32,
        authority_weight: f32,
        quality_weight: f32,
    ) -> Result<Vec<RankedSource>> {
        if sources.is_empty() {
            return Ok(vec![]);
        }

        let texts: Vec<String> = sources
            .iter()
            .map(|(s, _)| s.content.chars().take(2000).collect())
            .collect();

        debug!(count = texts.len(), "reranking sources");

        let url = format!("{}/rerank", self.base_url);
        let req = RerankRequest {
            query: query.to_string(),
            texts,
        };

        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("rerank request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("TEI rerank error {status}: {body}");
        }

        let results: Vec<RerankResult> = resp.json().await.context("rerank parse")?;

        // Build scored results
        let mut ranked: Vec<RankedSource> = sources
            .into_iter()
            .enumerate()
            .map(|(i, (source, quality))| {
                let relevance_score = results
                    .iter()
                    .find(|r| r.index == i)
                    .map(|r| r.score)
                    .unwrap_or(0.0);

                let q_score = crate::researcher::quality::quality_score(&quality);

                let combined_score = (relevance_score * relevance_weight)
                    + (quality.domain_authority * authority_weight)
                    + (q_score * quality_weight);

                RankedSource {
                    source,
                    quality,
                    relevance_score,
                    combined_score,
                }
            })
            .collect();

        ranked.sort_by(|a, b| b.combined_score.partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal));

        debug!(
            count = ranked.len(),
            top_score = ranked.first().map(|r| r.combined_score).unwrap_or(0.0),
            "reranking complete"
        );

        Ok(ranked)
    }
}
```

**Step 2: Add module to `src/embeddings/mod.rs`**

Add `pub mod reranker;` after the existing module declarations.

**Step 3: Verify**

```bash
cargo check
```

**Step 4: Commit**

```bash
git add src/embeddings/reranker.rs src/embeddings/mod.rs
git commit -m "feat: add cross-encoder reranker client

RerankerClient calls TEI /rerank endpoint with cross-encoder model.
Combines relevance score with domain authority and quality heuristics
into a weighted combined score."
```

---

### Task 7: LLM Relevance Judge in Summarizer

**Files:**
- Modify: `src/researcher/summarizer.rs:9-14` (`SourceSummary`), `src/researcher/summarizer.rs:17-36` (`summarize_source`), `src/researcher/summarizer.rs:39-75` (`summarize_all`)

**Context:** Replace the fragile `!summary.to_lowercase().contains("not relevant")` substring check with structured JSON output from the LLM. A single LLM call does both relevance judging and summarizing — no extra round trip.

**Step 1: Add JSON response struct**

Above `summarize_source`, add:

```rust
#[derive(serde::Deserialize)]
struct JudgedSummary {
    relevant: bool,
    #[allow(dead_code)]
    confidence: Option<f32>,
    summary: String,
}
```

**Step 2: Update `summarize_source` to return `Option<SourceSummary>`**

Change signature to `async fn summarize_source(llm, source, topic) -> Result<Option<SourceSummary>>`.

New prompt:

```rust
async fn summarize_source(llm: &LlmClient, source: &ScrapedSource, topic: &str) -> Result<Option<SourceSummary>> {
    let messages = vec![
        ChatMessage::system(
            "You are a research analyst. Evaluate the web page content for relevance to the \
             research question, then summarize if relevant.\n\n\
             Return JSON with exactly these fields:\n\
             {\"relevant\": true/false, \"confidence\": 0.0-1.0, \"summary\": \"...\"}\n\n\
             Set relevant=false if the content does not meaningfully address the research question.\n\
             When relevant=true, the summary should be concise and factual, including key facts, \
             data, and claims. When relevant=false, summary should be empty string.\n\n\
             Return ONLY the JSON object, no markdown fences or extra text.",
        ),
        ChatMessage::user(format!(
            "Research topic: {topic}\n\
             Specific question this source addresses: {}\n\n\
             Source URL: {}\n\
             Source content:\n{}\n",
            source.query, source.url, source.content,
        )),
    ];

    let response = llm.complete(messages).await?;

    // Try to parse structured JSON response
    match serde_json::from_str::<JudgedSummary>(response.trim()) {
        Ok(judged) => {
            if !judged.relevant {
                debug!(url = %source.url, "LLM judge: not relevant");
                return Ok(None);
            }
            if judged.summary.is_empty() {
                return Ok(None);
            }
            Ok(Some(SourceSummary {
                url: source.url.clone(),
                title: source.title.clone(),
                query: source.query.clone(),
                summary: judged.summary,
            }))
        }
        Err(_) => {
            // Fallback: treat entire response as plain summary (backward compat)
            debug!(url = %source.url, "LLM judge: JSON parse failed, using plain summary");
            if response.is_empty() || response.to_lowercase().contains("not relevant") {
                Ok(None)
            } else {
                Ok(Some(SourceSummary {
                    url: source.url.clone(),
                    title: source.title.clone(),
                    query: source.query.clone(),
                    summary: response,
                }))
            }
        }
    }
}
```

**Step 3: Simplify `summarize_all`**

Replace the `filter_map` closure. Now `summarize_source` returns `Option<SourceSummary>` directly:

```rust
pub async fn summarize_all(
    llm: &LlmClient,
    sources: &[ScrapedSource],
    topic: &str,
) -> Vec<SourceSummary> {
    debug!(count = sources.len(), "summarizing sources");

    let futs = sources.iter().map(|source| {
        let llm = llm.clone();
        let topic = topic.to_string();
        let source = source.clone();
        async move { summarize_source(&llm, &source, &topic).await }
    });

    join_all(futs)
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(Some(summary)) => Some(summary),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(%e, "summarize failed");
                None
            }
        })
        .collect()
}
```

**Step 4: Verify**

```bash
cargo check
```

**Step 5: Commit**

```bash
git add src/researcher/summarizer.rs
git commit -m "feat: structured LLM relevance judge in summarizer

Replace fragile 'not relevant' substring check with structured JSON
output. Single LLM call does both relevance judging and summarizing.
Falls back to plain-text parsing if JSON fails."
```

---

### Task 8: Pipeline Integration

**Files:**
- Modify: `src/researcher/pipeline.rs:119-236` (`run`), `src/researcher/pipeline.rs:250-260` (`ProgressEvent`)

**Context:** This is the main orchestrator. We need to: add new progress events, insert the quality filter stage, replace bi-encoder ranking with cross-encoder reranking, and extract `ScrapedSource` from `RankedSource` before passing to the summarizer.

**Step 1: Add new `ProgressEvent` variants**

Add to the enum:

```rust
    QualityFiltering { total: usize },
    Reranking { total: usize },
```

And corresponding `Display` arms:

```rust
    Self::QualityFiltering { total } => write!(f, "Filtering {total} sources by quality"),
    Self::Reranking { total } => write!(f, "Reranking {total} sources"),
```

**Step 2: Add imports at top of pipeline.rs**

```rust
use crate::researcher::quality::filter_sources;
use crate::embeddings::reranker::RerankerClient;
```

**Step 3: Rewrite the dedup/rank section of `run()` (approximately lines 197-215)**

Replace the current block that does dedup + rank_by_relevance with:

```rust
    // 8a. Quality filter (always active)
    on_progress(ProgressEvent::QualityFiltering { total: sources.len() });
    let quality_sources = filter_sources(sources, &request.target, cfg);
    info!(sources = quality_sources.len(), "quality filter complete");

    // 8b. Embedding dedup (if TEI configured)
    let sources = if !cfg.embed_base_url.is_empty() {
        on_progress(ProgressEvent::Deduplicating { total: quality_sources.len() });
        let embed = EmbedClient::new(&cfg.embed_base_url);
        let just_sources: Vec<ScrapedSource> = quality_sources.into_iter().map(|(s, _q)| s).collect();
        let deduped = deduplicate(&embed, just_sources, cfg.dedup_threshold).await;

        // Re-assess quality after dedup (since we lost the quality annotations)
        let quality_sources: Vec<_> = deduped
            .into_iter()
            .map(|s| {
                let q = crate::researcher::quality::assess_quality(&s, &request.target);
                (s, q)
            })
            .collect();

        // 8c. Cross-encoder rerank (if reranker configured)
        if !cfg.rerank_base_url.is_empty() {
            on_progress(ProgressEvent::Reranking { total: quality_sources.len() });
            let reranker = RerankerClient::new(&cfg.rerank_base_url);
            match reranker.rerank(
                topic,
                quality_sources,
                cfg.rerank_relevance_weight,
                cfg.rerank_authority_weight,
                cfg.rerank_quality_weight,
            ).await {
                Ok(ranked) => {
                    on_progress(ProgressEvent::CrawlComplete { sources: ranked.len() });
                    ranked.into_iter().map(|r| r.source).collect()
                }
                Err(e) => {
                    tracing::warn!(%e, "cross-encoder rerank failed, using dedup order");
                    quality_sources.into_iter().map(|(s, _)| s).collect()
                }
            }
        } else {
            on_progress(ProgressEvent::CrawlComplete { sources: quality_sources.len() });
            quality_sources.into_iter().map(|(s, _)| s).collect()
        }
    } else {
        on_progress(ProgressEvent::CrawlComplete { sources: quality_sources.len() });
        quality_sources.into_iter().map(|(s, _)| s).collect()
    };
```

Note: this replaces the old block that called `rank_by_relevance`. The variable shadowing `let sources = ...` keeps downstream code unchanged.

**Step 4: Fix the `SourceEntry` construction**

The existing code at ~line 182 builds `source_entries` from `sources` before dedup. This should stay as-is — it captures all crawled sources for the result, regardless of filtering. No change needed.

**Step 5: Verify**

```bash
cargo check
```

This is the critical integration point. Fix any remaining type errors.

**Step 6: Full build**

```bash
cargo build --release
```

**Step 7: Commit**

```bash
git add src/researcher/pipeline.rs
git commit -m "feat: integrate quality filter, cross-encoder rerank into pipeline

Pipeline flow: crawl → quality filter → bi-encoder dedup → cross-encoder
rerank → LLM judge+summarize → report. Each stage degrades gracefully
when its backing service isn't configured."
```

---

### Task 9: Docker Compose and Profiles

**Files:**
- Modify: `docker-compose.yml`
- Modify: `profiles.toml`

**Context:** Add the `tei-rerank` service for the cross-encoder model and update profiles.toml with domain authority tiers. The reranker is a tiny model (~80MB VRAM) alongside the existing TEI bi-encoder.

**Step 1: Add `tei-rerank` service to `docker-compose.yml`**

Add after the `tei-embed` service block:

```yaml
  # ── TEI: Cross-encoder reranker (for source relevance scoring) ─────────────
  # Lightweight cross-encoder (~80MB VRAM). Runs alongside tei-embed.
  tei-rerank:
    image: ghcr.io/huggingface/text-embeddings-inference:86-1.8
    container_name: tei-rerank
    networks:
      - researcher-net
    ports:
      - "8082:80"
    command: --model-id cross-encoder/ms-marco-MiniLM-L-6-v2 --port 80 --auto-truncate --max-concurrent-requests 32
    volumes:
      - tei-rerank-cache:/data
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: 1
              capabilities: [gpu]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:80/health"]
      interval: 10s
      timeout: 5s
      retries: 10
      start_period: 60s
    restart: unless-stopped
```

Add `tei-rerank-cache:` to the top-level `volumes:` section.

Add to the `researcher` service environment:
```yaml
      - RERANK_BASE_URL=http://tei-rerank:80
```

Add to `researcher` service `depends_on`:
```yaml
      tei-rerank:
        condition: service_healthy
```

**Step 2: Add domain authority section to `profiles.toml`**

Append at the end of the file:

```toml
# ── Domain authority tiers ────────────────────────────────────────────────────
# Used by the quality filter to weight sources by domain reputation.
# Tier 1 (1.0): Authoritative sources — official docs, .gov, .edu, encyclopedias
# Tier 2 (0.7): Established outlets — major news, tech publications, Q&A sites
# Tier 3 (0.5): Community sources — forums, blog platforms, social media
# Tier 4 (0.3): Default for unlisted domains
#
# Target-specific boosts (person/company/code) are hardcoded in quality.rs.

[domain-authority]
tier1 = [
  "wikipedia.org", "github.com", "arxiv.org", "docs.rs",
  "doc.rust-lang.org", "developer.mozilla.org", "w3.org",
  "python.org", "golang.org", "docs.oracle.com",
]
tier2 = [
  "nytimes.com", "bbc.com", "reuters.com", "bloomberg.com",
  "techcrunch.com", "arstechnica.com", "nature.com",
  "stackoverflow.com", "wired.com", "theverge.com",
]
tier3 = [
  "reddit.com", "medium.com", "quora.com", "dev.to",
  "hackernoon.com", "substack.com", "news.ycombinator.com",
]
```

Note: the current `quality.rs` implementation uses hardcoded domain lists. Loading from `profiles.toml` is a future enhancement — the TOML section documents the intent and can be loaded later without changing the file format.

**Step 3: Verify**

```bash
cargo check
```

**Step 4: Commit**

```bash
git add docker-compose.yml profiles.toml
git commit -m "infra: add cross-encoder reranker to Docker stack

tei-rerank service with cross-encoder/ms-marco-MiniLM-L-6-v2 (~80MB).
Document domain authority tiers in profiles.toml."
```

---

### Task 10: Update CLAUDE.md Env Vars Table

**Files:**
- Modify: `CLAUDE.md`

**Context:** The CLAUDE.md env vars table needs the new variables documented.

**Step 1: Add new env vars to the table**

After the `DEDUP_THRESHOLD` row, add:

```
| `RERANK_BASE_URL` | `` (disabled) | TEI cross-encoder URL; empty = skip reranking |
| `RERANK_RELEVANCE_WEIGHT` | `0.7` | Cross-encoder score weight in combined ranking |
| `RERANK_AUTHORITY_WEIGHT` | `0.2` | Domain authority weight in combined ranking |
| `RERANK_QUALITY_WEIGHT` | `0.1` | Content quality weight in combined ranking |
| `MIN_CONTENT_WORDS` | `100` | Quality filter: minimum word count |
| `MIN_TEXT_DENSITY` | `0.05` | Quality filter: minimum text/HTML density ratio |
```

**Step 2: Update Pipeline Flow diagram**

Update the pipeline flow in CLAUDE.md to reflect the new stages:

```
query → planner (LLM) → [search+scrape]×N → quality filter → embed-dedup → cross-encoder rerank → [summarize+judge]×M → publisher (LLM)
```

**Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add reranker and quality filter env vars to CLAUDE.md"
```

---

### Task 11: Final Verification

**Step 1: Full check**

```bash
cargo check
```

**Step 2: Clippy**

```bash
cargo clippy -- -D warnings
```

Fix any warnings.

**Step 3: Release build**

```bash
cargo build --release
```

**Step 4: Commit any clippy fixes**

```bash
git add -A
git commit -m "fix: address clippy warnings from ranking improvements"
```

(Only if there were fixes needed.)
