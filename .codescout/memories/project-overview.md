# Researcher

## Purpose
Fast async AI research agent in Rust. Given a topic/person/company, it plans sub-questions,
searches the web, scrapes and summarizes sources concurrently, and produces a markdown report.
Also includes a job-search pipeline against structured job board APIs.

## Tech Stack
- **Language:** Rust (edition 2021)
- **Async runtime:** Tokio (full features)
- **HTTP server:** Axum 0.7 with SSE streaming
- **MCP server:** rmcp 1.1 (stdio transport, `#[tool_router]` macros)
- **Key deps:** reqwest 0.12, scraper 0.21, serde_json, clap 4 (derive+env), anyhow/thiserror

## Two Binaries
- `researcher` (src/main.rs) — HTTP server + CLI; config via clap derive + env vars
- `researcher-mcp` (src/mcp_server.rs) — MCP stdio server; config via `config_from_env()` helper
  **Important:** MCP binary has its own `config_from_env()` — NOT the same code path as clap Config

## Runtime Requirements
- SearXNG instance (default :4000) — DuckDuckGo Lite is the automatic fallback
- Any OpenAI-compatible LLM endpoint (default :8080/v1 for llama.cpp)
- Optional: TEI embedding server for dedup/rerank (EMBED_BASE_URL)
- Optional: Adzuna API keys for job search (ADZUNA_APP_ID, ADZUNA_APP_KEY)
- profiles.toml must exist at CWD for domain profiles and job-profile to load

## MCP Tools Exposed
`research`, `research_person`, `research_company`, `search_jobs`
