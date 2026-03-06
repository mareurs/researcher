# Conventions

## Error Handling
- All fallible functions return `anyhow::Result<T>`
- Structs with their own error types use `thiserror`
- Pipeline bails with descriptive messages: `anyhow::bail!("No sources scraped. Check SearXNG...")`

## Naming
- Free functions use snake_case, named after their action: `crawl_all`, `summarize_all`, `generate_queries`
- Structs are PascalCase; request/response pairs: `ChatRequest`/`ChatResponse`, `EmbedRequest`/`EmbedResponse`
- Internal HTTP types (serde structs) live alongside their client in the same file

## Async
- All pipeline stages are `async`; use `.await` throughout
- `join_all` for parallelism (see `summarize_all`)
- `crawl_all` uses sequential per-query crawl but parallel scraping within each query

## Module Structure
- Each module (`llm/`, `search/`, `embeddings/`, `researcher/`) has a `mod.rs` re-exporting submodules
- Public API of each module is the functions in `mod.rs` or named submodules
- No traits for dependency injection — concrete types passed as `&LlmClient`, `&EmbedClient`

## Code Style
- Codescout rules enforced by hooks: never `Read` source files, never `edit_file` for structural changes (see CLAUDE.md § Codescout Rules)
- `cargo check` for fast validation; `cargo build --release` for full build
