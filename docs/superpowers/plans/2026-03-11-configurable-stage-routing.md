# Configurable Stage-Level Model Routing — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make LLM model routing (fast vs heavy) configurable per pipeline stage via config defaults and per-request overrides.

**Architecture:** Add `llm_fast_stages` to `Config` (comma-separated env var, default `planner,summarizer`). Add `fast_stages: Option<Vec<String>>` to `ResearchRequest` and all MCP/HTTP input structs. Pipeline resolves effective stages and picks the right `LlmClient` per stage.

**Tech Stack:** Rust, serde, clap, rmcp

**Spec:** `docs/superpowers/specs/2026-03-11-configurable-stage-routing-design.md`

---

## File Map

| File | Change | Responsibility |
|------|--------|----------------|
| `src/config.rs` | Modify | Add `llm_fast_stages: Vec<String>` field |
| `src/researcher/pipeline.rs` | Modify | Add `fast_stages` to `ResearchRequest`, routing logic in `run()` |
| `src/mcp_server.rs` | Modify | Add `fast_stages` to all 5 input structs + `config_from_env()` |
| `src/server.rs` | Modify | Add `fast_stages` to `ResearchBody` + `into_pipeline_request()` |
| `CLAUDE.md` | Modify | Add `LLM_FAST_STAGES` to env vars table |
| `.env.example` | Modify | Add `LLM_FAST_STAGES` |
| `docker-compose.yml` | Modify | Add `LLM_FAST_STAGES` env var to researcher service |

---

## Chunk 1: Core — Config + Pipeline

### Task 1: Add `llm_fast_stages` to Config

**Files:**
- Modify: `src/config.rs` — add field after `llm_fast_max_tokens` (line ~75)
- Modify: `src/mcp_server.rs` — add field in `config_from_env()` (line ~520)

- [ ] **Step 1: Add field to Config struct**

In `src/config.rs`, add after the `llm_fast_max_tokens` field:

```rust
/// Pipeline stages that use the fast LLM backend (comma-separated).
/// Valid: planner, summarizer, publisher. Default: planner,summarizer.
#[arg(long, env = "LLM_FAST_STAGES", value_delimiter = ',', default_values_t = vec!["planner".to_string(), "summarizer".to_string()])]
pub llm_fast_stages: Vec<String>,
```

- [ ] **Step 2: Add field to `config_from_env()` in mcp_server.rs**

In `config_from_env()`, add after `llm_fast_max_tokens`:

```rust
llm_fast_stages: env("LLM_FAST_STAGES", "planner,summarizer")
    .split(',')
    .map(|s| s.trim().to_lowercase())
    .filter(|s| !s.is_empty())
    .collect(),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: success

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/mcp_server.rs
git commit -m "feat: add llm_fast_stages config field"
```

### Task 2: Add `fast_stages` to ResearchRequest and wire routing in `run()`

**Files:**
- Modify: `src/researcher/pipeline.rs` — add field to `ResearchRequest`, update `run()`

- [ ] **Step 1: Add field to ResearchRequest**

Add to `ResearchRequest` struct (after `target` field):

```rust
/// Override which pipeline stages use the fast LLM backend.
/// When None, uses config default (cfg.llm_fast_stages).
pub fast_stages: Option<Vec<String>>,
```

- [ ] **Step 2: Update `run()` routing logic**

In `run()`, replace the hardcoded client assignments. After `let llm_fast = LlmClient::new_fast(cfg);`, add:

```rust
// Resolve effective fast stages (request override > config default)
let effective_fast: Vec<String> = request.fast_stages
    .clone()
    .unwrap_or_else(|| cfg.llm_fast_stages.clone());

for s in &effective_fast {
    if !["planner", "summarizer", "publisher"].contains(&s.as_str()) {
        warn!(stage = %s, "unknown stage in fast_stages, ignored");
    }
}

let planner_llm = if effective_fast.iter().any(|s| s == "planner") { &llm_fast } else { &llm };
let summarizer_llm = if effective_fast.iter().any(|s| s == "summarizer") { &llm_fast } else { &llm };
let publisher_llm = if effective_fast.iter().any(|s| s == "publisher") { &llm_fast } else { &llm };

info!(
    planner = if std::ptr::eq(planner_llm, &llm_fast) { "fast" } else { "heavy" },
    summarizer = if std::ptr::eq(summarizer_llm, &llm_fast) { "fast" } else { "heavy" },
    publisher = if std::ptr::eq(publisher_llm, &llm_fast) { "fast" } else { "heavy" },
    "stage routing"
);
```

Then update the three call sites:
- `generate_queries(&llm_fast, ...)` → `generate_queries(planner_llm, ...)`
- `summarize_all(&llm_fast, ...)` → `summarize_all(summarizer_llm, ...)`
- `write_report(&llm, ...)` → `write_report(publisher_llm, ...)`

- [ ] **Step 3: Fix all existing `ResearchRequest` construction sites**

Every place that builds a `ResearchRequest` needs `fast_stages: None`. Search for `ResearchRequest {` in:
- `src/mcp_server.rs` (5 tool handlers + person/company/code/market helpers)
- `src/server.rs` (`into_pipeline_request`)
- `src/main.rs` (CLI entry)

