# Progressive Disclosure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a research report exceeds a char threshold, save it to disk and return a compact summary + TOC + path instead of dumping the full report into the agent's context.

**Architecture:** Post-`run()` hook in the MCP layer. A new `src/researcher/progressive.rs` module provides three pure functions (`extract_toc`, `write_to_disk`, `generate_summary`) and one async entry point (`maybe_disclose`). All 5 research MCP tools and the HTTP JSON endpoint call `maybe_disclose` after `run()`. Reports below `PROGRESSIVE_DISCLOSURE_CHARS` (default 4000) return inline unchanged.

**Tech Stack:** Rust, `chrono 0.4` (new dep for timestamp formatting), `anyhow`, `serde_json`, existing `LlmClient`.

---

## File Map

| File | Change |
|---|---|
| `Cargo.toml` | Add `chrono = "0.4"` |
| `src/config.rs` | Add `progressive_disclosure_chars: usize` field |
| `src/mcp_server.rs` `config_from_env()` | Mirror `PROGRESSIVE_DISCLOSURE_CHARS` |
| `src/researcher/progressive.rs` | **New** — all progressive disclosure logic |
| `src/researcher/mod.rs` | Add `pub mod progressive` |
| `src/mcp_server.rs` structs | Add `summary_style: Option<String>` to 5 input structs |
| `src/mcp_server.rs` tool methods | Call `maybe_disclose` in 5 tool methods |
| `src/mcp_server.rs` `get_info()` | Document new param + envelope shape |
| `src/server.rs` | Add `summary_style` to `ResearchBody`, wire `research_json` |

---

## Task 1: Add `chrono` dep and `progressive_disclosure_chars` config field

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/config.rs` (end of `Config` struct, around line 196)
- Modify: `src/mcp_server.rs` (`config_from_env`, around line 532)

- [ ] **Step 1: Add chrono to Cargo.toml**

In `Cargo.toml`, add to `[dependencies]` after `anyhow = "1"`:

```toml
chrono = "0.4"
```

- [ ] **Step 2: Add config field to `src/config.rs`**

Find the `Config` struct's last field (around line 192):
```rust
    #[clap(skip)]
    pub job_profile: Option<JobProfile>,
}
```

Replace with:
```rust
    #[clap(skip)]
    pub job_profile: Option<JobProfile>,

    /// Char length above which MCP tools save the report to disk and return a compact envelope.
    /// Set to 0 to always return inline (disables feature).
    #[arg(long, env = "PROGRESSIVE_DISCLOSURE_CHARS", default_value = "4000")]
    pub progressive_disclosure_chars: usize,
}
```

- [ ] **Step 3: Mirror in `config_from_env()` in `src/mcp_server.rs`**

In `config_from_env()` (around line 532), add alongside the other `env_usize` calls:

```rust
        progressive_disclosure_chars: env_usize("PROGRESSIVE_DISCLOSURE_CHARS", 4000),
```

- [ ] **Step 4: Verify**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/config.rs src/mcp_server.rs
git commit -m "feat: add progressive_disclosure_chars config field"
```

---

## Task 2: Create `src/researcher/progressive.rs`

**Files:**
- Create: `src/researcher/progressive.rs`
- Modify: `Cargo.toml` (add `tempfile` dev-dep)

- [ ] **Step 1: Write tests + stub signatures**

Create `src/researcher/progressive.rs`:

