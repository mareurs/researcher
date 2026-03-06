# Code Research Tool — Design

## Overview

Add a new MCP tool `research_code` for developer-focused research: known bugs, changelogs, breaking changes, releases, and community sentiment for a given framework or library. Targets Claude Code as the primary caller.

## Input Shape

```rust
CodeResearchInput {
    framework: String,         // "axum", "claude code", "tokio"
    version: Option<String>,   // "0.8"; defaults to "latest" if omitted
    aspects: Vec<String>,      // ["bugs", "changelog", "community", "releases"]
                               // defaults to all three if empty
    repo: Option<String>,      // "tokio-rs/tokio" — anchors GitHub queries
}
```

## Query Templates (no LLM planner call)

Each aspect maps to 1-2 deterministic queries substituting `{framework}`, `{version}`, and `{repo}`:

| Aspect | Queries |
|--------|---------|
| `bugs` | `{framework} {version} bug issue` + `site:github.com/{repo}/issues` (if repo given) |
| `changelog` | `{framework} {version} changelog release notes` + `{framework} {version} breaking changes` |
| `community` | `{framework} {version} site:reddit.com` + `{framework} {version} site:news.ycombinator.com` |
| `releases` | `{framework} {version} release` + `site:github.com/{repo}/releases` (if repo given) |

Queries are built directly in the tool handler — no `generate_queries()` LLM call.

## Pipeline Flow

```
research_code input
  → build query list from templates          (no LLM call)
  → crawl_all(&http, &cfg, &queries)         (existing, unchanged)
  → summarize_all(&llm, &sources, topic)     (existing, unchanged)
  → write_code_report(llm, summaries,        (new fn in publisher.rs)
                      framework, version,
                      aspects)
  → return markdown string
```

## Publisher

New `write_code_report()` function in `src/researcher/publisher.rs` alongside the existing `write_report()`. Uses a specialized prompt that:
- Produces one `##` section per requested aspect (e.g. `## Known Bugs & Issues`, `## Changelog & Breaking Changes`, `## Community Sentiment`)
- Instructs the LLM to cite sources inline with `[N]` notation
- Uses `llm.complete()` — blocking, no streaming (consistent with all MCP tools)

## MCP Registration

New `research_code` method on `ResearcherServer` in `src/mcp_server.rs`, following the same pattern as `research_person` and `research_company`.

## Error Handling

- **No aspects provided** → default to `["bugs", "changelog", "community"]`
- **Unknown aspect string** → silently skip (don't fail the whole call)
- **No repo provided** → drop `site:github.com/{repo}` filter, use open search
- **version omitted** → substitute `"latest"` in all queries
- **No sources scraped** → return `"Error: no sources found for {framework} {version}"`

## No New Config Required

Reuses existing `max_sources_per_query`, `searxng_url`, and all LLM settings.
