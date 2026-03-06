# Researcher

A fast AI research agent written in Rust. Give it a topic; it plans sub-questions, searches the web, scrapes sources in parallel, summarizes each one concurrently, and produces a comprehensive markdown report.

**Improvements over gpt-researcher:**
- No Python GIL — true parallel scraping and concurrent LLM summarization via Tokio
- ~5MB static binary vs ~500MB Python + dependency image
- Zero LangChain — direct async HTTP to any OpenAI-compatible endpoint
- Swap LLM backend with one env var: local GPU ↔ OpenAI ↔ Ollama

## Architecture

```
topic
  │
  ▼
Planner (LLM) ──── generates N sub-questions
  │
  ▼
Crawler (parallel per query)
  ├─ SearXNG search
  └─ scrape URLs concurrently (reqwest + scraper)
  │
  ▼
Summarizer (join_all — all LLM calls concurrent)
  │
  ▼
Publisher (LLM) ──── final markdown report
```

## Quick Start

### Prerequisites

- Docker + Docker Compose
- NVIDIA GPU with drivers (for local LLM) — or an OpenAI API key

### Local GPU (recommended)

```bash
cp .env.example .env

# Download a model into the shared volume first:
docker run --rm -v researcher_llama-models:/models \
  alpine/curl curl -L -o /models/qwen2.5-7b-instruct-q4_k_m.gguf \
  "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf"

# Start everything (llama.cpp + searxng + researcher)
docker compose --profile local-llm up

# Research something
curl -X POST http://localhost:3000/research \
  -H 'Content-Type: application/json' \
  -d '{"query": "What are the latest advances in fusion energy?"}'
```

### OpenAI

```bash
cp .env.example .env
# Edit .env: set LLM_BASE_URL=https://api.openai.com/v1, LLM_MODEL=gpt-4o-mini, LLM_API_KEY=sk-...

docker compose up   # no --profile needed, llama-cpp is skipped

curl -X POST http://localhost:3000/research \
  -H 'Content-Type: application/json' \
  -d '{"query": "Impact of quantum computing on cryptography"}'
```

### CLI mode (no Docker)

```bash
cargo build --release

LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
./target/release/researcher --query "Rust async runtime internals"
```

## API

### `POST /research`
Blocking — waits for the full report.

```json
{ "query": "your research topic" }
```

Returns:
```json
{
  "topic": "...",
  "queries": ["sub-question 1", "..."],
  "source_count": 12,
  "report": "# Research Report\n\n..."
}
```

### `POST /research/stream`
SSE stream — progress events followed by the final report.

```bash
curl -X POST http://localhost:3000/research/stream \
  -H 'Content-Type: application/json' \
  -d '{"query": "history of the internet"}' \
  --no-buffer
```

Events:
```
data: {"type":"progress","message":"🔍 Planning research queries..."}
data: {"type":"progress","message":"📋 Generated 4 search queries","data":{"queries":[...]}}
data: {"type":"progress","message":"🌐 Crawling 4 queries in parallel..."}
...
event: complete
data: {"type":"complete","topic":"...","report":"# Research Report\n\n..."}
```

### `GET /health`
Returns `200 ok`.

## MCP Server

`researcher-mcp` is a separate binary that exposes the research pipeline as an
MCP tool over **stdio** (JSON-RPC). Use it with Claude Desktop, Claude Code,
or any MCP client. Returns the complete report — no streaming.

### Build

```bash
cargo build --release --bin researcher-mcp
# → target/release/researcher-mcp  (~6.6MB)
```

### Claude Desktop config (`~/.config/claude/claude_desktop_config.json`)

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
        "MAX_SEARCH_QUERIES": "4",
        "MAX_SOURCES_PER_QUERY": "4"
      }
    }
  }
}
```

### Claude Code config (`.mcp.json` or `~/.claude.json`)

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

### Tool: `research`

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | string | Research topic or question |
| `max_queries` | number? | Override sub-question count (default: 4) |
| `max_sources` | number? | Override sources per query (default: 4) |

Returns the full markdown research report as a string.

## LLM Backend Options

| Backend | Config | Notes |
|---------|--------|-------|
| **llama.cpp** (recommended local) | `LLM_BASE_URL=http://llama-cpp:8080/v1` | Lowest footprint, CUDA/ROCm/CPU, OpenAI-compatible |
| **Ollama** | `LLM_BASE_URL=http://host.docker.internal:11434/v1` | Easy model management |
| **vLLM** | `LLM_BASE_URL=http://vllm:8000/v1` | Best for multi-user / high concurrency |
| **OpenAI** | `LLM_BASE_URL=https://api.openai.com/v1` | Cloud, needs API key |
| **Anthropic** | Use a proxy like [LiteLLM](https://github.com/BerriAI/litellm) | OpenAI-compatible wrapper |

## Reusing the llama.cpp Container

The llama-cpp service is intentionally standalone. Other projects can use it:

```yaml
# In another project's docker-compose.yml:
services:
  my-app:
    environment:
      - LLM_BASE_URL=http://host.docker.internal:8080/v1
```

Or connect directly: `http://localhost:8080/v1/chat/completions`

## Tuning

| Env var | Default | Effect |
|---------|---------|--------|
| `MAX_SEARCH_QUERIES` | 4 | More = deeper research, slower |
| `MAX_SOURCES_PER_QUERY` | 4 | More sources per question |
| `LLAMA_GPU_LAYERS` | 99 | GPU layers (99=all, 0=CPU) |
| `LLAMA_PARALLEL` | 4 | Concurrent LLM request slots |
| `LLAMA_CTX_SIZE` | 8192 | Context window |
| `LLM_MAX_TOKENS` | 2048 | Max tokens per LLM response |
