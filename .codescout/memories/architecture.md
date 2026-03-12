# Architecture

## Layer Structure
```
src/main.rs / src/mcp_server.rs  — binary entry points
src/config.rs                     — Config (clap+env), AuthConfig, JobProfile
src/server.rs                     — Axum: GET /, GET /health, POST /research, POST /research/stream
src/researcher/                   — core pipeline (see Pipeline Stages below)
src/llm/                          — LLM client (blocking + streaming)
src/search/                       — SearXNG + DDG fallback
src/scraper/                      — HTML fetch + text extraction
src/embeddings/                   — TEI embed client, cosine-sim dedup, cross-encoder reranker
src/jobs/                         — job fetching (Remotive+Adzuna+SearXNG), LLM scoring, reporting
```

## Key Abstractions
- `run()` — `src/researcher/pipeline.rs:174` — 10-stage orchestrator; only external-facing surface of the pipeline
- `ResearchRequest { topic, mode, domains, domain_profile, target, fast_stages }` — controls ALL pipeline behavior
- `ResearchTarget` enum — `Topic | Person { method } | Company | Market { asset_class }` — fans out domain lists, prompts, report templates
- `ResearchMode` enum — `Quick | Summary | Report | Deep` — controls depth; Quick returns after crawl, no summarize/write
- `LlmClient { base_url, model, disable_thinking, strip_thinking, … }` — single struct for both heavy and fast LLM; `new()` vs `new_fast()`
- `ProgressEvent` enum (11 variants) — typed progress notifications; Display impl for human-readable strings
- `Config` — `src/config.rs` — single clap-derive struct serving both CLI and server; `config_from_env()` in mcp_server.rs is a manually maintained duplicate

## Pipeline Stages (run() — pipeline.rs:174)
1. Resolve effective domains (profile lookup ∪ raw domains ∪ target fallback)
2. Deep mode: 2× max_queries + max_sources
3. Build heavy LlmClient (`new`) + fast LlmClient (`new_fast`)
4. Assign stage LLM clients based on `fast_stages` (request overrides cfg; default: planner+summarizer=fast, publisher=heavy)
5. **Plan** — `generate_queries()` — LLM generates N sub-questions; domain hints injected per target type
6. **Crawl** — `crawl_all()` — sequential queries, concurrent fetches per query; shared visited-URL HashSet for global dedup
7. **Quick exit** — if mode==Quick, return sources without summarizing
8a. Quality filter — `filter_sources()` — drops thin (<100 words), paywalled, low-density pages
8b. Embed dedup — if EMBED_BASE_URL set: `deduplicate()` greedy cosine-sim (threshold 0.92), re-assess quality
8c. Cross-encoder rerank — if RERANK_BASE_URL set: `rerank()` combined score (relevance×0.7 + authority×0.2 + quality×0.1)
9. **Summarize** — `summarize_all()` — fully concurrent join_all; LlmClient cloned per future (reqwest::Client is Arc-backed)
10. **Write** — `write_report()` — dispatches on ResearchTarget+ResearchMode for prompt template; streaming (token_tx=Some) or blocking (token_tx=None); `format_report()` appends numbered Sources section

## Streaming vs Blocking Split
- CLI / HTTP SSE: `token_tx = Some(tx)` → `stream_completion()` (eventsource-stream SSE, mpsc channel)
- MCP / JSON API: `token_tx = None` → `llm.complete()` (blocking POST, returns full string)
- Same `run()` function handles both paths

## Design Patterns
- All pipeline stages are pure functions (no shared mutable state except `visited` HashSet in crawl)
- `on_progress: impl Fn(ProgressEvent)` closure — decouples transport from pipeline
- MCP tools: each builds a ResearchRequest and calls run() with eprintln! as progress handler
- Job search deep mode: calls `run(ResearchTarget::Company)` recursively for top 5; concurrent via join_all (no rate limit)

## Invariants

| Rule | Why it exists |
|---|---|
| Config field added → update BOTH `Config` AND `config_from_env()` | MCP binary can't use clap; manual duplicate; drift = silent wrong defaults |
| `crawl_all()` queries are sequential | Shared `visited: HashSet<String>` for global URL dedup; parallelizing requires Arc<Mutex<>> |
| `strip_thinking` exists in client.rs AND stream.rs | Both paths strip `<think>` — changing one without the other leaves thinking tokens in one mode |
| MCP tool methods return String not Result | rmcp tool return type is String; errors embedded as "Error: {e:#}" |

## Strong Defaults

| Default | When it's okay to break it |
|---|---|
| planner+summarizer use fast LLM | Override per-request via `fast_stages` field or `LLM_FAST_STAGES` env |
| publisher uses heavy LLM | Add "publisher" to fast_stages if report quality is acceptable from fast model |
| Dedup/rerank disabled if env vars empty | Enable by setting EMBED_BASE_URL / RERANK_BASE_URL |
