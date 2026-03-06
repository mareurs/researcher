# Design: Rich Research Request — Modes, Domains, Profiles

**Date:** 2026-03-06  
**Status:** Approved

## Context

The researcher tool is used ~90% via MCP from Claude Code, ~10% via HTTP/SSE. The current
pipeline has a single code path: web search → summarize → full markdown report. There is no
way to request a quick lookup vs. a deep report, or to scope search to specific sites.

Context token efficiency is a hard constraint: the MCP tool description must be slim and
expressive, and the output shape must let Claude choose what to use.

## Goals

1. Multiple research depths/output modes selectable per call
2. Domain filtering via named profiles (config file) or raw domains per call
3. Always-structured output: `{ report, sources }` so Claude can use either
4. Slim MCP tool description — one short paragraph, no bloat
5. Clean architecture: per-request params separate from startup config

## Non-Goals

- Local file / personal knowledge base ingestion (not needed)
- Conversational follow-up baked into the tool (Claude Code handles this)
- Plugin trait system for pipeline stages (YAGNI)

---

## Architecture

### `ResearchRequest` struct (new)

A new struct that carries all per-call parameters, separate from `Config` (which stays as
startup/env config). Every entry point (MCP tool, HTTP handler, CLI) populates a
`ResearchRequest` and passes it into `run()`.

```rust
pub struct ResearchRequest {
    pub topic: String,
    pub mode: ResearchMode,
    pub domains: Vec<String>,        // raw domain override
    pub domain_profile: Option<String>, // named profile key
}

pub enum ResearchMode {
    Quick,    // search only, no LLM
    Summary,  // bullets ~300 tok
    Report,   // full markdown (current default)
    Deep,     // 2× queries, higher source cap, detailed prompt
}
```

`run()` signature changes from `(cfg, topic, on_progress, token_tx)` to
`(cfg, request, on_progress, token_tx)`.

### Domain Profiles (`profiles.toml`)

A `profiles.toml` file (next to the binary, or at `$CONFIG_DIR/researcher/profiles.toml`)
defines named domain presets. Loaded once at startup into `Config`. At request time,
`domain_profile` is resolved to a `Vec<String>` and merged with any raw `domains`.

```toml
[shopping-ro]
domains = ["olx.ro", "publi24.ro", "okazii.ro", "emag.ro"]

[tech-news]
domains = ["news.ycombinator.com", "lobste.rs", "reddit.com/r/programming"]

[llm-news]
domains = ["huggingface.co", "arxiv.org", "reddit.com/r/LocalLLaMA", "reddit.com/r/LocalLLaMA"]

[academic]
domains = ["arxiv.org", "scholar.google.com", "semanticscholar.org", "pubmed.ncbi.nlm.nih.gov"]
```

### Domain Anchoring in Planner

When domains are present, `generate_queries()` appends a site-filter clause to the
system/user prompt instructing the LLM to generate queries that include
`site:domain1 OR site:domain2` modifiers. This scopes SearXNG results without changing
the search engine integration.

### Mode Behavior

| Mode | Planner | Crawler | Summarizer | Publisher |
|------|---------|---------|-----------|-----------|
| `quick` | runs normally | runs normally | **skipped** | **skipped** |
| `summary` | runs normally | runs normally | runs | bullet-point prompt (~300 tok) |
| `report` | runs normally | runs normally | runs | full report prompt (current) |
| `deep` | 2× `max_queries`, 2× `max_sources_per_query` | runs | runs | detailed long-form prompt |

In `quick` mode the pipeline short-circuits after crawling and returns sources only.

### Output Shape

All modes return `ResearchResult` (already exists), extended:

```rust
pub struct ResearchResult {
    pub report: Option<String>,   // None in quick mode
    pub sources: Vec<SourceEntry>,
}

pub struct SourceEntry {
    pub url: String,
    pub title: String,
    pub snippet: String,   // first ~200 chars of scraped content
}
```

MCP serializes this as JSON. Claude can read `sources` for quick scans or `report` for
synthesis.

### MCP Tool Description (slim)

```
research(topic, mode?, domain_profile?, domains?)

Modes: quick=links+snippets, summary=bullet facts, report=full markdown (default), deep=thorough.
Profiles: shopping-ro, tech-news, llm-news, academic. Or pass domains:["site1","site2"] directly.
```

Two lines. Claude infers the right mode and profile from user intent.

### HTTP API

`POST /research` and `POST /research/stream` accept the same extended JSON body:

```json
{
  "topic": "best standing desks under 500 EUR",
  "mode": "summary",
  "domain_profile": "shopping-ro",
  "domains": []
}
```

### CLI

New flags: `--mode quick|summary|report|deep`, `--domain-profile <name>`, `--domains <d1,d2>`.

---

## File Changes

| File | Change |
|------|--------|
| `src/researcher/pipeline.rs` | `ResearchRequest` struct + `ResearchMode` enum; `run()` takes request |
| `src/researcher/planner.rs` | `generate_queries()` accepts domain list, injects site filters |
| `src/researcher/crawler.rs` | no change |
| `src/researcher/summarizer.rs` | no change |
| `src/researcher/publisher.rs` | mode-aware prompt selection |
| `src/config.rs` | add `profiles: HashMap<String, Vec<String>>` loaded from profiles.toml |
| `src/mcp_server.rs` | tool params: mode, domain_profile, domains |
| `src/server.rs` | HTTP body extended with new fields |
| `src/main.rs` | CLI flags for mode + domains |
| `profiles.toml` (new) | default domain profiles |

---

## Open Questions (resolved)

- **Profiles location:** `profiles.toml` next to binary / in project root. Falls back to
  empty map if missing (graceful degradation).
- **Domain merge:** profile domains + raw domains are unioned. Duplicates ignored.
- **Deep mode token budget:** respects `LLM_MAX_TOKENS` env var; prompt instructs "be thorough".
- **quick mode sources snippet:** use first `min(content.len(), 200)` chars of scraped content.
