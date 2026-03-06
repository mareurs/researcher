# Code Research Tool — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `research_code` MCP tool that researches framework bugs, changelogs, breaking changes, releases, and community sentiment using hardcoded query templates (no LLM planner call).

**Architecture:** Two files touched. A new `write_code_report()` function added to `src/researcher/publisher.rs` handles the aspect-aware prompt. A new `CodeResearchInput` struct and `research_code` method added to `src/mcp_server.rs` builds query lists from templates and drives the existing crawl/summarize pipeline.

**Tech Stack:** Rust async, rmcp 1.1 `#[tool]` macro, existing `crawl_all` / `summarize_all` / `LlmClient` — no new dependencies.

---

### Task 1: Add `write_code_report()` to publisher.rs

**Files:**
- Modify: `src/researcher/publisher.rs`

This is a new public async function added after `format_report`. It builds a section-aware prompt from the requested aspects and calls `llm.complete()`.

**Step 1: Insert the function after `format_report`**

Use `insert_code` after `format_report` in `src/researcher/publisher.rs`:

```rust
pub async fn write_code_report(
    llm: &LlmClient,
    summaries: &[SourceSummary],
    framework: &str,
    version: &str,
    aspects: &[String],
) -> Result<String> {
    info!(sources = summaries.len(), "writing code research report");

    let sources_text = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!(
            "--- Source {} ---\nURL: {}\nSummary:\n{}\n",
            i + 1, s.url, s.summary,
        ))
        .collect::<Vec<_>>()
        .join("\n");

    let sections: Vec<&str> = aspects.iter().filter_map(|a| match a.as_str() {
        "bugs"      => Some("## Known Bugs & Issues\nRecent or notable bugs, regressions, and open issues. Include issue numbers and links where available."),
        "changelog" => Some("## Changelog & Breaking Changes\nRecent releases, notable changes, and any breaking changes since the specified version."),
        "community" => Some("## Community Sentiment\nRecent Reddit/HN discussions, developer opinions, pain points, and common complaints or praise."),
        "releases"  => Some("## Releases\nRecent release history with dates, version numbers, and highlights."),
        _           => None,
    }).collect();

    let section_instructions = sections.join("\n\n");

    let prompt = format!(
        "You are a developer-focused research analyst. Write a concise technical report on **{framework} {version}**.\n\
         Cover only these sections (skip any section if no relevant information was found in the sources):\n\n\
         {section_instructions}\n\n\
         Rules:\n\
         - Be specific: include version numbers, dates, issue numbers, PR links\n\
         - Cite sources inline with [N] notation\n\
         - Skip sections with no relevant data rather than speculating\n\
         - No fluff, no introductions, no conclusions — just the sections\n\n\
         Research gathered:\n{sources_text}"
    );

    let messages = vec![
        ChatMessage::system(
            "You are a concise technical research analyst specialising in software frameworks \
             and libraries. Write only what the sources support. Cite inline with [N] notation.",
        ),
        ChatMessage::user(prompt),
    ];

    llm.complete(messages).await
}
```

**Step 2: Verify it compiles**

```bash
cargo check 2>&1 | grep "^error"
```

Expected: no output (no errors).

**Step 3: Commit**

```bash
git add src/researcher/publisher.rs
git commit -m "feat: add write_code_report() to publisher"
```

---

### Task 2: Add `CodeResearchInput` struct to mcp_server.rs

**Files:**
- Modify: `src/mcp_server.rs`

Add the input struct after `JobSearchInput` (around line 90).

**Step 1: Insert struct after `JobSearchInput`**

