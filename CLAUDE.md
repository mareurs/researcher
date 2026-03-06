# Researcher — Claude Code Instructions

Fast AI research agent in Rust. Two binaries: `researcher` (HTTP server + CLI) and `researcher-mcp` (MCP stdio server).

## Build & Run

```bash
cargo build --release                        # both binaries
cargo build --release --bin researcher       # HTTP server only
cargo build --release --bin researcher-mcp   # MCP server only
cargo check                                  # fast type-check, no link
```

Release profile uses LTO + strip — binary is ~6-7MB.

## Running Locally (no Docker)

```bash
# Assumes llama.cpp on :8080 and SearXNG on :4000
LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
RUST_LOG=info \
cargo run --bin researcher -- --query "your topic" --output report.md

# Server mode
cargo run --bin researcher -- --server --bind-addr 0.0.0.0:3000
```

## Docker Stack

```bash
cp .env.example .env   # edit LLM/search settings

# Local GPU (llama-cpp + tei-embed + searxng + researcher)
docker compose --profile local-llm up

# OpenAI (searxng + researcher only)
OPENAI_API_KEY=sk-... docker compose up
```

Services: `searxng` (:4000), `llama-cpp` (:8080, profile `local-llm`), `tei-embed` (:8081), `researcher` (:3000).

## Project Structure

```
src/
├── main.rs              — CLI entry + server entry (Tokio #[main])
├── mcp_server.rs        — MCP stdio binary (rmcp, tool_router)
├── config.rs            — Config struct (clap + env vars)
├── server.rs            — Axum router: GET /, POST /research, POST /research/stream
├── llm/
│   ├── client.rs        — LlmClient: OpenAI-compat HTTP, blocking complete()
│   └── stream.rs        — stream_completion(): SSE token streaming via mpsc
├── search/
│   ├── mod.rs           — search_with_fallback() (SearXNG → DuckDuckGo)
│   ├── searxng.rs       — SearXNG JSON API client
│   └── duckduckgo.rs    — DDG Lite HTML scraper (fallback)
├── scraper/
│   └── html.rs          — fetch_and_extract(): reqwest + scraper HTML→text
├── embeddings/
│   ├── client.rs        — EmbedClient: TEI /embed batch API
│   └── dedup.rs         — deduplicate() + rank_by_relevance() (cosine sim)
└── researcher/
    ├── pipeline.rs      — run(): full pipeline, ProgressEvent, token_tx
    ├── planner.rs       — generate_queries(): LLM → sub-questions
    ├── crawler.rs       — crawl_all(): search + parallel scrape
    ├── summarizer.rs    — summarize_all(): concurrent LLM summaries
    └── publisher.rs     — write_report(): blocking or streaming report
static/
└── index.html           — SSE streaming web UI (served at GET /)
```

## Pipeline Flow

```
query → planner (LLM) → [search+scrape]×N → embed-dedup+rerank → [summarize]×M (concurrent) → publisher (LLM)
                                                     ↑                      ↑
                                               optional TEI          join_all (parallel)
```

`run()` signature: `run(cfg, topic, on_progress: Fn(ProgressEvent), token_tx: Option<Sender<String>>)`
- `token_tx = None` → blocking `complete()` for all LLM calls (MCP, JSON API)
- `token_tx = Some(tx)` → `stream_completion()` for the final report (CLI, SSE)

## Key Design Decisions

