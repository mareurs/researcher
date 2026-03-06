# Rich Research Request Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `ResearchMode` (quick/summary/report/deep), domain profiles, and structured output to the researcher pipeline, exposed through MCP, HTTP, and CLI.

**Architecture:** A new `ResearchRequest` struct carries all per-call params (topic, mode, domains, domain_profile) and flows through `run()` alongside `Config`. Domain profiles are loaded from `profiles.toml` at startup into `Config::profiles`. The quick mode short-circuits after crawling, skipping LLM entirely.

**Tech Stack:** Rust, `toml = "0.8"` (new dep), existing clap/serde/rmcp/axum stack.

---

## Prerequisite: read these files before starting

Before touching any file, read its current body:
- `src/researcher/pipeline.rs` — `run()`, `ResearchResult`, `ProgressEvent`
- `src/researcher/planner.rs` — `generate_queries()`
- `src/researcher/publisher.rs` — `write_report()`
- `src/mcp_server.rs` — `ResearchInput`, `research()` method
- `src/server.rs` — `ResearchRequest` (HTTP body), `research_json`, `research_stream`
- `src/main.rs` — `run_cli()`
- `src/config.rs` — `Config` struct

Use `find_symbol(name, path, include_body=true)` — never `read_file` on `.rs` files.

---

## Task 1: Add `toml` dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependency**

In `Cargo.toml` under `[dependencies]`, after the `serde_json` line, add:
```toml
toml = "0.8"
```

**Step 2: Verify it resolves**

```bash
cargo check 2>&1 | head -5
```
Expected: no error about `toml`.

---

## Task 2: Define `ResearchMode`, `ResearchRequest`, `SourceEntry` in pipeline.rs

**Files:**
- Modify: `src/researcher/pipeline.rs`

This task adds the new types. `run()` signature change comes in Task 6.

**Step 1: Add `ResearchMode` enum after the imports block**

Use `insert_code` positioned before the `run` function:

```rust
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchMode {
    Quick,
    Summary,
    #[default]
    Report,
    Deep,
}

pub struct ResearchRequest {
    pub topic: String,
    pub mode: ResearchMode,
    /// Raw domain list (e.g. ["olx.ro", "reddit.com"])
    pub domains: Vec<String>,
    /// Named profile from profiles.toml (e.g. "shopping-ro")
    pub domain_profile: Option<String>,
}

impl ResearchRequest {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            mode: ResearchMode::default(),
            domains: vec![],
            domain_profile: None,
        }
    }
}

/// A single crawled source returned to the caller.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SourceEntry {
    pub url: String,
    pub title: String,
    pub snippet: String,
}
```

**Step 2: Update `ResearchResult`**

Current body:
```rust
pub struct ResearchResult {
    pub report: String,
    pub sources: Vec<SourceSummary>,
}
```

New body (use `replace_symbol`):
```rust
pub struct ResearchResult {
    /// None in quick mode (no LLM report generated)
    pub report: Option<String>,
    pub sources: Vec<SourceEntry>,
    pub queries: Vec<String>,
}
```

**Step 3: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: errors about `ResearchResult` fields mismatch at call sites — these get fixed in later tasks. No errors *inside* pipeline.rs at this point.

---

## Task 3: Add domain profiles to `Config`

**Files:**
- Modify: `src/config.rs`

**Step 1: Add `profiles` field to `Config` struct**

Add after the `output` field (use `insert_code` after the `output` symbol, or `replace_symbol` on Config):

```rust
/// Domain profiles loaded from profiles.toml at startup. Not a CLI flag.
#[clap(skip)]
pub profiles: std::collections::HashMap<String, Vec<String>>,
```

**Step 2: Add CLI flags for mode and domains**

Add after `output` field:
```rust
/// Research mode: quick, summary, report (default), deep
#[clap(long, env = "RESEARCH_MODE", default_value = "report")]
pub mode: String,

/// Named domain profile from profiles.toml
#[clap(long, env = "DOMAIN_PROFILE")]
pub domain_profile: Option<String>,

/// Comma-separated domains to restrict search to
#[clap(long, value_delimiter = ',', env = "DOMAINS")]
pub cli_domains: Vec<String>,
```

**Step 3: Add `load_profiles` function at bottom of config.rs**