Use `insert_code` after `JobSearchInput` in `src/mcp_server.rs`:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CodeResearchInput {
    /// Framework or library to research, e.g. "axum", "tokio", "claude code"
    pub framework: String,
    /// Version to target, e.g. "0.8". Defaults to "latest" if omitted.
    pub version: Option<String>,
    /// Aspects to research: bugs, changelog, community, releases.
    /// Defaults to ["bugs", "changelog", "community"] if omitted.
    pub aspects: Option<Vec<String>>,
    /// GitHub repo slug, e.g. "tokio-rs/tokio". Anchors bug/release queries to GitHub.
    pub repo: Option<String>,
}
```

**Step 2: Verify it compiles**

```bash
cargo check 2>&1 | grep "^error"
```

Expected: no output.

**Step 3: Commit**

```bash
git add src/mcp_server.rs
git commit -m "feat: add CodeResearchInput struct"
```

---

### Task 3: Add `research_code` method to `ResearcherServer`

**Files:**
- Modify: `src/mcp_server.rs`

Add the method after `search_jobs` inside `impl ResearcherServer` (around line 249).

**Step 1: Insert method after `search_jobs`**

Use `insert_code` after `impl ResearcherServer/search_jobs` in `src/mcp_server.rs`:

```rust
#[tool(description = "Research a framework or library: known bugs, changelogs, breaking changes, \
releases, and community sentiment. \
aspects: bugs, changelog, community, releases — pass one or more (default: bugs+changelog+community). \
repo: GitHub slug e.g. 'tokio-rs/tokio' — anchors bug/release queries to GitHub issues/releases. \
version: e.g. '0.8' — defaults to 'latest'. \
Sources: GitHub Issues, Reddit, Hacker News, official changelogs.")]
async fn research_code(
    &self,
    Parameters(input): Parameters<CodeResearchInput>,
) -> String {
    use crate::researcher::crawler::crawl_all;
    use crate::researcher::summarizer::summarize_all;
    use crate::researcher::publisher::write_code_report;
    use crate::llm::client::LlmClient;

    let version = input.version.as_deref().unwrap_or("latest");
    let framework = &input.framework;
    let aspects = input.aspects.unwrap_or_else(|| {
        vec!["bugs".to_string(), "changelog".to_string(), "community".to_string()]
    });

    // Build query list from templates — no LLM planner call
    let mut queries: Vec<String> = Vec::new();
    for aspect in &aspects {
        match aspect.as_str() {
            "bugs" => {
                queries.push(format!("{framework} {version} bug issue"));
                if let Some(repo) = &input.repo {
                    queries.push(format!("{framework} {version} issue site:github.com/{repo}/issues"));
                }
            }
            "changelog" => {
                queries.push(format!("{framework} {version} changelog release notes"));
                queries.push(format!("{framework} {version} breaking changes"));
            }
            "community" => {
                queries.push(format!("{framework} {version} site:reddit.com"));
                queries.push(format!("{framework} {version} site:news.ycombinator.com"));
            }
            "releases" => {
                queries.push(format!("{framework} {version} release"));
                if let Some(repo) = &input.repo {
                    queries.push(format!("{framework} {version} site:github.com/{repo}/releases"));
                }
            }
            _ => {} // silently skip unknown aspects
        }
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let llm = LlmClient::new(&self.cfg);
    let topic = format!("{framework} {version}");

    let sources = crawl_all(&http, &self.cfg, &queries).await;
    if sources.is_empty() {
        return format!("Error: no sources found for {framework} {version}");
    }

    let summaries = summarize_all(&llm, &sources, &topic).await;
    if summaries.is_empty() {
        return format!("Error: could not summarize sources for {framework} {version}");
    }

    match write_code_report(&llm, &summaries, framework, version, &aspects).await {
        Ok(report) => report,
        Err(e)     => format!("Error writing report: {e:#}"),
    }
}
```

**Step 2: Verify it compiles**

```bash
cargo check 2>&1 | grep "^error"
```

Expected: no output.

**Step 3: Full release build**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished release profile [optimized] target(s) in ...`

**Step 4: Commit**

```bash
git add src/mcp_server.rs
git commit -m "feat: add research_code MCP tool for framework bugs/changelog/community research"
```

---

### Task 4: Test via MCP

**Step 1: Restart MCP server**

In Claude Code: `/mcp` → restart researcher (or restart the session).
The new binary must be loaded before calling the tool.

**Step 2: Quick smoke test — bugs only**

```json
{
  "framework": "axum",
  "version": "0.8",
  "aspects": ["bugs"],
  "repo": "tokio-rs/axum"
}
```

Expected: markdown report with `## Known Bugs & Issues` section, citations `[N]`, no game results.

**Step 3: Multi-aspect test**

```json
{
  "framework": "tokio",
  "aspects": ["changelog", "community"]
}
```

Expected: report with `## Changelog & Breaking Changes` and `## Community Sentiment` sections. No `## Known Bugs` section (not requested).

**Step 4: Default aspects test (no aspects field)**

```json
{ "framework": "claude code" }
```

Expected: report covering bugs + changelog + community (the three defaults).
