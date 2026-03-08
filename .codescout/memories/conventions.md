# Conventions

## Naming
| Entity | Convention | Example |
|--------|-----------|---------|
| Async functions | `async fn` everywhere in pipeline | `run()`, `crawl_all()`, `summarize_all()` |
| Progress callbacks | `impl Fn(ProgressEvent)` passed into `run()` | `\|ev\| eprintln!("[researcher] {ev}")` |
| MCP tool methods | `async fn` on `ResearcherServer`, `#[tool(...)]` attribute | `research_person()` |
| Config fields | snake_case matching env var name lowercased | `llm_base_url` ↔ `LLM_BASE_URL` |
| Error returns | `anyhow::Result<T>` everywhere; `format!("Error: {e:#}")` at MCP boundary | |

## Patterns

### Error handling
- Internal: `?` propagation with `anyhow::Result`
- At MCP boundary: `match run(...) { Ok(r) => ..., Err(e) => format!("Error: {e:#}") }`
- Search failures: logged via `warn!(%e, ...)` and swallowed (returns empty vec)
- Scrape failures: per-URL `warn!` + `filter_map` drops failed entries

### LLM calls
- Non-streaming: `llm.complete(messages).await` — used in planner, summarizer, MCP, HTTP-JSON
- Streaming: `llm.stream(messages, tx).await` — used only in CLI and HTTP SSE
- Both paths apply `strip_thinking` if configured (Qwen3 `<think>` blocks)

### Config loading (two paths)
- `researcher` binary: `clap` derive on `Config` struct — CLI flags + env vars merged by clap
- `researcher-mcp` binary: `config_from_env()` local function reads env directly

### Async concurrency
- Within-query URL fetches: `join_all(futs)` in `crawl_query()`
- Summarization: `join_all(futs)` in `summarize_all()` — ALL sources concurrent
- Cross-query crawl: sequential loop in `crawl_all()` to share `visited_urls`

## Code Quality
- No formatter config found — use standard `rustfmt`
- `cargo check` for fast type-check; `cargo build --release` for full build
- `RUST_LOG=info` / `RUST_LOG=debug` controls tracing output
- **No test suite exists** — verify by running the binaries manually

## Testing
No test framework configured. No test files. Manual verification only.
See CLAUDE.md for run commands.
