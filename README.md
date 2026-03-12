# Researcher

> Fast AI research agent in Rust — plans sub-questions, searches the web, scrapes sources in parallel, and writes a comprehensive markdown report.

```
query → planner (LLM) → search+scrape×N → quality filter → dedup → rerank → summarize×M → report (LLM)
```

**Why Rust?** No GIL — true parallel scraping, concurrent LLM summarization, ~5MB static binary, zero LangChain.

## Features

- **Multi-stage pipeline** — LLM-driven query planning, parallel web crawling, concurrent summarization, final report synthesis
- **Any OpenAI-compatible LLM** — local (llama.cpp, Ollama, vLLM) or cloud (OpenAI, Anthropic via LiteLLM)
- **Dual-model routing** — route cheap structured tasks (planner, summarizer) to a fast small model; reserve the large model for final report generation
- **Semantic deduplication** — TEI embeddings + cosine similarity drop near-duplicate sources before summarization
- **Cross-encoder reranking** — `ms-marco-MiniLM` scores and reranks sources by relevance, authority, and content quality
- **Domain profiles** — pin searches to curated source lists (tech-news, academic, llm-news, shopping, travel, news)
- **6 MCP tools** — `research`, `research_person`, `research_company`, `research_code`, `search_jobs`, `market_insight`
- **Streaming HTTP API** — SSE token stream for the web UI; blocking JSON for MCP and programmatic use
- **Job search** — finds remote jobs matching your `profiles.toml` preferences, with optional deep company briefs

## Architecture

```
topic
  │
  ▼
Planner (LLM) ──── generates N sub-questions
  │
  ▼
Crawler (parallel per query)
  ├─ SearXNG search (→ DuckDuckGo fallback)
  └─ scrape URLs concurrently (reqwest + scraper crate)
  │
  ▼
Quality filter ──── min word count, text density
  │
  ▼
Dedup (TEI embed → cosine sim) ──── optional, requires EMBED_BASE_URL
  │
  ▼
Cross-encoder rerank (TEI) ──── optional, requires RERANK_BASE_URL
  │
  ▼
Summarizer (LLM, join_all — all calls concurrent)
  │
  ▼
Publisher (LLM) ──── final markdown report / streaming tokens
```

Two binaries:
- **`researcher`** — HTTP server (`POST /research`, `POST /research/stream`, `GET /`) + CLI (`--query`)
- **`researcher-mcp`** — MCP stdio server for Claude Desktop / Claude Code

## Requirements

| Component | Required | Notes |
|-----------|----------|-------|
| **Rust 1.80+** | For building from source | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **Docker + Compose** | For the full stack | v2.20+ recommended |
| **LLM backend** | One of below | |
| └ NVIDIA GPU | For local llama.cpp | Any CUDA-capable card with ≥8GB VRAM |
| └ AMD GPU | For ROCm llama.cpp | RDNA2/RDNA3, kernel 6.x |
| └ OpenAI API key | Cloud alternative | No GPU needed |
| └ Ollama | Local alternative | CPU or GPU |
| **SearXNG** | Bundled in infra stack | Private metasearch engine |
| **TEI** (optional) | For dedup + reranking | CPU-only images work fine |

## Quick Start

### Option A — Full local stack (NVIDIA GPU)

```bash
# 1. Clone and configure
git clone https://github.com/your-org/researcher.git
cd researcher
cp .env.example .env
cp infra/.env.example infra/.env
# Edit infra/.env: set LLAMA_MODELS_PATH to where your GGUF files live

# 2. Download a model (example: Qwen3.5-9B, ~6GB)
huggingface-cli download bartowski/Qwen_Qwen3.5-9B-GGUF \
  --include "Qwen_Qwen3.5-9B-Q4_K_M.gguf" \
  --local-dir /path/to/your/models/bartowski/Qwen_Qwen3.5-9B-GGUF/

# 3. Start infrastructure (llama-cpp + SearXNG + TEI embed + TEI rerank)
make infra-up

# 4. Start the researcher service
make up

# 5. Research something
curl -X POST http://localhost:33100/research \
  -H 'Content-Type: application/json' \
  -d '{"query": "What are the latest advances in fusion energy?"}'
```

Web UI with token streaming: http://localhost:33100/

### Option B — OpenAI

