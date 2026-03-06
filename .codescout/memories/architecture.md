# Architecture

## Key Abstractions (with file paths)

- **`Config`** (`src/config.rs`) — all runtime settings (clap+env); passed as `&Config` or `Arc<Config>` everywhere
- **`LlmClient`** (`src/llm/client.rs`) — wraps OpenAI-compat HTTP; `complete()` is blocking; streaming via `stream_completion()` in `src/llm/stream.rs`
- **`ScrapedSource`** (`src/researcher/crawler.rs`) — central data struct flowing from crawler → embeddings → summarizer: `{ url, title, content }`
- **`SourceSummary`** (`src/researcher/summarizer.rs`) — output of summarizer: `{ url, title, summary }`
- **`ProgressEvent`** (`src/researcher/pipeline.rs`) — enum emitted during pipeline; `Display` impl gives human-readable messages
- **`EmbedClient`** (`src/embeddings/client.rs`) — optional TEI client; only used if `cfg.embed_base_url` is non-empty

## Data Flow (actual function names)

```
run() [pipeline.rs]
  → generate_queries(&llm, topic, max) [planner.rs]  — LLM → Vec<String>
  → crawl_all(&http, cfg, &queries) [crawler.rs]     — search_with_fallback + scrape → Vec<ScrapedSource>
  → deduplicate(&embed, sources, threshold)           — optional, cosine sim filter
  → rank_by_relevance(&embed, topic, deduped)         — optional, rerank by query similarity
  → summarize_all(&llm, &sources, topic) [summarizer.rs] — concurrent join_all → Vec<SourceSummary>
  → write_report(&llm, topic, &summaries, token_tx)  [publisher.rs]
  → format_report(&report, &summaries)               — appends sources list
```

## Entry Points

- **CLI/server:** `src/main.rs` → `run_cli()` or `run_server()`
- **MCP:** `src/mcp_server.rs` → `ResearcherServer` implements `ServerHandler`
- **HTTP:** `src/server.rs` → `router()` mounts `POST /research` (JSON) and `POST /research/stream` (SSE)

## Streaming Pattern

`token_tx: Option<mpsc::Sender<String>>`:
- `None` → blocking `complete()` used for final report (MCP mode, JSON API mode)
- `Some(tx)` → `stream_completion()` for final report (CLI mode, SSE mode)
- Progress always goes through `on_progress: impl Fn(ProgressEvent)` callback (separate from token stream)

## Design Notes

- `run()` in pipeline.rs is `async`; all LLM calls inside are `.await`
- `summarize_all` uses `join_all` for concurrent LLM calls
- Search fallback is transparent: `search_with_fallback()` tries SearXNG first, then DDG Lite
- `STRIP_THINKING_TOKENS=true` strips `<think>...</think>` in both `complete()` and `stream_completion()`