```rust
/// Load domain profiles from `profiles.toml` in the current directory.
/// Returns empty map if file is missing or malformed.
pub fn load_profiles() -> std::collections::HashMap<String, Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ProfileEntry {
        domains: Vec<String>,
    }

    let Ok(content) = std::fs::read_to_string("profiles.toml") else {
        return Default::default();
    };
    let Ok(raw) = toml::from_str::<std::collections::HashMap<String, ProfileEntry>>(&content) else {
        tracing::warn!("profiles.toml parse failed — using empty profiles");
        return Default::default();
    };
    raw.into_iter().map(|(k, v)| (k, v.domains)).collect()
}
```

**Step 4: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: errors at call sites in `main.rs` and `mcp_server.rs` about `Config` missing `profiles` during construction — fixed in Tasks 7–9. No errors inside config.rs itself.

---

## Task 4: Update `generate_queries()` to inject domain filters

**Files:**
- Modify: `src/researcher/planner.rs`

Read the current body first: `find_symbol("generate_queries", path="src/researcher/planner.rs", include_body=true)`

**Step 1: Change signature**

Old: `fn generate_queries(llm: &LlmClient, topic: &str, max: usize) -> Result<Vec<String>>`
New: `fn generate_queries(llm: &LlmClient, topic: &str, max: usize, domains: &[String]) -> Result<Vec<String>>`

Use `replace_symbol` to update the full body. Keep the existing prompt but append domain instructions when `domains` is non-empty:

```rust
pub async fn generate_queries(
    llm: &LlmClient,
    topic: &str,
    max: usize,
    domains: &[String],
) -> Result<Vec<String>> {
    let domain_instruction = if domains.is_empty() {
        String::new()
    } else {
        let site_list = domains
            .iter()
            .map(|d| format!("site:{d}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        format!(
            "\n\nIMPORTANT: Restrict ALL queries to these domains only. \
             Each query MUST include a site filter, e.g.: {site_list}\n\
             Domain list: {}",
            domains.join(", ")
        )
    };

    // ── keep the rest of the existing function body unchanged ──
    // just append `domain_instruction` to the user prompt string
```

**Important:** Read the existing `generate_queries` body fully before replacing. Append `domain_instruction` to the user message string, do not change the JSON parsing logic.

**Step 2: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: error at the `generate_queries` call site in `pipeline.rs` about missing argument — fixed in Task 6.

---

## Task 5: Mode-aware prompts in `publisher.rs`

**Files:**
- Modify: `src/researcher/publisher.rs`

Read current body first: `find_symbol("write_report", path="src/researcher/publisher.rs", include_body=true)`

**Step 1: Update `write_report` signature and add mode-based prompt**

Old: `fn write_report(llm, topic, summaries, token_tx) -> Result<String>`
New: `fn write_report(llm, topic, summaries, mode, token_tx) -> Result<String>`

Add `mode: &ResearchMode` as the 4th parameter (before `token_tx`). Add import at top:
```rust
use crate::researcher::pipeline::ResearchMode;
```

Inside the function, replace the hardcoded prompt string with:

```rust
let prompt = match mode {
    ResearchMode::Summary => format!(
        "Using the research context below, write a concise bullet-point summary:\n\
         - 5-8 key findings as bullet points\n\
         - Each bullet: one concrete fact or conclusion\n\
         - Prioritize actionable facts, numbers, and dates\n\
         - No introduction, no conclusion, no section headers\n\n\
         Topic: {topic}\n\nSources:\n{sources_text}"
    ),
    ResearchMode::Deep => format!(
        "Using the research context below, write a thorough, detailed report on: {topic}\n\
         - Begin with an executive summary (2-3 paragraphs)\n\
         - Cover all major angles with dedicated sections and headers\n\
         - Include specific facts, numbers, dates, and source references\n\
         - Conclude with key takeaways and open questions\n\
         - Use markdown formatting throughout\n\nSources:\n{sources_text}"
    ),
    _ => format!(
        // existing default prompt — keep whatever is currently there
        // just substitute {topic} and {sources_text}
    ),
};
```

**Important:** Read the existing prompt string before replacing — preserve the variable names used to build `sources_text` etc. Only the final prompt string changes.

**Step 2: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: error at `write_report` call site in pipeline.rs — fixed in Task 6.

---

## Task 6: Update `run()` in pipeline.rs — wire everything together

**Files:**
- Modify: `src/researcher/pipeline.rs`

This is the central task. Read the full current body first:
`find_symbol("run", path="src/researcher/pipeline.rs", include_body=true)`

**Step 1: Replace `run()` signature and body**

Use `replace_symbol("run", path="src/researcher/pipeline.rs", new_body=...)`.

