# Researcher

## Purpose
Fast AI research agent: takes a topic, spawns sub-questions, crawls+scrapes the web,
summarizes sources, and generates a Markdown report. Two binaries: HTTP server/CLI
(`researcher`) and MCP stdio server (`researcher-mcp`).

## Tech Stack
- **Language:** Rust 2021 edition
- **Async runtime:** Tokio (full features)
- **HTTP server:** Axum 0.7 (SSE streaming)
- **MCP:** `rmcp = "1.1"` with `#[tool_router]` / `#[tool]` macros
- **Key deps:** reqwest (HTTP client), scraper (HTML→text), clap (CLI+env), anyhow/thiserror (errors)

## Runtime Requirements
- SearXNG instance (default :4000) — required for search; falls back to DuckDuckGo Lite automatically
- Any OpenAI-compat LLM at `LLM_BASE_URL` (default :8080/v1) — required
- TEI embedding server at `EMBED_BASE_URL` — optional; enables dedup+rerank
- See CLAUDE.md § Env Vars Reference for full list with defaults