**LLM backend**: Any OpenAI-compatible `/v1/chat/completions`. Switch with `LLM_BASE_URL`.
- Local: `llama.cpp server` (CUDA image `ghcr.io/ggml-org/llama.cpp:server-cuda`)
- Model: `Qwen_Qwen3.5-9B-Q4_K_M.gguf` (matches gpt-researcher's Qwen3.5-9B)
- Cloud: `https://api.openai.com/v1` + `OPENAI_API_KEY`

**Thinking tokens**: `STRIP_THINKING_TOKENS=true` strips `<think>...</think>` from all Qwen3 responses in `LlmClient::complete()` and `stream_completion()`.

**Deduplication**: TEI (`BAAI/bge-large-en-v1.5`) embeds sources → cosine similarity → drop duplicates above `DEDUP_THRESHOLD` → rerank by similarity to original query.

**Search fallback**: SearXNG → DuckDuckGo Lite (automatic, no config needed).

**MCP server**: stdio transport, `#[tool_router]` / `#[tool]` macros from `rmcp = "1.1"`. No streaming — returns complete report. All logging to stderr.

## Codescout Rules (enforced by hooks)

- **Never `Read` source files** — use `list_symbols` + `find_symbol(include_body=true)`
- **Never `edit_file` for structural changes** — use `replace_symbol`, `insert_code`, `remove_symbol`
- **Structural edits** = anything changing function bodies, adding methods, struct fields
- **`edit_file` is OK for** imports, string literals, comments, single-line changes

## Env Vars Reference

| Variable | Default | Notes |
|----------|---------|-------|
| `LLM_BASE_URL` | `http://localhost:8080/v1` | Any OpenAI-compat endpoint |
| `LLM_MODEL` | `Qwen_Qwen3.5-9B-Q4_K_M` | Model name sent in requests |
| `LLM_API_KEY` | `no-key-needed` | Set to `sk-...` for OpenAI |
| `LLM_MAX_TOKENS` | `2048` | Max tokens per LLM call |
| `LLM_TEMPERATURE` | `0.3` | Generation temperature |
| `STRIP_THINKING_TOKENS` | `true` | Strip `<think>` from Qwen3 |
| `SEARXNG_URL` | `http://localhost:4000` | SearXNG instance |
| `SEARCH_RESULTS_PER_QUERY` | `8` | Results fetched per sub-question |
| `EMBED_BASE_URL` | `` (disabled) | TEI URL; empty = skip dedup |
| `DEDUP_THRESHOLD` | `0.92` | Cosine sim cutoff for dedup |
| `MAX_SEARCH_QUERIES` | `4` | Sub-questions from planner |
| `MAX_SOURCES_PER_QUERY` | `4` | Pages scraped per query |
| `MAX_PAGE_CHARS` | `8000` | Max chars extracted per page |
| `RUST_LOG` | `info` | Tracing filter |
| `BIND_ADDR` | `0.0.0.0:3000` | HTTP server bind address |
| `RESEARCH_MODE`   | `report` | Research depth: quick, summary, report, deep |
| `DOMAIN_PROFILE`  | `` | Named profile from profiles.toml (e.g. shopping-ro) |
| `DOMAINS`         | `` | Comma-separated domain override |
| `LINKEDIN_COOKIE` | `` | Cookie header for linkedin.com (optional auth) |
| `FB_COOKIE` | `` | Cookie header for facebook.com (optional auth) |
| `INSTAGRAM_COOKIE` | `` | Cookie header for instagram.com (optional auth) |
| `TWITTER_COOKIE` | `` | Cookie header for twitter.com / x.com (optional auth) |

## Model Download

```bash
# Download Qwen3.5-9B GGUF (bartowski quant — same model as gpt-researcher)
huggingface-cli download bartowski/Qwen_Qwen3.5-9B-GGUF \
  --include "Qwen_Qwen3.5-9B-Q4_K_M.gguf" \
  --local-dir ./models/

# Mount into Docker volume
docker run --rm \
  -v researcher_llama-models:/models \
  -v ./models:/src alpine \
  cp /src/Qwen_Qwen3.5-9B-Q4_K_M.gguf /models/
```

## MCP Config (Claude Desktop / Claude Code)

```json
{
  "mcpServers": {
    "researcher": {
      "command": "/path/to/researcher-mcp",
      "env": {
        "LLM_BASE_URL": "http://localhost:8080/v1",
        "SEARXNG_URL": "http://localhost:4000",
        "STRIP_THINKING_TOKENS": "true"
      }
    }
  }
}
```
