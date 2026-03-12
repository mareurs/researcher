# Researcher

## Purpose
Fast AI research agent that generates sub-questions via LLM, crawls + scrapes the web, deduplicates/reranks results, summarizes sources concurrently, and produces a structured report. Targets meeting prep, market intelligence, and job search. Ships as two binaries: HTTP/CLI (`researcher`) and MCP stdio server (`researcher-mcp`).

## Tech Stack
- **Language:** Rust (tokio async runtime)
- **HTTP:** axum 0.7 (server), reqwest 0.12 (client)
- **HTML scraping:** scraper 0.21 (CSS selectors)
- **MCP framework:** rmcp 1.1 (tool_router macros, stdio transport)
- **CLI:** clap 4 (derive + env)
- **Error handling:** anyhow (app), thiserror (library types)
- **Streaming:** eventsource-stream (SSE token streaming)
- **Python skeleton:** src/model/, src/training/ — future cross-encoder fine-tuning (not yet implemented)

## Runtime Requirements
- OpenAI-compatible LLM endpoint (`LLM_BASE_URL`, any llama.cpp or OpenAI API)
- SearXNG instance (`SEARXNG_URL`) — DuckDuckGo Lite is automatic fallback (no config)
- Optional: TEI embedding server (`EMBED_BASE_URL`) for dedup, TEI cross-encoder (`RERANK_BASE_URL`) for reranking
- Optional: Adzuna API keys for job search, social platform cookies for scraping auth
- `profiles.toml` at runtime — domain profiles + job profile (see CLAUDE.md for format)

## Two Docker Stacks
Split architecture (not the single-compose described in CLAUDE.md):
- `infra/docker-compose.yml` — always-on shared AI infra: SearXNG, llama-cpp (NVIDIA A5000/CUDA), llama-cpp-fast (AMD RX7800XT/ROCm), tei-embed, tei-rerank. Uses external network `ai-infra-net`.
- `docker-compose.yml` (root) — researcher app only, joins `ai-infra-net`.