Add `fast_stages: None,` to each.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check`
Expected: success

- [ ] **Step 5: Smoke test**

Run:
```bash
LLM_BASE_URL=http://localhost:30080/v1 \
LLM_FAST_BASE_URL=http://localhost:30081/v1 \
LLM_FAST_MODEL=Qwen3.5-4B-Q4_K_M \
SEARXNG_URL=http://localhost:4000 \
STRIP_THINKING_TOKENS=true \
RUST_LOG=info \
LLM_FAST_STAGES=planner,summarizer \
cargo run --release --bin researcher -- --query "test query" --mode quick 2>&1 | grep "stage routing"
```
Expected: `stage routing planner=fast summarizer=fast publisher=heavy`

Then test with all-fast:
```bash
LLM_FAST_STAGES=planner,summarizer,publisher \
cargo run --release --bin researcher -- --query "test query" --mode quick 2>&1 | grep "stage routing"
```
Expected: `stage routing planner=fast summarizer=fast publisher=fast`

- [ ] **Step 6: Commit**

```bash
git add src/researcher/pipeline.rs src/mcp_server.rs src/server.rs src/main.rs
git commit -m "feat: configurable stage-level model routing in pipeline"
```

---

## Chunk 2: MCP + HTTP API + Docs

### Task 3: Add `fast_stages` to all MCP input structs

**Files:**
- Modify: `src/mcp_server.rs` — add field to 5 input structs and wire through to `ResearchRequest`

- [ ] **Step 1: Add field to all 5 input structs**

Add to `ResearchInput`, `PersonResearchInput`, `CompanyResearchInput`, `CodeResearchInput`, and `MarketInsightInput`:

```rust
#[schemars(description = "Override which pipeline stages use the fast LLM: planner, summarizer, publisher. Default from config.")]
pub fast_stages: Option<Vec<String>>,
```

- [ ] **Step 2: Wire through in each tool handler**

In each of the 5 tool handlers (`research`, `research_person`, `research_company`, `research_code`, `market_insight`), where `ResearchRequest` is built, replace `fast_stages: None` with:

```rust
fast_stages: input.fast_stages.map(|v| v.iter().map(|s| s.trim().to_lowercase()).collect()),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: success

- [ ] **Step 4: Commit**

```bash
git add src/mcp_server.rs
git commit -m "feat: add fast_stages param to all MCP tools"
```

### Task 4: Add `fast_stages` to HTTP API

**Files:**
- Modify: `src/server.rs` — add field to `ResearchBody` and `into_pipeline_request()`

- [ ] **Step 1: Add field to ResearchBody**

```rust
#[serde(default)]
pub fast_stages: Option<Vec<String>>,
```

- [ ] **Step 2: Wire through in `into_pipeline_request()`**

Add to the `ResearchRequest` construction:

```rust
fast_stages: body.fast_stages.map(|v| v.iter().map(|s| s.trim().to_lowercase()).collect()),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: success

- [ ] **Step 4: Commit**

```bash
git add src/server.rs
git commit -m "feat: add fast_stages param to HTTP API"
```

### Task 5: Update MCP instructions and docs

**Files:**
- Modify: `src/mcp_server.rs` — update `get_info()` instructions
- Modify: `CLAUDE.md` — add env var row
- Modify: `.env.example` — add variable
- Modify: `docker-compose.yml` — add env var to researcher service

- [ ] **Step 1: Update `get_info()` in mcp_server.rs**

In the `with_instructions(...)` block, update each tool's description to mention the `fast_stages` parameter. For example, the `research` tool line becomes:

```
• research(query, mode?, domain_profile?, domains?, max_queries?, max_sources?, fast_stages?)
```

- [ ] **Step 2: Add to CLAUDE.md env vars table**

Add row after `LLM_FAST_MAX_TOKENS`:

```
| `LLM_FAST_STAGES` | `planner,summarizer` | Comma-separated pipeline stages using fast LLM (planner, summarizer, publisher) |
```

- [ ] **Step 3: Add to .env.example**

After the `LLM_FAST_MAX_TOKENS` line:

```
LLM_FAST_STAGES=planner,summarizer    # Pipeline stages using fast LLM (planner, summarizer, publisher)
```

- [ ] **Step 4: Add to docker-compose.yml**

In the researcher service `environment:` section, add:

```yaml
- LLM_FAST_STAGES=${LLM_FAST_STAGES:-planner,summarizer}
```

- [ ] **Step 5: Build release and smoke test via MCP**

```bash
cargo build --release
```

After build, restart MCP server (`/mcp` → restart), then test:
```
research("test query", mode="quick", fast_stages=["planner","summarizer","publisher"])
```

Verify in llama-cpp-fast container logs that it received the planner request.

- [ ] **Step 6: Commit**

```bash
git add src/mcp_server.rs CLAUDE.md .env.example docker-compose.yml
git commit -m "docs: add LLM_FAST_STAGES to config and MCP instructions"
```
