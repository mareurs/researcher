# Ranking Improvements Design

**Date:** 2026-03-09
**Status:** Approved

## Problem

The current ranking system is functional but minimal:
- Only 512 chars of each source are embedded (BGE supports ~2000 chars)
- Bi-encoder only — no cross-encoder reranking
- No domain-aware scoring (LinkedIn vs blog vs news treated identically)
- No content quality signals (ad-heavy, thin, paywalled pages score the same)
- Fragile "not relevant" substring check during summarization
- Falls back to crawl order when embeddings are disabled
- `extract_text` misses `<table>`, `<pre>`, `<blockquote>` elements

## Constraints

- Local GPU stack (Docker compose with TEI + llama-cpp)
- Single GPU with headroom (~24GB, quantized 7-9B LLM leaves 4-8GB free)
- Quality over latency — LLM-based ranking is acceptable
- Must degrade gracefully when services aren't configured

## Design

### New Pipeline Flow

```
Plan → Crawl → Quality Filter → Dedup(bi-encoder) → Rerank(cross-encoder) → LLM Judge+Summarize → Report
```

Changes from current:
1. **Quality Filter** — new stage after crawl, pure heuristics, no model calls
2. **Rerank** — replaces bi-encoder `rank_by_relevance()` with cross-encoder `/rerank`
3. **LLM Judge** — replaces "not relevant" substring check with structured JSON output inside `summarize_source()`

### Section 1: Scraper Enrichment

`fetch_and_extract` returns `ExtractedPage` instead of `String`:

```rust
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

Metadata collected during the existing DOM traversal at near-zero cost:
- `raw_html_len`: `html.len()` before parsing
- `has_headings/has_lists/has_code_blocks`: detected during selector walk
- `paywall_detected`: substring scan for common patterns ("subscribe to continue", "sign in to read", `tp-modal`, `pw-overlay`)
- `link_count/ad_link_count`: count `<a>` hrefs, check against hardcoded ad domain list

`extract_text` gains `<table>`, `<pre>`, `<blockquote>` selectors for tech/academic content.

`ScrapedSource` gains fields:

```rust
pub struct ScrapedSource {
    pub url: String,
    pub title: String,
    pub query: String,
    pub content: String,
    pub domain: String,                   // extracted from url
    pub word_count: usize,                // content.split_whitespace().count()
    pub quality: Option<ContentQuality>,  // None when quality filter disabled
}
```

### Section 2: Content Quality Filter

New module: `src/researcher/quality.rs`

```rust
pub struct ContentQuality {
    pub word_count: usize,
    pub text_density: f32,       // text chars / total HTML chars
    pub has_structure: bool,     // headings, lists, or code blocks
    pub is_paywall: bool,
    pub ad_link_ratio: f32,      // external ad links / total links
    pub domain_authority: f32,   // from configurable domain weight map
}
```

Filtering rules:
- **Drop** if `word_count < 100` (thin content)
- **Drop** if `is_paywall == true`
- **Drop** if `text_density < 0.05` (almost entirely markup)
- **Penalize** (don't drop) low `domain_authority` and high `ad_link_ratio`

Domain authority tiers in `profiles.toml`:
```toml
[domain-authority]
tier1 = ["wikipedia.org", "github.com", "arxiv.org"]       # 1.0
tier2 = ["nytimes.com", "bbc.com", "reuters.com"]           # 0.7
tier3 = ["reddit.com", "medium.com", "quora.com"]           # 0.5
# unlisted domains default to tier 4                        # 0.3
```

Target-specific overrides: person research boosts LinkedIn/GitHub, company research boosts Crunchbase/Bloomberg, etc. Uses existing `ResearchTarget` enum.

### Section 3: Embedding Window Fix + Bi-Encoder Dedup

Bump `chars().take(512)` → `chars().take(2000)` in both `deduplicate()` and embedding input. BGE-large-en-v1.5 supports 512 tokens ≈ 2000 chars. TEI `--auto-truncate` handles any overshoot.

`rank_by_relevance()` is **removed** — its job is taken over by the cross-encoder reranker. Bi-encoder stays for dedup only (symmetric similarity is well-suited for dedup).

### Section 4: Cross-Encoder Reranker

New module: `src/embeddings/reranker.rs`

```rust
pub struct RerankerClient {
    http: Client,
    base_url: String,
}

