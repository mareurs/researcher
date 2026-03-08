# Architecture

## Module Structure
```
src/
  config.rs          — Config + AuthConfig + JobProfile structs; load_profiles(), load_job_profile()
  main.rs            — CLI entry: run_server() or run_cli(); clap Config
  mcp_server.rs      — MCP binary entry; ResearcherServer + config_from_env()
  server.rs          — Axum router: GET /, POST /research, POST /research/stream, GET /health
  researcher/
    pipeline.rs      — run() orchestrator; ResearchRequest, ResearchMode, ResearchTarget, ProgressEvent
    planner.rs       — generate_queries() — LLM → sub-questions list
    crawler.rs       — crawl_all() + crawl_query() — search + parallel scrape
    summarizer.rs    — summarize_all() — concurrent join_all LLM summaries
    publisher.rs     — write_report() + format_report() — final report LLM call
  llm/
    client.rs        — LlmClient: .complete() (blocking) and .stream() (SSE/mpsc)
    stream.rs        — stream_completion() — raw SSE token streaming
  search/
    mod.rs           — search_with_fallback(): SearXNG → DuckDuckGo
    searxng.rs       — JSON API client
    duckduckgo.rs    — DDG Lite HTML scraper fallback
  scraper/html.rs    — fetch_and_extract(): reqwest + scraper crate HTML→text; cookie auth support
  embeddings/
    client.rs        — EmbedClient: TEI /embed batch API (supports TEI batch and OpenAI formats)
    dedup.rs         — deduplicate() + rank_by_relevance() using cosine similarity
  jobs/
    fetcher.rs       — fetch_jobs(): Remotive + Adzuna + SearXNG → Vec<JobListing>
    scorer.rs        — score_listings(): single LLM call scores all listings against JobProfile
    publisher.rs     — write_job_report(): markdown table; optionally spawns company_research per top job
```

## Key Abstractions

| Type | File | Role |
|------|------|------|
| `run()` | researcher/pipeline.rs:119 | Central orchestrator — all 10 pipeline stages |
| `ResearchRequest` | researcher/pipeline.rs:53 | Input: topic, mode, target, domains |
| `ResearchMode` | researcher/pipeline.rs:16 | Quick/Summary/Report/Deep — affects depth and short-circuits |
| `ResearchTarget` | researcher/pipeline.rs:43 | Topic / Person{method} / Company — affects domains + prompts |
| `LlmClient` | llm/client.rs:50 | OpenAI-compat HTTP; blocking complete() or streaming stream() |
| `Config` | config.rs:32 | All tunables; `profiles` loaded from profiles.toml at startup |

## Data Flow (research pipeline)

```
ResearchRequest
  → generate_queries(&llm, topic, max_queries, &domains, &target)  [planner.rs]
  → crawl_all(&http, &cfg, &queries)                                [crawler.rs]
      for each query (sequential for visited_url dedup):
        search_with_fallback() → fresh URLs → join_all(fetch_and_extract)
  → if EMBED_BASE_URL set:
      deduplicate(&embed, sources, threshold)
      rank_by_relevance(&embed, topic, sources)
  → summarize_all(&llm, &sources, topic)  ← join_all (all LLM calls concurrent) [summarizer.rs]
  → write_report(&llm, topic, &summaries, &mode, &target, token_tx)  [publisher.rs]
  → format_report(raw_report, &summaries)  ← appends Sources section
```

## ResearchMode Behaviors
- `Quick` — short-circuits after crawl, returns SourceEntry list with `report: None`
- `Summary` — bullet-point prompt, no exec summary
- `Report` (default) — full structured report with sections + citations
- `Deep` — doubles max_queries and max_sources, uses deep-detail prompt

## Job Search Flow (separate from pipeline)
```
search_jobs input → fetch_jobs() [Remotive + Adzuna + SearXNG deduplicated]
  → score_listings() [single LLM call, JSON scores array]
  → write_job_report() [markdown; deep=true spawns company research per top job]
```

## Invariants

| Rule | Why it matters |
|------|---------------|
| `token_tx = None` for MCP/HTTP-JSON | Streaming requires SSE transport; MCP is blocking |
| `crawl_all()` runs queries sequentially | Shares `visited_urls: HashSet` for cross-query dedup |
| `profiles.toml` read at startup (Config::new) | Missing file = empty profiles map + no job-profile |
| `strip_thinking` applied in both client.rs and stream.rs | Qwen3 emits `<think>` tokens in both paths |

## Strong Defaults

| Default | When to override |
|---------|-----------------|
| SearXNG → DDG fallback automatic | Never — it's always-on in search_with_fallback() |
| Embedding dedup skipped if EMBED_BASE_URL empty | Set EMBED_BASE_URL when source quality matters |
| Domain list empty = unrestricted search | Set domains or domain_profile for scoped research |
