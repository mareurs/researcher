# Gotchas & Known Issues

## Pipeline
- **Problem:** `run()` bails if `sources.is_empty()` or `summaries.is_empty()` — this happens silently if SearXNG is down and DDG fallback also fails.
  **Fix:** Check SearXNG health before running; error message includes the configured URL.

- **Problem:** `summarize_all` uses `join_all` — all LLM calls fire concurrently. With many sources this can overwhelm a local LLM server.
  **Fix:** Tune `MAX_SOURCES_PER_QUERY` and `MAX_SEARCH_QUERIES` to limit concurrency. Verify at `src/researcher/summarizer.rs:summarize_all`.

## MCP Binary
- **Problem:** All logging in `researcher-mcp` MUST go to stderr (stdout is the MCP protocol wire).
  **Fix:** Use `tracing` (which goes to stderr by default when configured). Never `println!` in MCP code.

## Build
- **Problem:** Release builds are slow due to `lto=true, codegen-units=1`.
  **Fix:** Use `cargo check` during development; only `cargo build --release` when you need the binary.

## Thinking Token Stripping
- **Problem:** `STRIP_THINKING_TOKENS` is `true` by default — if debugging Qwen3 reasoning, raw `<think>` blocks won't appear in output.
  **Fix:** Set `STRIP_THINKING_TOKENS=false`. Verify stripping logic in `src/llm/client.rs:strip_thinking` and `src/llm/stream.rs:strip_think`.

No additional gotchas discovered during onboarding. Update as issues are found.