pub struct RankedSource {
    pub source: ScrapedSource,
    pub relevance_score: f32,    // cross-encoder score
    pub quality_score: f32,      // from quality filter
    pub combined_score: f32,     // weighted blend
}
```

Infrastructure: second TEI container with `cross-encoder/ms-marco-MiniLM-L-6-v2` (~80MB VRAM). TEI exposes `/rerank` endpoint natively.

The `/rerank` call:
- Input: research topic as query, first ~2000 chars of each source as documents
- Output: trained relevance scores per document (0-1 range)
- Single batched HTTP call

Score combination:
```
combined_score = (relevance_score * 0.7) + (domain_authority * 0.2) + (quality_score * 0.1)
```

Weights configurable via env vars. Sources sorted by `combined_score` descending.

Fallback: when `RERANK_BASE_URL` is empty, fall back to bi-encoder ranking (existing behavior with wider window).

Docker compose addition:
```yaml
tei-rerank:
  image: ghcr.io/huggingface/text-embeddings-inference:86-1.8
  command: --model-id cross-encoder/ms-marco-MiniLM-L-6-v2 --port 80
```

### Section 5: LLM Relevance Judge

Replaces the `!summary.to_lowercase().contains("not relevant")` check. Runs inside `summarize_source()` — single LLM call does both judging and summarizing.

Prompt returns structured JSON:
```json
{
  "relevant": true,
  "confidence": 0.85,
  "summary": "The actual summary text..."
}
```

Changed function:
```rust
// Before
async fn summarize_source(llm, source, topic) -> Result<String>

// After
async fn summarize_source(llm, source, topic) -> Result<Option<SourceSummary>>
// Returns None when LLM judges source as irrelevant
```

Fallback: if JSON parsing fails, treat response as plain summary and keep it (same as current behavior).

`confidence` is logged but not used for filtering initially — available for future tuning.

`summarize_all` simplifies: collect `Some(...)` values, drop `None`.

### Section 6: Configuration

New env vars:

| Variable | Default | Notes |
|----------|---------|-------|
| `RERANK_BASE_URL` | `` (disabled) | TEI reranker URL |
| `RERANK_RELEVANCE_WEIGHT` | `0.7` | Cross-encoder score weight |
| `RERANK_AUTHORITY_WEIGHT` | `0.2` | Domain authority weight |
| `RERANK_QUALITY_WEIGHT` | `0.1` | Content quality weight |
| `MIN_CONTENT_WORDS` | `100` | Quality filter: min word count |
| `MIN_TEXT_DENSITY` | `0.05` | Quality filter: min text/HTML ratio |

Degradation ladder:

| Configuration | Behavior |
|---|---|
| `EMBED_BASE_URL` + `RERANK_BASE_URL` | Full: quality filter → bi-encoder dedup → cross-encoder rerank → LLM judge |
| Only `EMBED_BASE_URL` | Quality filter → bi-encoder dedup → bi-encoder rank (wider window) → LLM judge |
| Neither | Quality filter → crawl order → LLM judge |

Quality filter and LLM judge are always active (no external service dependency).

Both `Config` and `config_from_env()` updated per project rules.

## Files Changed

**New files:**
- `src/researcher/quality.rs` — ContentQuality, filter_sources(), domain authority
- `src/embeddings/reranker.rs` — RerankerClient, RankedSource, /rerank call

**Modified files:**
- `src/scraper/html.rs` — ExtractedPage return type, table/pre/blockquote selectors, metadata collection
- `src/researcher/crawler.rs` — ScrapedSource gains domain/word_count/quality fields
- `src/embeddings/dedup.rs` — 512→2000 chars, remove rank_by_relevance()
- `src/researcher/summarizer.rs` — structured JSON output, Option<SourceSummary> return
- `src/researcher/pipeline.rs` — new stages, new ProgressEvent variants, remove rank_by_relevance call
- `src/config.rs` — new fields for reranker, weights, quality thresholds
- `src/mcp_server.rs` — mirror config in config_from_env()
- `docker-compose.yml` — add tei-rerank service
- `profiles.toml` — add [domain-authority] section

**Unchanged:**
- `src/researcher/planner.rs`, `src/researcher/publisher.rs`, `src/llm/`, public API types

## Risk Areas

1. **Structured JSON from Qwen 7-9B** — sometimes produces malformed JSON. Mitigated by fallback to plain-text summary.
2. **ExtractedPage touches critical path** — every source goes through the scraper. Needs careful testing.
3. **Domain authority tiers are opinionated** — wrong weights could hurt. Configurable weights are the safety net.
