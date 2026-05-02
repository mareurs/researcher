# Researcher Progressive Disclosure

**Status:** draft  
**Date:** 2026-05-02

## Problem

Researcher MCP tools return `serde_json::to_string_pretty(&result)` inline — the full markdown
report lands directly in the agent's context window. For `report` and `deep` modes this is
routinely 3000–10000+ tokens, bloating context and degrading downstream reasoning quality.
Research (see code-explorer's `docs/research-progressive-disclosure.md`) shows success rates
drop from 40–50% to under 10% in long-context conditions even with perfect retrieval.

## Design

### Threshold gate

After `run()` returns in `mcp_server.rs::research()`, check `report.len() > DISCLOSURE_THRESHOLD`
(4000 chars, ~1000 tokens). If exceeded → disclosure path. Otherwise → inline as today.

`quick` and `summary` mode results are almost always under threshold and return inline unchanged.
`report` and `deep` almost always exceed it. The gate is content-driven, not mode-driven, so
edge cases (a tiny deep report, a large summary) are handled correctly. Response shape is
consistent within each path — callers never need to branch on mode.

### Planner-derived `DisclosureHint`

The planner (`src/researcher/planner.rs`) already makes an LLM call to generate query strategy.
Its output schema is extended to also emit:

```rust
pub struct DisclosureHint {
    pub summary_focus: String,
    pub include_sources: bool,
}
```

Classification is baked into the planner's existing LLM prompt — no extra call, no extra latency.

| Query type | `summary_focus` | `include_sources` |
|---|---|---|
| Fact lookup ("what is X", "when did Y") | `"key facts and direct answer"` | false |
| How-to / implementation | `"what solves it and how to implement"` | true |
| News / events | `"what happened and outcome"` | true |
| General research | `"main findings and key points"` | true |

`ResearchInput` gains an optional `disclosure_hint` field. When present it overrides the planner
default. When absent the planner default is used. Simple callers need not set it.

### `disclosure.rs` — new module

`src/researcher/disclosure.rs` is called from `mcp_server.rs` after `run()` when the threshold
is exceeded.

```rust
pub struct TocEntry {
    pub heading: String,
    pub line: usize,
}

pub struct DisclosedResult {
    pub summary: String,
    pub toc: Vec<TocEntry>,
    pub source_count: usize,
    pub queries_used: Vec<String>,
    pub report_path: String,
    pub sources: Option<Vec<SourceEntry>>,  // None when include_sources: false
}

pub async fn disclose(
    report: &str,
    hint: &DisclosureHint,
    llm: &LlmClient,
    slug: &str,
) -> Result<DisclosedResult>
```

Three steps:

**1. Save** — write report to `/tmp/researcher/<slug>-<unix_timestamp>.md`.  
`slug` = first 40 chars of query, lowercased, non-alphanumeric replaced with `-`.  
Creates `/tmp/researcher/` if absent.

**2. Extract TOC** — scan markdown lines for `^#{1,3} `, collect `(heading_text, line_number)`.
Zero LLM cost. Level-4+ headings omitted (too granular for navigation hints).

**3. Summarize** — one focused LLM call:

> *"In 3–5 sentences, summarize this research focused on: {summary_focus}.
> Do not repeat section headings. Be direct. Omit caveats unless critical."*

### Response shape

When disclosure triggers, `research()` serializes `DisclosedResult` instead of `ResearchResult`:

```json
{
  "summary": "Axum 0.8 breaks handler signatures — ...",
  "toc": [
    { "heading": "## Breaking Changes", "line": 12 },
    { "heading": "## Migration Guide", "line": 28 },
    { "heading": "## Compatibility Matrix", "line": 47 }
  ],
  "source_count": 8,
  "queries_used": ["axum 0.8 changelog", "axum 0.8 migration guide"],
  "report_path": "/tmp/researcher/axum-0-8-changelog-1746123456.md"
}
```

`sources` is present only when `include_sources: true`. For fact queries the field is omitted
entirely — agent doesn't need 12 URLs to answer "what year was X released".

### `server_instructions` navigation block

Added to `get_info()`. All agents (main and subagents) see it once via the MCP initialize
response.

```
## Research Results

When a result contains `report_path`, the full report is saved on disk.
Read `summary` first — if it answers your question, skip the file entirely.
Only drill down when you need details the summary omits.

Navigation:
- Full report:      read_file("<report_path>")
- Targeted section: run_command("sed -n '<line>,<end_line>p' <report_path>")
- Search within:    run_command("grep -n '<keyword>' <report_path>")
- Sources:          listed inline in `sources[]` when present

For fact queries and quick lookups where summary suffices — do not open the file.
```

## Files changed

| File | Action |
|---|---|
| `src/researcher/disclosure.rs` | Create |
| `src/researcher/mod.rs` | Add `pub mod disclosure` |
| `src/researcher/planner.rs` | Extend output schema with `DisclosureHint` |
| `src/mcp_server.rs` | Add `disclosure_hint` field to `ResearchInput`; add threshold gate + `disclose()` call in `research()`; update `server_instructions` in `get_info()` |

All other tools (`research_person`, `research_company`, etc.) are unaffected. Their reports are
typically smaller and structurally different. Progressive disclosure can be extended to them
in a follow-up if needed.

## Constants

```rust
const DISCLOSURE_THRESHOLD: usize = 4_000; // chars (~1000 tokens)
```

Tunable via env var `RESEARCHER_DISCLOSURE_THRESHOLD` if needed.

## Non-goals

- No new MCP tools. Agents navigate with `read_file` / `run_command` on the file path.
- No disk cleanup / TTL. `/tmp` is ephemeral; OS handles it.
- No streaming of the disclosure response.
- No changes to `research_person`, `research_company`, `search_jobs`, `research_code`, `market_insight`.