New signature:
```rust
pub async fn run(
    cfg: &Config,
    request: &ResearchRequest,
    on_progress: impl Fn(ProgressEvent),
    token_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<ResearchResult>
```

New body (keep all existing logic, just adapt):

```rust
{
    let topic = &request.topic;

    // ── resolve effective domains (profile union raw) ──────────────
    let mut domains: Vec<String> = request
        .domain_profile
        .as_deref()
        .and_then(|p| cfg.profiles.get(p))
        .cloned()
        .unwrap_or_default();
    for d in &request.domains {
        if !domains.contains(d) {
            domains.push(d.clone());
        }
    }

    // ── effective depth multiplier for deep mode ───────────────────
    let depth = matches!(request.mode, ResearchMode::Deep);
    let max_queries = if depth {
        cfg.max_search_queries * 2
    } else {
        cfg.max_search_queries
    };
    let max_sources = if depth {
        cfg.max_sources_per_query * 2
    } else {
        cfg.max_sources_per_query
    };

    // ── keep all existing client setup (llm, http) ────────────────
    // [copy existing let llm = ...; let http = ...; lines here]

    on_progress(ProgressEvent::Planning);
    let queries = generate_queries(&llm, topic, max_queries, &domains).await?;
    on_progress(ProgressEvent::Queries(queries.clone()));   // NOTE: may need to add Queries variant

    // ── crawl (use max_sources instead of cfg.max_sources_per_query) ──
    on_progress(ProgressEvent::Crawling { total: queries.len() });
    // [keep existing crawl_all call, pass max_sources via a cfg override or directly]

    // ... [keep dedup/rerank logic unchanged] ...

    // ── build SourceEntry vec from scraped sources ─────────────────
    let source_entries: Vec<SourceEntry> = sources
        .iter()
        .map(|s| SourceEntry {
            url: s.url.clone(),
            title: s.title.clone(),
            snippet: s.content.chars().take(200).collect(),
        })
        .collect();

    // ── quick mode: skip LLM, return sources only ──────────────────
    if matches!(request.mode, ResearchMode::Quick) {
        on_progress(ProgressEvent::Done);
        return Ok(ResearchResult {
            report: None,
            sources: source_entries,
            queries,
        });
    }

    // ── summarize ─────────────────────────────────────────────────
    on_progress(ProgressEvent::Summarizing { total: sources.len() });
    let summaries = summarize_all(&llm, &sources, topic).await;
    on_progress(ProgressEvent::SummarizingComplete { summaries: summaries.len() });

    // ── write report (mode-aware) ──────────────────────────────────
    on_progress(ProgressEvent::WritingReport);
    let raw_report = write_report(&llm, topic, &summaries, &request.mode, token_tx).await?;
    let report = format_report(&raw_report, &summaries);

    on_progress(ProgressEvent::Done);
    Ok(ResearchResult {
        report: Some(report),
        sources: source_entries,
        queries,
    })
}
```

**Important notes for implementer:**
- Copy the existing `let llm = ...` and `let http = ...` setup blocks verbatim — do not rewrite them
- The `crawl_all` function takes `cfg` — for `max_sources` override in deep mode, temporarily clone cfg or pass the value directly. Simplest: create `let mut eff_cfg = cfg.clone(); eff_cfg.max_sources_per_query = max_sources;` and pass `&eff_cfg`
- Add `Queries(Vec<String>)` variant to `ProgressEvent` if it doesn't exist, with a Display impl line

**Step 2: Add `Queries` variant to `ProgressEvent` if missing**

Check the enum — if `Queries` is not there, add it after `Planning`:
```rust
Queries(Vec<String>),
```
And in the Display impl:
```rust
ProgressEvent::Queries(q) => write!(f, "Generated {} queries", q.len()),
```

**Step 3: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: errors at MCP/HTTP/CLI call sites. No errors in `src/researcher/`.

---

## Task 7: Update MCP server

**Files:**
- Modify: `src/mcp_server.rs`

Read these first:
- `find_symbol("ResearchInput", path="src/mcp_server.rs", include_body=true)`
- `find_symbol("research", path="src/mcp_server.rs", include_body=true)`
- `find_symbol("config_from_env", path="src/mcp_server.rs", include_body=true)`

**Step 1: Extend `ResearchInput`**

