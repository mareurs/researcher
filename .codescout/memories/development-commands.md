# Development Commands

See CLAUDE.md for the canonical build/run/Docker commands. This supplements with workflow details.

## Before Completing Work
1. `cargo check` — fast type check (seconds)
2. `cargo clippy -- -D warnings` — must pass clean
3. `cargo build --release` — full build with LTO (~30-60s); required before testing MCP tools
4. If adding a Config field: verify BOTH `Config` (src/config.rs) AND `config_from_env()` (src/mcp_server.rs) are updated
5. If adding an MCP tool: update `get_info()` in `src/mcp_server.rs` with a bullet for the new tool

## Testing MCP After Changes
The `researcher-mcp` binary is loaded once at MCP server start.
After `cargo build --release`: restart MCP server (`/mcp` → restart in Claude Code, or restart the session).
Do NOT test via `mcp__researcher__*` tools without rebuilding first.

## Infra Stack
Two separate docker-compose stacks (NOT the single-compose described in CLAUDE.md):
```bash
# Start shared AI infra (SearXNG, llama-cpp heavy+fast, TEI embed+rerank)
cd infra && docker compose up -d

# Start researcher app (joins ai-infra-net)
docker compose up -d   # from repo root
```
The Makefile at root wraps some of these targets.

## Environment Setup
```bash
cp .env.example .env   # then edit LLM_BASE_URL, SEARXNG_URL, etc.
```
Key non-obvious env vars not shown in CLAUDE.md:
- `RERANK_MIN_SCORE` — cross-encoder logit threshold (default -5.0); increase to be more selective
- `LLM_FAST_STAGES` — comma-separated; default `planner,summarizer`; add `publisher` to route all to fast
