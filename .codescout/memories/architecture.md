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
- `run()` — `src/researcher/pipeline.rs` — main research orchestrator
- `ResearchRequest { topic, mode, domains, domain_profile, target, intent }` — controls ALL pipeline behavior
- `ResearchTarget` enum — `Topic | Person { method } | Company | Market { asset_class }` — fans out domain lists, prompts, report templates
- `ResearchMode` enum — `Quick | Summary | Report | Deep` — controls depth; Quick returns after crawl, no summarize/write
- `LlmClient { base_url, model, disable_thinking, strip_thinking, … }` — single struct for both heavy and fast LLM; `new()` vs `new_fast()`
- `ProgressEvent` enum (11 variants) — typed progress notifications; Display impl for human-readable strings
- `Config` — `src/config.rs` — single clap-derive struct serving both CLI and server; `config_from_env()` in mcp_server.rs is a manually maintained duplicate

## Pipeline Stages
1. Resolve explicit profile/raw domains as hard backend constraints; target fallback domains remain soft planning hints
2. Deep mode: 2× max_queries + max_sources
3. Build heavy LlmClient (`new`) + fast LlmClient (`new_fast`)
4. Assign stage clients using Config.llm_fast_stages; empty routes all stages to the primary client
5. **Plan** — `generate_queries()` — LLM generates N sub-questions; domain hints injected per target type
6. **Crawl** — `crawl_all()` — sequential queries, concurrent fetches per query; shared visited-URL HashSet for global dedup
7. **Quick exit** — if mode==Quick, return sources without summarizing
8a. Quality filter — `filter_sources()` — drops thin (<100 words), paywalled, low-density pages
8b. Embed dedup — if EMBED_BASE_URL set: `deduplicate()` greedy cosine-sim (threshold 0.92), re-assess quality
8c. Rerank independently of dedup when RERANK_BASE_URL is set. Validate full unique index coverage/finite scores; combine percentile relevance with authority/quality. Absolute raw-score filtering defaults disabled (-inf)
9. **Select evidence** — summarize_all has concurrency3; relevant quotes must occur verbatim in retrieved text. Origin, observation time and truncation follow the retained excerpts
10. **Write and verify** — bounded <=1200-character segments, <=24 checks, concurrency3, at most2 drafts. LLM_VERIFIER_MODEL optionally selects an independent model on the publisher backend. Property-level failures or invalid verdicts trigger repair, then attributed evidence fallback. Final SourceEntry values are rebuilt from retained evidence (including recovery sources); format_report appends authoritative provenance

## Streaming vs Blocking Split
- CLI / HTTP SSE: draft tokens are streamed into an internal drain; validated report is then sent to the caller. Unverified draft text is withheld
- MCP / JSON API: `token_tx = None` → `llm.complete()` (blocking POST, returns full string)
- Same run() handles both paths. Vertex output usage sums reported candidates and thoughts in blocking and SSE paths; disclosure abstracts/takeaways are extractive

## Design Patterns
- All pipeline stages are pure functions (no shared mutable state except `visited` HashSet in crawl)
- `on_progress: impl Fn(ProgressEvent)` closure — decouples transport from pipeline
- MCP exposes one research(query,target?,...) dispatcher; it uses the pipeline or target-specific helpers
- Job search deep mode: calls `run(ResearchTarget::Company)` recursively for top 5; concurrent via join_all (no rate limit)

## Invariants

| Rule | Why it exists |
|---|---|
| Config field added → update BOTH `Config` AND `config_from_env()` | MCP binary can't use clap; manual duplicate; drift = silent wrong defaults |
| `crawl_all()` queries are sequential | Shared `visited: HashSet<String>` for global URL dedup; parallelizing requires Arc<Mutex<>> |
| `strip_thinking` exists in client.rs AND stream.rs | Both paths strip `<think>` — changing one without the other leaves thinking tokens in one mode |
| MCP dispatch helpers return Result<String>; the tool returns CallToolResult | Map failures to CallToolResult::error (isError=true), preserving the full anyhow error chain |

## Strong Defaults

| Default | When it's okay to break it |
|---|---|
| Stage routing uses Config.llm_fast_stages | Configure LLM_FAST_STAGES and compatible fast backend/model |
| Verifier inherits publisher model unless configured | Set LLM_VERIFIER_MODEL to a stronger compatible model; local Vertex uses gemini-3.1-pro-preview while other stages use Flash Lite |
| Dedup/rerank disabled if env vars empty | Enable by setting EMBED_BASE_URL / RERANK_BASE_URL |