```rust
use std::path::PathBuf;
use anyhow::Result;
use chrono::Local;
use crate::llm::client::{ChatMessage, LlmClient};
use crate::researcher::pipeline::ResearchResult;

#[derive(Debug, serde::Serialize)]
pub struct ProgressiveEnvelope {
    pub summary: String,
    pub toc: Vec<String>,
    pub path: String,
    pub word_count: usize,
    pub hint: &'static str,
}

pub enum MaybeDisclosed {
    Inline(String),
    Progressive(String),
}

impl MaybeDisclosed {
    pub fn into_string(self) -> String {
        match self {
            MaybeDisclosed::Inline(s) | MaybeDisclosed::Progressive(s) => s,
        }
    }
}

pub fn extract_toc(report: &str) -> Vec<String> {
    todo!()
}

pub fn write_to_disk(topic: &str, report: &str) -> Result<PathBuf> {
    todo!()
}

pub async fn generate_summary(llm: &LlmClient, report: &str, style: &str) -> Result<String> {
    todo!()
}

pub async fn maybe_disclose(
    llm: &LlmClient,
    topic: &str,
    result: &ResearchResult,
    summary_style: &str,
    threshold: usize,
) -> Result<MaybeDisclosed> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_toc_returns_h2_headings() {
        let report = "# Title\n\nSome text.\n\n## Executive Summary\n\nMore.\n\n## Key Findings\n\n- foo\n\n### Subsection\n\n## Sources\n";
        let toc = extract_toc(report);
        assert_eq!(toc, vec![
            "## Executive Summary".to_string(),
            "## Key Findings".to_string(),
            "## Sources".to_string(),
        ]);
    }

    #[test]
    fn extract_toc_empty_report() {
        assert!(extract_toc("").is_empty());
    }

    #[test]
    fn extract_toc_no_h2() {
        let report = "# Title\n\n### Only h3\n\ntext";
        assert!(extract_toc(report).is_empty());
    }

    #[test]
    fn write_to_disk_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let path = write_to_disk("test topic", "# Report\n\nContent here.").unwrap();
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "# Report\n\nContent here.");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-test-topic.md"), "got: {name}");
    }

    #[test]
    fn write_to_disk_slug_strips_specials() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let path = write_to_disk("Rust async/await: deep dive!", "content").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.'));
    }
}
```

- [ ] **Step 2: Add `tempfile` dev-dependency**

In `Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Run tests to confirm they fail (not panic)**

```bash
cargo test progressive 2>&1 | head -30
```

Expected: compile error or `todo!()` panics — confirms tests are wired up.

- [ ] **Step 4: Implement `extract_toc`**

Replace `todo!()` in `extract_toc`:
```rust
pub fn extract_toc(report: &str) -> Vec<String> {
    report
        .lines()
        .filter(|line| line.starts_with("## "))
        .map(str::to_string)
        .collect()
}
```

- [ ] **Step 5: Implement `write_to_disk`**

Replace `todo!()` in `write_to_disk`:
```rust
pub fn write_to_disk(topic: &str, report: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let data_dir = PathBuf::from(home).join(".local/share/researcher");
    std::fs::create_dir_all(&data_dir)?;

    let slug: String = topic
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect();

    let ts = Local::now().format("%Y-%m-%dT%H-%M-%S");
    let filename = format!("{ts}-{slug}.md");
    let path = data_dir.join(filename);
    std::fs::write(&path, report)?;
    Ok(path)
}
```

- [ ] **Step 6: Run tests — 5 tests should pass**

```bash
cargo test progressive 2>&1 | tail -15
```

Expected:
```
test researcher::progressive::tests::extract_toc_empty_report ... ok
test researcher::progressive::tests::extract_toc_no_h2 ... ok
test researcher::progressive::tests::extract_toc_returns_h2_headings ... ok
test researcher::progressive::tests::write_to_disk_creates_file ... ok
test researcher::progressive::tests::write_to_disk_slug_strips_specials ... ok
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/researcher/progressive.rs
git commit -m "feat: progressive module — extract_toc and write_to_disk with tests"
```

---

## Task 3: Register module and implement async fns

**Files:**
- Modify: `src/researcher/mod.rs`
- Modify: `src/researcher/progressive.rs`

- [ ] **Step 1: Register module**

Append to `src/researcher/mod.rs`:
```rust
pub mod progressive;
```

- [ ] **Step 2: Implement `generate_summary`**

Replace `todo!()` in `generate_summary`:
```rust
pub async fn generate_summary(llm: &LlmClient, report: &str, style: &str) -> Result<String> {
    if style == "toc" {
        return Ok(extract_toc(report).join("\n"));
    }

    let prompt = match style {
        "takeaways" => format!(
            "Extract the 5-8 most important findings from this research report as a concise bullet list. \
             Each bullet should be one specific fact, number, or conclusion. No introduction or headers.\n\n{report}"
        ),
        _ => format!(
            "Write a 3-5 sentence abstract summarizing the key findings of this research report. \
             Be specific — include key facts, numbers, and conclusions. No headers.\n\n{report}"
        ),
    };

    let messages = vec![
        ChatMessage::system("You are a research analyst. Summarize research reports concisely and accurately."),
        ChatMessage::user(prompt),
    ];
    llm.complete(messages).await
}
```

- [ ] **Step 3: Implement `maybe_disclose`**

Replace `todo!()` in `maybe_disclose`:
```rust
pub async fn maybe_disclose(
    llm: &LlmClient,
    topic: &str,
    result: &ResearchResult,
    summary_style: &str,
    threshold: usize,
) -> Result<MaybeDisclosed> {
    let report = match &result.report {
        Some(r) => r,
        None => {
            return Ok(MaybeDisclosed::Inline(serde_json::to_string_pretty(result)?));
        }
    };

    if threshold > 0 && report.len() <= threshold {
        return Ok(MaybeDisclosed::Inline(serde_json::to_string_pretty(result)?));
    }

    let path = match write_to_disk(topic, report) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("progressive disclosure: disk write failed ({e:#}), returning inline");
            return Ok(MaybeDisclosed::Inline(serde_json::to_string_pretty(result)?));
        }
    };

    let summary = match generate_summary(llm, report, summary_style).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("progressive disclosure: summary failed ({e:#}), using toc");
            extract_toc(report).join("\n")
        }
    };

    let envelope = ProgressiveEnvelope {
        toc: extract_toc(report),
        word_count: report.split_whitespace().count(),
        path: path.to_string_lossy().into_owned(),
        summary,
        hint: "Full report saved to disk. Read the file at 'path' using a file-reading tool, or read specific sections by heading.",
    };

    Ok(MaybeDisclosed::Progressive(serde_json::to_string_pretty(&envelope)?))
}
```

- [ ] **Step 4: cargo check**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/researcher/mod.rs src/researcher/progressive.rs
git commit -m "feat: progressive disclosure — generate_summary and maybe_disclose"
```

