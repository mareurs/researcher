# Conventions

See CLAUDE.md § Rust Coding Standards for the full style guide. This file captures patterns discovered during exploration that aren't in CLAUDE.md.

## Naming

| Entity | Convention | Example |
|---|---|---|
| Pipeline stage functions | verb_noun free fn | `generate_queries()`, `crawl_all()`, `summarize_all()`, `write_report()` |
| LLM client constructors | `new()` (heavy) / `new_fast()` (fast) | `LlmClient::new(cfg)`, `LlmClient::new_fast(cfg)` |
| MCP input structs | `<Tool>Input` | `ResearchInput`, `JobSearchInput`, `MarketInsightInput` |
| Progress events | PascalCase enum variants, no payload for state signals | `ProgressEvent::Planning`, `ProgressEvent::Queries(vec)` |
| Config env var | SCREAMING_SNAKE matching clap long name | `--llm-base-url` → `LLM_BASE_URL` |

## Patterns

### Error Handling
- All public functions return `Result<T, anyhow::Error>` via `?`
- Scrape failures in `crawl_query()`: fall back to search snippet, do NOT propagate
- Summarize failures in `summarize_all()`: silently drop with `warn!()`, not panic
- Rerank failure: fall back to dedup/original order
- MCP tools: catch all errors and return `format!("Error: {e:#}")` as String

### LLM Prompt Conventions
- Prepend `/no_think` to system prompts for planner and summarizer (Qwen3 convention)
- JSON output requested via explicit schema in system prompt
- JSON parse failures: fall back to treating full response as plain text

### Thinking Token Suppression (three mechanisms — keep in sync)
1. `/no_think` prefix in system prompt (planner, summarizer) — Qwen3 chat template
2. `disable_thinking: true` in LlmClient → sends `chat_template_kwargs: {"enable_thinking": false}` — llama.cpp extension, fast client always sets this
3. `STRIP_THINKING_TOKENS=true` → post-hoc strips `<think>...</think>` — applies in both client.rs complete() AND stream.rs

### Fast LLM Stage Routing
```rust
// run() stage assignment pattern:
let effective_fast = request.fast_stages.as_deref().unwrap_or(&cfg.llm_fast_stages);
let planner_llm = if effective_fast.contains("planner") { &llm_fast } else { &llm };
```

### Cloning LlmClient for Concurrency
`LlmClient` clones are cheap — the inner `reqwest::Client` is Arc-backed. Clone per async future in `join_all`:
```rust
let futs = sources.iter().map(|s| summarize_source(llm.clone(), s, topic));
join_all(futs).await
```

## Testing
No Rust tests. Verify changes with `cargo check` (fast) then `cargo build --release` (full LTO, ~30-60s).
Python tests exist in `tests/model/` for a planned cross-encoder module that isn't implemented yet (untracked in git).

## Code Quality
```bash
cargo check            # fast type check
cargo clippy -- -D warnings   # must pass clean
cargo build --release  # final verification before shipping
```
