# Researcher — Code Explorer Guidance

## Entry Points
- `src/researcher/pipeline.rs:119` — `run()` — the central orchestrator for all research
- `src/mcp_server.rs:101` — `ResearcherServer` — MCP tool definitions (research, research_person, research_company, search_jobs)
- `src/main.rs:19` — `main()` — CLI/server binary entry; clap Config
- `src/jobs/fetcher.rs:132` — `fetch_jobs()` — job search entry point (separate from pipeline)

## Key Abstractions
- `run()` — `src/researcher/pipeline.rs` — 10-stage pipeline; understand this first
- `ResearchRequest` / `ResearchMode` / `ResearchTarget` — `src/researcher/pipeline.rs` — control all pipeline behavior
- `LlmClient` — `src/llm/client.rs` — `.complete()` vs `.stream()` is the only streaming switch
- `Config` — `src/config.rs` — all tunables; note MCP has a separate `config_from_env()` copy

## Search Tips
Good queries: "pipeline stages", "progress events", "domain profile resolution", "token streaming", "job scoring", "cookie auth scraping"
Avoid: "data", "result", "error" (too broad)
`ResearchTarget` and `ResearchMode` are the key enums that fan out behavior — search those first when tracing a research path.

## Navigation Strategy
1. `memory(action="read", topic="architecture")` — orient with module map and data flow
2. `find_symbol("run", path="src/researcher/pipeline.rs", include_body=true)` — read the orchestrator
3. `list_symbols("src/researcher/")` — survey the pipeline modules
4. `semantic_search("your concept")` — find connecting code
5. For job search: start at `src/jobs/` — completely separate from the research pipeline

## Project Rules
- After adding a Config field: update BOTH `Config` (src/config.rs) AND `config_from_env()` (src/mcp_server.rs:307)
- `crawl_all()` is sequential by design — don't parallelize the query loop
- `strip_thinking` logic exists in TWO places — client.rs and stream.rs — keep them in sync
- No test suite — run `cargo check` then `cargo build --release` to verify changes