```bash
cp .env.example .env
# Edit .env:
#   LLM_BASE_URL=https://api.openai.com/v1
#   LLM_MODEL=gpt-4o-mini
#   LLM_API_KEY=sk-...
#   LLM_FAST_BASE_URL=   (leave empty — use same backend for all stages)

# Start infra (SearXNG only; llama-cpp is not required)
make infra-up
make up

curl -X POST http://localhost:33100/research \
  -H 'Content-Type: application/json' \
  -d '{"query": "Impact of quantum computing on cryptography"}'
```

### Option C — CLI (no Docker)

```bash
cargo build --release

# Run against a local llama.cpp + SearXNG
LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
RUST_LOG=info \
./target/release/researcher --query "Rust async runtime internals"

# Save report to file
./target/release/researcher --query "..." --output report.md
```

### Option D — Ollama

```bash
# In .env:
LLM_BASE_URL=http://host.docker.internal:11434/v1
LLM_MODEL=qwen2.5:7b
LLM_API_KEY=ollama
LLM_FAST_BASE_URL=   # empty = same model for all stages
```

## MCP Server

`researcher-mcp` exposes the full pipeline as MCP tools over stdio. Use with Claude Desktop, Claude Code, or any MCP client.

```bash
cargo build --release --bin researcher-mcp
# → target/release/researcher-mcp  (~6MB)
```

### Claude Desktop (`~/.config/claude/claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "researcher": {
      "command": "/path/to/researcher-mcp",
      "env": {
        "LLM_BASE_URL": "http://localhost:8080/v1",
        "LLM_MODEL": "Qwen_Qwen3.5-9B-Q4_K_M",
        "SEARXNG_URL": "http://localhost:4000",
        "STRIP_THINKING_TOKENS": "true",
        "EMBED_BASE_URL": "http://localhost:8081",
        "RERANK_BASE_URL": "http://localhost:8082"
      }
    }
  }
}
```

### Claude Code (`.mcp.json`)

```json
{
  "mcpServers": {
    "researcher": {
      "command": "/path/to/researcher-mcp",
      "env": {
        "LLM_BASE_URL": "http://localhost:8080/v1",
        "SEARXNG_URL": "http://localhost:4000"
      }
    }
  }
}
```

### MCP Tools

| Tool | Parameters | Description |
|------|-----------|-------------|
| `research` | `query`, `mode?`, `domain_profile?`, `domains?`, `max_queries?`, `max_sources?` | General web research → markdown report |
| `research_person` | `name`, `method?` | Meeting prep brief (career, voice, interests, hooks). `method`: professional\|personal\|both |
| `research_company` | `name`, `country?` | Company brief (what they do, size, news, culture, strategy) |
| `research_code` | `framework`, `version?`, `aspects?`, `repo?`, `query?` | Library research: bugs, changelog, community sentiment |
| `search_jobs` | `query`, `mode?` | Remote job search matched to your `profiles.toml [job-profile]`. `mode`: list\|deep |
| `market_insight` | `query`, `asset_class?`, `mode?` | Stock/crypto/macro research. `asset_class`: stock\|crypto\|macro |

**Research modes:** `quick` (snippets), `summary` (bullets), `report` (full markdown, default), `deep` (thorough)

## HTTP API

### `POST /research`

Blocking — waits for the full report.

```bash
curl -X POST http://localhost:33100/research \
  -H 'Content-Type: application/json' \
  -d '{
    "query": "Rust async runtimes compared",
    "mode": "report",
    "max_queries": 4,
    "max_sources": 4
  }'
```

Response:
```json
{
  "topic": "Rust async runtimes compared",
  "queries": ["What is Tokio?", "..."],
  "source_count": 14,
  "report": "# Research Report\n\n..."
}
```

### `POST /research/stream`

SSE token stream — progress events then report tokens.

```bash
curl -X POST http://localhost:33100/research/stream \
  -H 'Content-Type: application/json' \
  -d '{"query": "history of the internet"}' \
  --no-buffer
```

Events:
```
data: {"type":"progress","message":"🔍 Planning research queries..."}
data: {"type":"progress","message":"📋 Generated 4 search queries","data":{"queries":[...]}}
data: {"type":"progress","message":"🌐 Crawling 4 queries in parallel..."}
data: {"type":"token","token":"# Research Report\n\n"}
...
event: complete
data: {"type":"complete","topic":"...","report":"# Research Report\n\n..."}
```

### `GET /health`

Returns `200 ok`.

## Configuration

All settings are environment variables. Copy `.env.example` to `.env` and edit.

### LLM

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_BASE_URL` | `http://localhost:8080/v1` | Any OpenAI-compatible endpoint |
| `LLM_MODEL` | `Qwen_Qwen3.5-9B-Q4_K_M` | Model name sent in requests |
| `LLM_API_KEY` | `no-key-needed` | Set to `sk-...` for OpenAI |
| `LLM_MAX_TOKENS` | `4096` | Max tokens per LLM call |
| `LLM_TEMPERATURE` | `0.3` | Generation temperature |
| `STRIP_THINKING_TOKENS` | `true` | Strip `<think>...</think>` from Qwen3 responses |