---

## Task 4: Wire into MCP tool methods

**Files:**
- Modify: `src/mcp_server.rs`

- [ ] **Step 1: Add `summary_style` to all 5 input structs**

For each struct, add before the closing `}`. The `#[schemars]` attribute makes the field appear in the MCP tool schema:

`ResearchInput`:
```rust
    /// How to summarize when report exceeds threshold: "toc", "abstract" (default), "takeaways"
    #[schemars(description = "Summary style when report exceeds size threshold: toc | abstract (default) | takeaways")]
    pub summary_style: Option<String>,
```

Apply identically to `PersonResearchInput`, `CompanyResearchInput`, `CodeResearchInput`, `MarketInsightInput`.

- [ ] **Step 2: Add imports**

Near the top of `src/mcp_server.rs`:
```rust
use crate::researcher::progressive::maybe_disclose;
use crate::llm::client::LlmClient;
```

- [ ] **Step 3: Update `research` method**

Find and replace the `match run(...).await` block:
```rust
        match run(
            &cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await
        {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
```

Replace with:
```rust
        match run(
            &cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await
        {
            Ok(r) => {
                let llm = LlmClient::new(&cfg);
                let style = input.summary_style.as_deref().unwrap_or("abstract");
                match maybe_disclose(&llm, &request.topic, &r, style, cfg.progressive_disclosure_chars).await {
                    Ok(d)  => d.into_string(),
                    Err(e) => {
                        eprintln!("[researcher] progressive disclosure failed: {e:#}");
                        serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                    }
                }
            }
            Err(e) => format!("Error: {e:#}"),
        }
```

- [ ] **Step 4: Update `research_person` — topic is `input.name`**

Same replacement pattern. In the `Ok(r)` arm:
```rust
            Ok(r) => {
                let llm = LlmClient::new(&cfg);
                let style = input.summary_style.as_deref().unwrap_or("abstract");
                match maybe_disclose(&llm, &input.name, &r, style, cfg.progressive_disclosure_chars).await {
                    Ok(d)  => d.into_string(),
                    Err(e) => {
                        eprintln!("[researcher] progressive disclosure failed: {e:#}");
                        serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                    }
                }
            }
```

- [ ] **Step 5: Update `research_company` — topic is `input.name`**

Identical to `research_person` replacement above.

- [ ] **Step 6: Update `research_code` — topic is `input.framework`**

Same pattern, `&input.framework` as topic arg.

- [ ] **Step 7: Update `market_insight` — topic is `input.query`**

Same pattern, `&input.query` as topic arg.

