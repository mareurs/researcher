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

## Testing via MCP After Code Changes

The `researcher-mcp` binary is loaded once when the MCP server starts. After modifying source code:

1. `cargo build --release` — wait for it to finish (LTO takes ~30-60s)
2. Restart the MCP server — in Claude Code: `/mcp` → restart, or restart the session
3. The new binary will be picked up on next tool call

**Do not test via the `mcp__researcher__*` tools without rebuilding first** — you'll be running the old binary.

## Running Locally (no Docker)

```bash
# Vertex/Gemini backend (live config) — run `gcloud auth application-default login` first
LLM_PROVIDER=vertex \
VERTEX_PROJECT=sp-mrv-gemini-dev \
VERTEX_LOCATION=eu \
LLM_MODEL=gemini-3.1-flash-lite \
SEARXNG_URL=http://localhost:4000 \
RUST_LOG=info \
cargo run --bin researcher -- --query "your topic" --output report.md

# OpenAI-compatible backend (e.g. local llama.cpp on :30080)
LLM_PROVIDER=openai \
LLM_BASE_URL=http://localhost:30080/v1 \
SEARXNG_URL=http://localhost:4000 \
cargo run --bin researcher -- --query "your topic" --output report.md

# Server mode
cargo run --bin researcher -- --server --bind-addr 0.0.0.0:3000
```

## Docker Stack

Two independent stacks, coordinated over the external `ai-infra-net` Docker
network. Bring infra up first, then the app.

```bash
cp .env.example .env   # edit LLM/search settings (live config uses Vertex)

# 1. Shared AI infra (searxng, tei-embed, tei-rerank, optional llama-cpp GPU)
make infra-up          # bootstraps config/searxng/settings.yml, then compose up -d

# 2. Researcher app
make up                # docker compose up -d

make stop-all          # tear both down (keeps volumes)
```

Infra services (`infra/docker-compose.yml`): `searxng` (:4000), `tei-embed` (:30082), `tei-rerank` (:30083), `llama-cpp` (:30080, GPU), `llama-cpp-fast` (:30081, GPU). App service (root `docker-compose.yml`): `researcher` (:3000).

With the live Vertex backend the `llama-cpp*` GPU services are unused — only `searxng` + `tei-embed` + `tei-rerank` are needed.

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
query → planner (LLM) → [search+scrape]×N → quality filter → embed-dedup → cross-encoder rerank → [summarize+judge]×M → publisher (LLM)
```

`run()` signature: `run(cfg, topic, on_progress: Fn(ProgressEvent), token_tx: Option<Sender<String>>)`
- `token_tx = None` → blocking `complete()` for all LLM calls (MCP, JSON API)
- `token_tx = Some(tx)` → `stream_completion()` for the final report (CLI, SSE)

## Key Design Decisions

**LLM backend**: Provider-agnostic, selected by `LLM_PROVIDER` (`openai` | `vertex`).
- **Vertex (live-config default)**: Google Vertex AI Gemini. Set `LLM_PROVIDER=vertex`, `VERTEX_PROJECT`, `VERTEX_LOCATION` (live: project `sp-mrv-gemini-dev`, location `eu`, model `gemini-3.1-flash-lite`). Base URL defaults to `https://aiplatform.googleapis.com/v1beta1`. Auth uses Application Default Credentials — run `gcloud auth application-default login` (no API key in env). See `src/llm/vertex.rs`.
- **OpenAI-compatible**: any `/v1/chat/completions` endpoint. `LLM_PROVIDER=openai` (default) + `LLM_BASE_URL`. Local option: `llama.cpp server` (CUDA image `ghcr.io/ggml-org/llama.cpp:server-cuda`, e.g. Qwen3.5); cloud: `https://api.openai.com/v1` + `OPENAI_API_KEY` (via `LLM_API_KEY`).