### Dual-model routing (optional)

Route cheap structured tasks (planner, summarizer) to a fast small model, reserving the large model for final report generation. Leave `LLM_FAST_BASE_URL` empty to use a single backend for everything.

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_FAST_BASE_URL` | `` (disabled) | Fast LLM endpoint; empty = use heavy backend |
| `LLM_FAST_MODEL` | `Qwen3.5-4B-Q4_K_M` | Model name for fast LLM |
| `LLM_FAST_API_KEY` | `` | Fast LLM API key; empty = inherit `LLM_API_KEY` |
| `LLM_FAST_MAX_TOKENS` | `4096` | Max tokens for fast model |
| `LLM_FAST_STAGES` | `planner,summarizer,publisher` | Pipeline stages routed to fast LLM |

Valid stage names: `planner`, `summarizer`, `publisher`

### Search & crawling

| Variable | Default | Description |
|----------|---------|-------------|
| `SEARXNG_URL` | `http://localhost:4000` | SearXNG instance URL |
| `SEARCH_RESULTS_PER_QUERY` | `8` | Results fetched per sub-question |
| `MAX_SEARCH_QUERIES` | `4` | Sub-questions the planner generates |
| `MAX_SOURCES_PER_QUERY` | `4` | Pages scraped per query |
| `MAX_PAGE_CHARS` | `8000` | Max characters extracted per page |

### Embeddings & reranking (optional)

Both are disabled when their `*_BASE_URL` is empty — the pipeline skips those stages gracefully.

| Variable | Default | Description |
|----------|---------|-------------|
| `EMBED_BASE_URL` | `` (disabled) | TEI embed endpoint (e.g. `http://localhost:8081`) |
| `DEDUP_THRESHOLD` | `0.92` | Cosine similarity cutoff for deduplication |
| `RERANK_BASE_URL` | `` (disabled) | TEI rerank endpoint (e.g. `http://localhost:8082`) |
| `RERANK_RELEVANCE_WEIGHT` | `0.7` | Cross-encoder score weight |
| `RERANK_AUTHORITY_WEIGHT` | `0.2` | Domain authority weight |
| `RERANK_QUALITY_WEIGHT` | `0.1` | Content quality weight |

### Quality filter

| Variable | Default | Description |
|----------|---------|-------------|
| `MIN_CONTENT_WORDS` | `100` | Minimum word count per page |
| `MIN_TEXT_DENSITY` | `0.05` | Minimum text/HTML density ratio |

### Auth (optional — for gated sources)

| Variable | Description |
|----------|-------------|
| `LINKEDIN_COOKIE` | Cookie header for linkedin.com |
| `TWITTER_COOKIE` | Cookie header for twitter.com / x.com |
| `FB_COOKIE` | Cookie header for facebook.com |
| `INSTAGRAM_COOKIE` | Cookie header for instagram.com |
| `ADZUNA_APP_ID` | Adzuna API app ID (job search) — free at developer.adzuna.com |
| `ADZUNA_APP_KEY` | Adzuna API key |
| `ADZUNA_COUNTRY` | `us` — Adzuna country code (us, gb, de, fr, …) |

### Server

| Variable | Default | Description |
|----------|---------|-------------|
| `BIND_ADDR` | `0.0.0.0:3000` | HTTP server bind address |
| `RUST_LOG` | `info` | Log level filter |

## Domain Profiles

`profiles.toml` defines named source lists. Pass `domain_profile="tech-news"` to any tool or API call to restrict searches to those domains. Profiles can be combined with a raw `domains` list — they are unioned.

Built-in profiles:

| Profile | Sources |
|---------|---------|
| `tech-news` | Hacker News, lobste.rs, r/programming, r/rust, r/technology |
| `llm-news` | HuggingFace, arXiv, r/LocalLLaMA, r/MachineLearning |
| `academic` | arXiv, Semantic Scholar, PubMed |
| `news` | BBC, Reuters, r/worldnews, r/news, r/europe |
| `travel` | TripAdvisor, Lonely Planet, Wikivoyage, r/travel |
| `shopping-ro` | OLX.ro, eMag.ro, Altex.ro (Romanian market) |

