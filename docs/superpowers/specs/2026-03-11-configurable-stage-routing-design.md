# Configurable Stage-Level Model Routing

**Date:** 2026-03-11
**Status:** Approved

## Overview

Make LLM model routing (fast AMD vs heavy NVIDIA) configurable per pipeline stage, with config defaults and per-request overrides. This lets the caller decide which stages run on the fast model for testing different quality/speed tradeoffs.

## Pipeline Stages

Three stages use LLM:

| Stage | Function | Current routing |
|-------|----------|----------------|
| `planner` | `generate_queries()` | fast |
| `summarizer` | `summarize_all()` (includes judge) | fast |
| `publisher` | `write_report()` | heavy |

## Config Default

New field in `Config` (`src/config.rs`) and `config_from_env()` (`src/mcp_server.rs`):

| Field | Env var | Default |
|-------|---------|---------|
| `llm_fast_stages` | `LLM_FAST_STAGES` | `planner,summarizer` |

Parsed as comma-separated list of stage names, trimmed and lowercased at parse time. Matches current hardcoded behavior out of the box.

## Per-Request Parameter

New optional field on MCP tool inputs, HTTP API (`ResearchBody`), and `ResearchRequest`:

```
fast_stages: Option<Vec<String>>
```

Valid stage names: `planner`, `summarizer`, `publisher`. Unrecognized names emit a `warn!` log and are ignored.

When provided, replaces the config default for that request. When omitted, uses config default.

Input values are trimmed and lowercased at parse boundaries (MCP input handler, `ResearchBody` deserialization) to prevent case mismatches.

### Affected MCP tools

- `research` — add `fast_stages` param
- `research_person` — add `fast_stages` param
- `research_company` — add `fast_stages` param
- `research_code` — add `fast_stages` param
- `market_insight` — add `fast_stages` param

### HTTP API

`ResearchBody` in `src/server.rs` gets `fast_stages: Option<Vec<String>>`, wired through `into_pipeline_request()` to `ResearchRequest`.

### CLI

The CLI binary does not expose `fast_stages` as a flag — uses config default only.

### Excluded: `search_jobs`

`search_jobs` has its own pipeline (`jobs::scorer`, `jobs::publisher`) and does not go through `run()`. Not affected by this change.

## Pipeline Routing Logic

In `run()` (`src/researcher/pipeline.rs`), replace hardcoded client assignments:

```rust
let effective_fast: Vec<String> = request.fast_stages
    .clone()
    .unwrap_or_else(|| cfg.llm_fast_stages.clone());

// Warn on unrecognized stage names
for s in &effective_fast {
    if !["planner", "summarizer", "publisher"].contains(&s.as_str()) {
        warn!(stage = %s, "unknown stage in fast_stages, ignored");
    }
}

let planner_llm = if effective_fast.iter().any(|s| s == "planner") { &llm_fast } else { &llm };
let summarizer_llm = if effective_fast.iter().any(|s| s == "summarizer") { &llm_fast } else { &llm };
let publisher_llm = if effective_fast.iter().any(|s| s == "publisher") { &llm_fast } else { &llm };
```

Then pass each to the respective function call.

`ResearchRequest::new()` constructor gets `fast_stages: None` as default — callers that don't care about routing get config defaults automatically.

## Logging

Log effective routing at pipeline start:

```
INFO pipeline: stage routing planner=fast summarizer=fast publisher=heavy
```

## Fallback Behavior

When `LLM_FAST_BASE_URL` is empty, `LlmClient::new_fast()` already falls back to the heavy backend. Setting `fast_stages: ["planner","summarizer","publisher"]` on a single-GPU setup is harmless — all three stages use the same backend.

## What Doesn't Change

- `LlmClient::new()` / `new_fast()` constructors
- `disable_thinking` on fast client
- `stream()` method — works regardless of which client calls it
- Quick mode — still short-circuits before summarizer/publisher
- No new dependencies