- [ ] **Step 8: Update `get_info()` instructions**

In the `with_instructions(...)` string, append after the tool bullets:

```
\n\n## Progressive Disclosure\n\
When a report exceeds PROGRESSIVE_DISCLOSURE_CHARS (default: 4000 chars), the full report\n\
is saved to ~/.local/share/researcher/<timestamp>-<slug>.md and a compact envelope is returned:\n\
  { summary, toc, path, word_count, hint }\n\
Read the full report at 'path' with a standard file-reading tool.\n\n\
summary_style param (optional, all research tools except search_jobs):\n\
  \"abstract\"  — 3-5 sentence LLM abstract + TOC + path (default)\n\
  \"toc\"       — headings only + path, no LLM call (cheapest)\n\
  \"takeaways\" — 5-8 bullet key findings + TOC + path\n\n\
Set PROGRESSIVE_DISCLOSURE_CHARS=0 to always return inline.
```

- [ ] **Step 9: cargo check**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add src/mcp_server.rs
git commit -m "feat: wire progressive disclosure into MCP tool methods"
```

---

## Task 5: Wire into HTTP server

**Files:**
- Modify: `src/server.rs`

- [ ] **Step 1: Add `summary_style` to `ResearchBody`**

In `ResearchBody` struct (around line 22), add after `intent`:
```rust
    pub summary_style: Option<String>,
```

- [ ] **Step 2: Replace `research_json` body**

Current body (lines 81–98):
```rust
async fn research_json(
    State(cfg): State<AppState>,
    Json(req): Json<ResearchBody>,
) -> Response {
    let request = into_pipeline_request(req);
    let result = run(&cfg, &request, |_| {}, None).await;
    match result {
        Ok(r) => Json(serde_json::json!({
            "report": r.report,
            "sources": r.sources,
            "queries": r.queries,
        })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ).into_response(),
    }
}
```

Replace with:
```rust
async fn research_json(
    State(cfg): State<AppState>,
    Json(req): Json<ResearchBody>,
) -> Response {
    use crate::researcher::progressive::maybe_disclose;
    use crate::llm::client::LlmClient;

    let summary_style = req.summary_style.clone().unwrap_or_else(|| "abstract".to_string());
    let request = into_pipeline_request(req);
    let result = run(&cfg, &request, |_| {}, None).await;
    match result {
        Ok(r) => {
            let llm = LlmClient::new(&cfg);
            match maybe_disclose(&llm, &request.topic, &r, &summary_style, cfg.progressive_disclosure_chars).await {
                Ok(d) => (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    d.into_string(),
                ).into_response(),
                Err(e) => {
                    tracing::warn!("progressive disclosure failed: {e:#}");
                    Json(serde_json::json!({
                        "report": r.report,
                        "sources": r.sources,
                        "queries": r.queries,
                    })).into_response()
                }
            }
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ).into_response(),
    }
}
```

- [ ] **Step 3: cargo check**

```bash
cargo check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/server.rs
git commit -m "feat: wire progressive disclosure into HTTP JSON endpoint"
```

---

## Task 6: Release build + smoke tests

- [ ] **Step 1: Full release build**

```bash
cargo build --release
```

Expected: success (LTO takes 30–60s).

- [ ] **Step 2: Run unit tests**

```bash
cargo test progressive
```

Expected: 5 tests pass.

- [ ] **Step 3: Smoke — always-inline path**

```bash
PROGRESSIVE_DISCLOSURE_CHARS=999999 \
LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
./target/release/researcher --query "test" 2>/dev/null | head -5
```

Expected: JSON with `"report":` key (inline, unchanged behavior).

- [ ] **Step 4: Smoke — forced progressive path**

```bash
PROGRESSIVE_DISCLOSURE_CHARS=1 \
LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
./target/release/researcher --query "rust ownership" 2>/dev/null | python3 -m json.tool | grep -E '"path"|"toc"|"word_count"'
```

Expected: `"path"`, `"toc"`, `"word_count"` keys present. Verify file on disk:
```bash
ls ~/.local/share/researcher/ | tail -3
```

- [ ] **Step 5: Rebuild MCP binary and restart**

```bash
cargo build --release --bin researcher-mcp
```

Then restart the MCP server in Claude Code: `/mcp` → restart, or restart the session.
