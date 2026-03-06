# Researcher — Code Explorer Guidance

## Entry Points
- `src/researcher/pipeline.rs` → `run()` — the core orchestrator; start here for any pipeline task
- `src/main.rs` → `run_cli()` / `run_server()` — binary entry points
- `src/mcp_server.rs` → `ResearcherServer` — MCP entry point

## Key Abstractions
- `Config` (`src/config.rs`) — all settings, passed everywhere as `&Config` or `Arc<Config>`
- `LlmClient` (`src/llm/client.rs`) — OpenAI-compat wrapper; `complete()` blocking, `stream_completion()` in `src/llm/stream.rs`
- `ScrapedSource` (`src/researcher/crawler.rs`) — central data struct: url+title+content, flows crawler→embeddings→summarizer
- `ProgressEvent` (`src/researcher/pipeline.rs`) — pipeline lifecycle enum with Display impl
- `EmbedClient` (`src/embeddings/client.rs`) — optional TEI client for dedup+rerank

## Search Tips
- Good queries: "pipeline stages", "streaming token", "search fallback", "progress event", "summarize concurrent"
- Avoid broad terms: "client", "config", "result" — too many hits
- `find_symbol("ScrapedSource")` is your anchor when tracing data flow through the pipeline

## Navigation Strategy
1. `memory(action="read", topic="architecture")` — data flow with actual function names
2. `list_symbols("src/researcher/")` — see all pipeline stages
3. `find_symbol("run", path="src/researcher/pipeline.rs", include_body=true)` — read the orchestrator
4. Follow `ScrapedSource` refs to understand a stage end-to-end

## Project Rules
- Two binaries: `researcher` (src/main.rs) and `researcher-mcp` (src/mcp_server.rs) — never mix stdout/stderr in MCP binary
- `token_tx: Option<mpsc::Sender<String>>` = None means blocking LLM, Some means streaming — always thread this through
- No test suite; validate with `cargo check` then manual run
- Codescout rules enforced by hooks: use symbol tools for source code, never `read_file` on .rs files