Use `replace_symbol("ResearchInput", ...)`:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResearchInput {
    #[schemars(description = "Research topic or question")]
    pub query: String,

    #[schemars(description = "quick=links+snippets only, summary=bullet facts, report=full markdown (default), deep=thorough long-form")]
    pub mode: Option<String>,

    #[schemars(description = "Named domain profile: shopping-ro, tech-news, llm-news, academic, news, travel")]
    pub domain_profile: Option<String>,

    #[schemars(description = "Raw domain list, e.g. [\"olx.ro\", \"reddit.com/r/LocalLLaMA\"]")]
    pub domains: Option<Vec<String>>,

    #[schemars(description = "Override max search sub-queries (default from config)")]
    pub max_queries: Option<usize>,

    #[schemars(description = "Override max sources per query (default from config)")]
    pub max_sources: Option<usize>,
}
```

**Step 2: Update `research()` method**

Replace `research` method body. Remove the `cfg` mutation hack. Build a `ResearchRequest`:

```rust
#[tool(description = "Web research agent.\nModes: quick=links+snippets, summary=bullet facts, report=full markdown (default), deep=thorough.\nProfiles: shopping-ro, tech-news, llm-news, academic, news, travel. Or pass domains:[\"site\"] directly.")]
async fn research(
    &self,
    Parameters(input): Parameters<ResearchInput>,
) -> String {
    use crate::researcher::pipeline::{ResearchMode, ResearchRequest, run};

    let mode = match input.mode.as_deref().unwrap_or("report") {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };

    let mut cfg = (*self.cfg).clone();
    if let Some(mq) = input.max_queries  { cfg.max_search_queries   = mq; }
    if let Some(ms) = input.max_sources  { cfg.max_sources_per_query = ms; }

    let request = ResearchRequest {
        topic: input.query,
        mode,
        domains: input.domains.unwrap_or_default(),
        domain_profile: input.domain_profile,
    };

    let rt = tokio::runtime::Handle::current();
    let result = rt.block_on(run(
        &cfg,
        &request,
        |ev| eprintln!("[researcher] {ev}"),
        None,
    ));

    match result {
        Ok(r) => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
        Err(e) => format!("Error: {e:#}"),
    }
}
```

**Step 3: Wire `load_profiles` in `config_from_env`**

At the end of `config_from_env()`, before returning `Config { ... }`, add:
```rust
profiles: crate::config::load_profiles(),
```

**Step 4: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: only server.rs and main.rs errors remain.

---

## Task 8: Update HTTP server

**Files:**
- Modify: `src/server.rs`

Read: `find_symbol("ResearchRequest", path="src/server.rs", include_body=true)`  
Read: `find_symbol("research_json", path="src/server.rs", include_body=true)`  
Read: `find_symbol("research_stream", path="src/server.rs", include_body=true)`

**Step 1: Rename `ResearchRequest` → `ResearchBody` and extend**

Use `rename_symbol("ResearchRequest", path="src/server.rs", new_name="ResearchBody")` — this renames all usages in the file.

Then use `replace_symbol("ResearchBody", ...)` to extend the struct:

```rust
#[derive(serde::Deserialize)]
pub struct ResearchBody {
    pub query: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub domain_profile: Option<String>,
    #[serde(default)]
    pub domains: Option<Vec<String>>,
}
```

**Step 2: Add helper to build `ResearchRequest` from `ResearchBody`**

Add a free function after `ResearchBody`:

```rust
fn into_pipeline_request(body: ResearchBody) -> crate::researcher::pipeline::ResearchRequest {
    use crate::researcher::pipeline::{ResearchMode, ResearchRequest};
    let mode = match body.mode.as_deref().unwrap_or("report") {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };
    ResearchRequest {
        topic: body.query,
        mode,
        domains: body.domains.unwrap_or_default(),
        domain_profile: body.domain_profile,
    }
}
```

**Step 3: Update `research_json` and `research_stream`**

In both handlers, replace the `run(&cfg, &req.query, ...)` call with:
```rust
let request = into_pipeline_request(req);
run(&cfg, &request, ...).await
```

Update the `ResearchResponse` returned from `research_json` to match the new `ResearchResult`:
- `report` is now `Option<String>` — use `.unwrap_or_default()`
- `source_count` — use `result.sources.len()`

**Step 4: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: only main.rs errors remain.

---

## Task 9: Update CLI (`main.rs`)

**Files:**
- Modify: `src/main.rs`

Read: `find_symbol("run_cli", path="src/main.rs", include_body=true)`

**Step 1: Update `run_cli` to build `ResearchRequest`**

Replace `run_cli` body. The `cfg.mode`, `cfg.domain_profile`, `cfg.cli_domains` fields (added in Task 3) drive the request:

```rust
async fn run_cli(cfg: Config) -> anyhow::Result<()> {
    use crate::researcher::pipeline::{ResearchMode, ResearchRequest, run};

    let topic = cfg.query.clone().unwrap_or_else(|| {
        eprintln!("No query provided. Use --query \"your topic\"");
        std::process::exit(1);
    });

    let mode = match cfg.mode.as_str() {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };

    let request = ResearchRequest {
        topic,
        mode,
        domains: cfg.cli_domains.clone(),
        domain_profile: cfg.domain_profile.clone(),
    };

    // ── keep existing streaming token setup unchanged ──────────────
    // [copy the existing (token_tx, token_rx) / print_task / run() call]
    // just change: run(&cfg, &topic, ...) → run(&cfg, &request, ...)
```

**Step 2: Wire `load_profiles` in `main()`**

After `let mut cfg = Config::parse();`, add:
```rust
cfg.profiles = crate::config::load_profiles();
```

**Step 3: Cargo check**

```bash
cargo check 2>&1 | grep "^error" | head -20
```
Expected: clean compile or only warnings.

---

## Task 10: Create `profiles.toml`

**Files:**
- Create: `profiles.toml`

```toml
# Domain profiles for the researcher agent.
# Each profile restricts searches to the listed domains.
# Pass domain_profile="<key>" to any research call.

[shopping-ro]
domains = ["olx.ro", "publi24.ro", "okazii.ro", "emag.ro", "altex.ro"]

[tech-news]
domains = [
  "news.ycombinator.com",
  "lobste.rs",
  "reddit.com/r/programming",
  "reddit.com/r/rust",
  "reddit.com/r/technology",
]

[llm-news]
domains = [
  "huggingface.co",
  "arxiv.org",
  "reddit.com/r/LocalLLaMA",
  "reddit.com/r/MachineLearning",
  "reddit.com/r/artificial",
]

[academic]
domains = [
  "arxiv.org",
  "semanticscholar.org",
  "pubmed.ncbi.nlm.nih.gov",
]

[news]
domains = [
  "reddit.com/r/worldnews",
  "reddit.com/r/news",
  "reddit.com/r/europe",
  "bbc.com",
  "reuters.com",
]

[travel]
domains = [
  "reddit.com/r/travel",
  "reddit.com/r/solotravel",
  "tripadvisor.com",
  "lonelyplanet.com",
  "wikivoyage.org",
]
```

---

## Task 11: Final verification

**Step 1: Full cargo check**

```bash
cargo check 2>&1
```
Expected: exit 0, zero errors. Warnings about unused fields are acceptable.

**Step 2: Cargo check both binaries explicitly**

```bash
cargo check --bin researcher && cargo check --bin researcher-mcp
```

**Step 3: Smoke test MCP serialization**

Build and run a quick JSON roundtrip to verify ResearchResult serializes:

```bash
cargo build --bin researcher 2>&1 | tail -5
```

**Step 4: Update CLAUDE.md env var table**

Add to the env vars table:
```
| `RESEARCH_MODE`   | `report` | CLI default mode |
| `DOMAIN_PROFILE`  | `` | Named profile from profiles.toml |
| `DOMAINS`         | `` | Comma-separated domain override |
```

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add ResearchMode, domain profiles, structured ResearchResult output"
```

---

## MCP Tool Description (final, slim)

The `#[tool(description = ...)]` on the `research` method should read:

```
Web research agent.
Modes: quick=links+snippets, summary=bullet facts, report=full markdown (default), deep=thorough.
Profiles: shopping-ro, tech-news, llm-news, academic, news, travel. Or pass domains:["site"] directly.
```

This is 3 lines / ~200 chars — Claude learns all capabilities with minimal token spend.

---

## Key Gotchas

1. **`crawl_all` takes `&Config`** — for deep mode's doubled `max_sources_per_query`, clone cfg and override the field before passing. This is the only place cfg is cloned per-request.
2. **`ResearchResult` derives `serde::Serialize`** — add this derive so MCP can `serde_json::to_string_pretty` the result.
3. **`SourceEntry` derives `serde::Serialize`** — same reason.
4. **`ResearchMode` in `serde` round-trips** — the `#[serde(rename_all = "snake_case")]` means `"quick"`, `"summary"`, `"report"`, `"deep"` from JSON/env.
5. **Profiles path is relative to CWD** — when running via MCP, CWD is wherever the shell launched from. Document in README that `profiles.toml` must be in CWD or symlinked.