Add custom profiles in `profiles.toml`:

```toml
[my-profile]
domains = ["example.com", "docs.example.com"]
```

## Job Search

Configure your profile in `profiles.toml` under `[job-profile]`:

```toml
[job-profile]
title = "Senior AI Engineer"
seniority = "senior"
salary_floor = "150000 USD"
remote_only = true
skills = ["Rust", "Python", "LLMs", "MLOps", "RAG"]
preferred_company_size = "startup to mid-size"
avoid_industries = ["gambling", "crypto"]
about_me = """
Brief summary of your background and what you're looking for.
"""
```

Then call `search_jobs` via MCP or HTTP. Use `mode: "deep"` for full company briefs on the top 5 matches.

## Infrastructure Stack

The project uses a two-compose layout to keep the AI infrastructure reusable across projects:

```
infra/docker-compose.yml   ← always-on: SearXNG, llama-cpp, TEI embed, TEI rerank
docker-compose.yml         ← researcher app only (joins ai-infra-net)
```

```bash
# Start infrastructure first
make infra-up

# Then start the researcher app
make up

# Logs
make infra-logs   # infrastructure services
make logs         # researcher app

# Stop everything
make stop-all
```

### Services

| Service | Port | Description |
|---------|------|-------------|
| `searxng` | 4000 | Private metasearch (Google/DDG/Bing, optionally via Tor) |
| `llama-cpp` | 30080 | Heavy LLM — NVIDIA GPU (llama.cpp CUDA image) |
| `llama-cpp-fast` | 30081 | Fast LLM — AMD GPU via ROCm, or second card |
| `tei-embed` | 8081 | `BAAI/bge-large-en-v1.5` embeddings (CPU) |
| `tei-rerank` | 8082 | `cross-encoder/ms-marco-MiniLM-L-6-v2` reranker (CPU) |
| `researcher` | 33100 | Researcher HTTP server |

The infra stack creates a shared Docker network `ai-infra-net`. Other projects can join it and reuse the LLM and search services without running their own copies.

### Single-GPU setup

Set `LLM_FAST_BASE_URL=` (empty) in `.env`. All pipeline stages use the same `llama-cpp` backend.

### AMD GPU (ROCm)

`llama-cpp-fast` uses the ROCm image and targets `/dev/kfd` + `/dev/dri`. Works with RDNA2/RDNA3 on kernel 6.x.

## Building from Source

```bash
# Prerequisites (Debian/Ubuntu)
sudo apt-get install pkg-config libssl-dev

# Type-check only (fast)
cargo check

# Build both binaries (optimized — ~30-60s with LTO)
cargo build --release

# Lint
cargo clippy -- -D warnings
```

Binaries:
- `target/release/researcher` — HTTP server + CLI
- `target/release/researcher-mcp` — MCP stdio server (~6MB)

### Docker image

```bash
docker build -t researcher .
# Multi-stage build: rust:slim builder → distroless runtime (~8MB total)
```

## LLM Backend Compatibility

| Backend | `LLM_BASE_URL` | Notes |
|---------|----------------|-------|
| **llama.cpp** | `http://localhost:8080/v1` | Recommended local; CUDA/ROCm/CPU images available |
| **Ollama** | `http://localhost:11434/v1` | Easy model management |
| **vLLM** | `http://localhost:8000/v1` | Best for multi-user / high concurrency |
| **LM Studio** | `http://localhost:1234/v1` | Desktop GUI for local models |
| **OpenAI** | `https://api.openai.com/v1` | Set `LLM_API_KEY=sk-...` |
| **Anthropic** | Use [LiteLLM](https://github.com/BerriAI/litellm) proxy | OpenAI-compatible wrapper |

## Recommended Models

| Use case | Model | VRAM |
|----------|-------|------|
| Heavy (reports) | `Qwen3.5-27B-Q4_K_M` | ~18GB |
| Heavy (reports) | `Qwen3.5-9B-Q4_K_M` | ~6GB |
| Fast (planner/summarizer) | `Qwen3.5-4B-Q4_K_M` | ~3GB |
| Cloud | `gpt-4o-mini` | — |

Set `STRIP_THINKING_TOKENS=true` for all Qwen3 models to strip internal `<think>` tokens from responses.

## License

MIT