**Thinking tokens**: `STRIP_THINKING_TOKENS=true` strips `<think>...</think>` from Qwen3 responses in `LlmClient::complete()` and `stream_completion()` (no-op for Gemini/OpenAI models that don't emit them).

**Deduplication**: TEI (`BAAI/bge-large-en-v1.5`) embeds sources → cosine similarity → drop duplicates above `DEDUP_THRESHOLD` → rerank by similarity to original query.

**Search fallback**: profile primary (Brave when `BRAVE_API_KEY` set / Tavily for `news` / Exa for `academic`) → SearXNG → DuckDuckGo Lite. First non-empty result wins; see `search_with_fallback` in `src/search/mod.rs`.

**MCP server**: stdio transport, `#[tool_router]` / `#[tool]` macros from `rmcp = "1.1"`. No streaming — returns complete report. All logging to stderr.

## Bug Tracking

Per-file bug tracking lives in `docs/issues/`. Every bug noticed during work gets its
own file, copied from `docs/issues/_TEMPLATE.md` — that template carries the status
vocabulary, the trigger rules, and the archive flow, so it is the one place to read
before filing.

Two behaviors are load-bearing and easy to skip:

- **Capture on notice** — add the file the moment a bug is noticed (wrong output,
  silent failures, misleading errors, tool quirks), not at task end.
- **Archive through the librarian** — `doc(action="move", id=..., new_rel_path=...)`,
  never a bare `git mv`. `id = sha256(abs_path)`, so a hand-move leaves the catalog row
  pointing at a path that no longer exists.

List what's open:

```
doc(action="find", kind="bug",
    filter={"status": {"in": ["open", "investigating"]}})
```

`researcher` belongs to the `codescout-ecosystem` librarian umbrella, so `scope="umbrella"`
surfaces these files from a codescout session and codescout's from here. Membership lives
in a per-machine registry outside every repo and is deliberately not named in this file —
run a `scope="umbrella"` query and read the returned `scope` block to see what actually
resolves on the host you are standing on.

Before 2026-08-08 researcher had no issues directory, and its bugs were filed in
codescout's tracker for want of anywhere better. One file was relocated when this
directory was created. If you find a researcher bug still filed over there, move it here.

## MCP Server Instructions Rule

**Every time a new tool is added to `researcher-mcp`, update `get_info()` in `src/mcp_server.rs`** to add a one-line bullet for the new tool in the `with_instructions(...)` block. The instructions must always enumerate all available tools with their signatures and key parameters so the LLM host can pick the right tool without guessing.

## Rust Coding Standards

### Ownership first
- Restructure data and function signatures to satisfy the borrow checker.
  Never reach for `.clone()` as a first resort — only clone when semantically
  appropriate (the clone represents intentional duplication, not a workaround).
- Design ownership topology before writing implementations. Ask: who owns this
  data, and what borrows it?

### Error handling
- All fallible functions return `Result<T, E>`. Never use `.unwrap()` in
  library code. In application code, only use `.unwrap()` where a panic is
  genuinely the correct behavior and document why.
- Use `anyhow` for application-level error propagation, `thiserror` for
  library error types.
- Propagate errors with `?`. Avoid nested match blocks for error handling.

### Iterators over loops
- Prefer iterator adapters (`.map()`, `.filter()`, `.fold()`, `.collect()`)
  over explicit `for` loops when the intent is a transformation pipeline.
- Use `.iter()` / `.iter_mut()` / `.into_iter()` correctly — do not
  implicitly rely on auto-deref behavior.

### Generics vs trait objects
- Default to generics (`fn foo<T: Trait>(x: T)`) for zero-cost dispatch.
- Only use `Box<dyn Trait>` / `Arc<dyn Trait>` when you need runtime
  polymorphism (heterogeneous collections, plugin systems, dynamic dispatch).
  Always comment why dynamic dispatch is needed.

### Concurrency
- Before reaching for `Arc<Mutex<T>>`, ask whether ownership can be
  structured to avoid shared state entirely.
- Prefer message passing (channels) over shared state for complex coordination.
- Use `tokio` for async I/O. Never block inside async functions.

### Clippy
- All code must pass `cargo clippy -- -D warnings`.
- When clippy suggests an alternative, prefer it unless there's a documented
  reason not to.

### Lifetimes
- Annotate lifetimes explicitly when the compiler cannot infer them and
  explain the relationship in a comment.
- Prefer owned types in struct fields over references where the struct
  needs to be self-contained or sent across threads.

### unsafe
- Never write `unsafe` without a `// SAFETY:` comment explaining the
  invariants being upheld.
- Prefer safe abstractions. Reach for `unsafe` only when there is no
  safe alternative and the performance gain is measured and documented.

## Codescout Rules (enforced by hooks)

- **Never `Read` source files** — use `symbols(path=...)` for an overview, `symbols(name=..., include_body=true)` for bodies
- **Never `edit_file` for structural changes** — use `edit_code` (`action=replace|insert|remove|rename`)
- **Structural edits** = anything changing function bodies, adding methods, struct fields
- **`edit_file` is OK for** imports, string literals, comments, single-line changes

## Env Vars Reference

| Variable | Default | Notes |
|----------|---------|-------|
| `LLM_PROVIDER` | `openai` | Primary LLM backend: `openai` \| `vertex` (live config uses `vertex`) |
| `LLM_BASE_URL` | `http://localhost:8080/v1` | OpenAI-compat endpoint; for `vertex` defaults to `https://aiplatform.googleapis.com/v1beta1` |
| `LLM_MODEL` | `Qwen_Qwen3.5-9B-Q4_K_M` | Model name sent in requests (live vertex: `gemini-3.1-flash-lite`) |
| `LLM_API_KEY` | `no-key-needed` | Set to `sk-...` for OpenAI; unused for `vertex` (uses ADC) |
| `VERTEX_PROJECT` | `` | GCP project id — required when `LLM_PROVIDER=vertex` (live: `sp-mrv-gemini-dev`) |
| `VERTEX_LOCATION` | `us-central1` | Vertex AI region (live: `eu`) |
| `LLM_MAX_TOKENS` | `4096` | Max tokens per LLM call |
| `LLM_TEMPERATURE` | `0.3` | Generation temperature |
| `STRIP_THINKING_TOKENS` | `true` | Strip `<think>` from Qwen3 |
| `LLM_FAST_PROVIDER` | `` | Provider for fast LLM; empty = inherit `LLM_PROVIDER` |
| `LLM_FAST_BASE_URL` | `` (disabled) | Fast LLM endpoint; empty = use heavy backend |
| `LLM_FAST_MODEL` | `Qwen3.5-4B-Q4_K_M` | Model name for fast LLM |
| `LLM_FAST_API_KEY` | `` | Fast LLM API key; empty = use `LLM_API_KEY` |
| `LLM_FAST_MAX_TOKENS` | `4096` | Max tokens for fast LLM responses |
| `LLM_FAST_STAGES` | `planner,summarizer,publisher` | Comma-separated pipeline stages using fast LLM (planner, summarizer, publisher) |
| `BRAVE_API_KEY` | `` | Brave Search API key; empty disables Brave |
| `TAVILY_API_KEY` | `` | Tavily API key; used for "news" profile (empty = disabled) |
| `EXA_API_KEY` | `` | Exa API key; used for "academic" profile (empty = disabled) |
| `SEARXNG_URL` | `http://localhost:4000` | SearXNG instance |
| `SEARCH_RESULTS_PER_QUERY` | `8` | Results fetched per sub-question |
| `EMBED_BASE_URL` | `` (disabled) | TEI URL; empty = skip dedup |
| `DEDUP_THRESHOLD` | `0.92` | Cosine sim cutoff for dedup |
| `RERANK_BASE_URL` | `` (disabled) | TEI cross-encoder URL; empty = skip reranking |
| `RERANK_RELEVANCE_WEIGHT` | `0.7` | Cross-encoder score weight in combined ranking |
| `RERANK_AUTHORITY_WEIGHT` | `0.2` | Domain authority weight in combined ranking |
| `RERANK_QUALITY_WEIGHT` | `0.1` | Content quality weight in combined ranking |
| `MIN_CONTENT_WORDS` | `100` | Quality filter: minimum word count |
| `MIN_TEXT_DENSITY` | `0.05` | Quality filter: minimum text/HTML density ratio |
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
| `ADZUNA_APP_ID`  | `` | Adzuna API app ID (free tier at developer.adzuna.com) |
| `ADZUNA_APP_KEY` | `` | Adzuna API app key |
| `ADZUNA_COUNTRY` | `us` | Adzuna API country code (us, gb, de, fr, etc.) |
| `LANGFUSE_HOST` | `` (disabled) | Langfuse base URL; empty disables tracing |
| `LANGFUSE_PUBLIC_KEY` | `` | Langfuse project public key |
| `LANGFUSE_SECRET_KEY` | `` | Langfuse project secret key |
| `LANGFUSE_MAX_PAYLOAD_CHARS` | `50000` | Truncate captured LLM/stage I/O |
| `LLM_JUDGE_ENABLED` | `false` | Opt-in LLM-judge scoring of the final report (relevance/coverage/faithfulness/citation-validity) → Langfuse scores |
| `PROGRESSIVE_DISCLOSURE_CHARS` | `4000` | Report size (chars) above which a compact envelope (summary/toc/path) replaces the inline report; `0` disables disclosure |
| `MCP_IDLE_TIMEOUT_SECS` | `0` (disabled) | `researcher-mcp` only: self-exit after N seconds with no inbound client traffic and no in-flight tool call. Reaps servers stranded when an MCP client restarts/reconnects without closing the old stdio socket. `0` = never self-exit |

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
        "LLM_PROVIDER": "vertex",
        "LLM_MODEL": "gemini-3.1-flash-lite",
        "VERTEX_PROJECT": "sp-mrv-gemini-dev",
        "VERTEX_LOCATION": "eu",
        "SEARXNG_URL": "http://localhost:4000",
        "BRAVE_API_KEY": "<brave-search-api-key>",
        "EMBED_BASE_URL": "http://localhost:30082",
        "RERANK_BASE_URL": "http://localhost:30083"
      }
    }
  }
}
```

Vertex uses Application Default Credentials — run `gcloud auth application-default login` on the host so the MCP process can mint access tokens (no API key needed in env).
