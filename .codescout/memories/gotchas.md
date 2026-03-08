# Gotchas & Known Issues

## Configuration

- **MCP binary has separate config loading** — `config_from_env()` in `src/mcp_server.rs:307` is NOT the same as the clap `Config`. If you add a new config field to `Config`, you must also add it to `config_from_env()` manually.

- **`profiles.toml` must exist at the working directory** — Missing file silently produces empty profiles map and no job-profile. `load_profiles()` and `load_job_profile()` both swallow errors; no warning is emitted.

- **`EMBED_BASE_URL` empty = dedup disabled** — An empty string (the default) fully skips the embedding pipeline. Dedup and reranking only run when the env var is set to a real TEI endpoint.

## Pipeline Behavior

- **`crawl_all()` is NOT fully parallel** — Queries execute sequentially in a for loop (`src/researcher/crawler.rs:100`). The README and docs say "parallel per query" — this refers to URL fetches within a single query, not the queries themselves. This is intentional (shared `visited_urls`).

- **`Quick` mode returns `report: None`** — Callers must handle `ResearchResult.report` being `Option<String>`. The MCP `research` tool serializes the whole struct as JSON, so clients see `"report": null`.

- **`anyhow::bail!` on empty sources** — If SearXNG is unreachable AND DuckDuckGo fails, `run()` returns an error. The MCP tool converts this to an error string. Verify SearXNG URL before debugging LLM issues.

## Code Duplication

- **`strip_thinking` duplicated** — `strip_thinking()` in `src/llm/client.rs:144` and `strip_think()` in `src/llm/stream.rs:97` are near-identical. If you fix a bug in one, fix the other.

## Job Search

- **`search_jobs` requires `[job-profile]` in profiles.toml** — The tool returns a user-visible error string (not an Err) if the profile is missing. Verify `load_job_profile()` returns `Some` before debugging scorer failures.

- **Score threshold is hardcoded to 6** — `score_listings(&llm, &listings, &profile, 6)` in `src/mcp_server.rs:240`. Not configurable via env or MCP input currently.

## Build Warnings (expected, not real dead code)

- **`src/jobs/` functions show dead-code warnings when building the `researcher` binary** — `fetch_jobs`, `fetch_remotive`, `fetch_adzuna`, `fetch_searxng`, `score_listings`, `write_job_report` are only called from `researcher-mcp`. Because each binary is compiled independently, the `researcher` binary sees them as unused. These warnings are false positives — the code is live in the MCP binary. Do not delete these functions based on the warnings.

- **`ResearchTarget::Person` and `::Company` variants flagged unused in `researcher` binary** — Same reason: they're only constructed in `src/mcp_server.rs`. Safe to ignore.

## No Tests

- **Zero test suite** — There are no test files. All verification is manual. Be careful when refactoring shared modules (config, llm, search) — breakage won't be caught automatically.
