# Researcher Progressive Disclosure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a research report exceeds 4000 chars, save it to `/tmp/researcher/` and return a compact JSON (summary + TOC with line numbers + source count + file path) instead of dumping the full report inline.

**Architecture:** New `src/researcher/disclosure.rs` holds types and logic. `src/researcher/planner.rs` gains a pure heuristic classifier. `mcp_server.rs` adds a threshold gate after `run()` that calls `disclose()` when the report is large. `server_instructions` teaches agents how to navigate the saved file.

**Tech Stack:** Rust, `serde`/`serde_json`, `anyhow`, `tokio`, `LlmClient` (already in crate)

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src/researcher/disclosure.rs` | Create | Types, TOC extraction, slug/save, LLM summarize, `disclose()` |
| `src/researcher/mod.rs` | Modify | Add `pub mod disclosure` |
| `src/researcher/planner.rs` | Modify | Add `classify_disclosure()` heuristic |
| `src/mcp_server.rs` | Modify | `DisclosureHint` field on input, threshold gate, updated instructions |

---

### Task 1: Create `disclosure.rs` with types and TOC extraction

**Files:**
- Create: `src/researcher/disclosure.rs`
- Modify: `src/researcher/mod.rs`

- [ ] **Step 1: Write the failing test for `extract_toc`**

Add at the bottom of `src/researcher/disclosure.rs` (file doesn't exist yet — create it):

```rust
use crate::researcher::pipeline::SourceEntry;
use crate::llm::client::{ChatMessage, LlmClient};
use anyhow::Result;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DisclosureHint {
    pub summary_focus: String,
    pub include_sources: bool,
}

impl Default for DisclosureHint {
    fn default() -> Self {
        Self {
            summary_focus: "main findings and key points".into(),
            include_sources: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TocEntry {
    pub heading: String,
    pub line: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct DisclosedResult {
    pub summary: String,
    pub toc: Vec<TocEntry>,
    pub source_count: usize,
    pub queries_used: Vec<String>,
    pub report_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceEntry>>,
}

pub fn extract_toc(report: &str) -> Vec<TocEntry> {
    // placeholder — tests will drive implementation
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_toc_finds_headings() {
        let report = "# Title\n\nsome text\n\n## Section One\n\nmore text\n\n### Sub\n\n#### Skip\n\n## Section Two\n";
        let toc = extract_toc(report);
        assert_eq!(toc.len(), 4); // h1, h2, h3, h2 — h4 excluded
        assert_eq!(toc[0].heading, "# Title");
        assert_eq!(toc[0].line, 1);
        assert_eq!(toc[1].heading, "## Section One");
        assert_eq!(toc[1].line, 5);
        assert_eq!(toc[2].heading, "### Sub");
        assert_eq!(toc[2].line, 9);
        assert_eq!(toc[3].heading, "## Section Two");
        assert_eq!(toc[3].line, 13);
    }

    #[test]
    fn test_extract_toc_empty_report() {
        assert!(extract_toc("").is_empty());
    }

    #[test]
    fn test_extract_toc_no_headings() {
        assert!(extract_toc("just some plain text\nno headings here").is_empty());
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd /home/marius/work/claude/researcher
cargo test disclosure::tests::test_extract_toc_finds_headings 2>&1
```

Expected: FAIL — `assert_eq!(toc.len(), 4)` fails because `extract_toc` returns `vec![]`

- [ ] **Step 3: Implement `extract_toc`**

Replace the placeholder in `src/researcher/disclosure.rs`:

```rust
pub fn extract_toc(report: &str) -> Vec<TocEntry> {
    report
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line.trim_start();
            if !trimmed.starts_with('#') {
                return None;
            }
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level > 3 {
                return None;
            }
            Some(TocEntry { heading: line.to_string(), line: i + 1 })
        })
        .collect()
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /home/marius/work/claude/researcher
cargo test disclosure::tests 2>&1
```

Expected: 3 tests pass

- [ ] **Step 5: Add `pub mod disclosure` to `src/researcher/mod.rs`**

The file currently contains:
```
pub mod crawler;
pub mod pipeline;
pub mod planner;
pub mod publisher;
pub mod quality;
pub mod summarizer;
```

Add one line:
```
pub mod disclosure;
```

- [ ] **Step 6: Verify it compiles**

```bash
cd /home/marius/work/claude/researcher
cargo check 2>&1
```

Expected: no errors

- [ ] **Step 7: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/researcher/disclosure.rs src/researcher/mod.rs
git commit -m "feat(disclosure): types + TOC extraction"
```

---

### Task 2: Slug generation and file saving

**Files:**
- Modify: `src/researcher/disclosure.rs`

- [ ] **Step 1: Write failing tests for `make_slug` and `save_report`**

Add to the `tests` module in `src/researcher/disclosure.rs`:

```rust
    #[test]
    fn test_make_slug_basic() {
        assert_eq!(make_slug("Axum 0.8 breaking changes"), "axum-0-8-breaking-changes");
    }

    #[test]
    fn test_make_slug_truncates_at_40() {
        let long = "this is a very long query that exceeds forty characters easily";
        assert!(make_slug(long).len() <= 40);
    }

    #[test]
    fn test_make_slug_collapses_separators() {
        assert_eq!(make_slug("hello -- world  !!"), "hello-world");
    }

    #[test]
    fn test_save_report_creates_file() {
        let content = "# Test\n\nsome content";
        let path = save_report(content, "test save query").unwrap();
        assert!(std::path::Path::new(&path).exists());
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, content);
        std::fs::remove_file(&path).ok();
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /home/marius/work/claude/researcher
cargo test disclosure::tests::test_make_slug 2>&1
```

Expected: compile error — `make_slug` not defined

- [ ] **Step 3: Implement `make_slug` and `save_report`**

Add to `src/researcher/disclosure.rs` (before the `#[cfg(test)]` block):

```rust
pub fn make_slug(query: &str) -> String {
    let slug: String = query
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // collapse multiple dashes and trim edges
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    let joined = parts.join("-");
    joined.chars().take(40).collect()
}

pub fn save_report(report: &str, query: &str) -> Result<String> {
    let dir = std::path::Path::new("/tmp/researcher");
    std::fs::create_dir_all(dir)?;
    let slug = make_slug(query);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = dir.join(format!("{slug}-{ts}.md"));
    std::fs::write(&path, report)?;
    Ok(path.to_string_lossy().into_owned())
}
```

- [ ] **Step 4: Run tests**

```bash
cd /home/marius/work/claude/researcher
cargo test disclosure::tests 2>&1
```

Expected: all 7 tests pass

- [ ] **Step 5: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/researcher/disclosure.rs
git commit -m "feat(disclosure): slug generation + file saving"
```

---

### Task 3: Query classifier in `planner.rs`

**Files:**
- Modify: `src/researcher/planner.rs`

- [ ] **Step 1: Write failing tests**

Add to the bottom of `src/researcher/planner.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::researcher::disclosure::DisclosureHint;

    #[test]
    fn test_classify_fact_query() {
        let h = classify_disclosure("what is the capital of France", None);
        assert_eq!(h.summary_focus, "key facts and direct answer");
        assert!(!h.include_sources);
    }

    #[test]
    fn test_classify_howto_query() {
        let h = classify_disclosure("how to implement OAuth2 in Rust", None);
        assert_eq!(h.summary_focus, "what solves the problem and how to implement it");
        assert!(h.include_sources);
    }

    #[test]
    fn test_classify_news_query() {
        let h = classify_disclosure("OpenAI latest announcements 2026", None);
        assert_eq!(h.summary_focus, "what happened and the key outcome");
        assert!(h.include_sources);
    }

    #[test]
    fn test_classify_general_query() {
        let h = classify_disclosure("transformer attention mechanisms", None);
        assert_eq!(h.summary_focus, "main findings and key points");
        assert!(h.include_sources);
    }

    #[test]
    fn test_classify_intent_overrides_topic() {
        // intent="developer-docs" → howto classification
        let h = classify_disclosure("axum middleware", Some("developer-docs"));
        assert_eq!(h.summary_focus, "what solves the problem and how to implement it");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /home/marius/work/claude/researcher
cargo test planner::tests 2>&1
```

Expected: compile error — `classify_disclosure` not defined

- [ ] **Step 3: Implement `classify_disclosure` in `src/researcher/planner.rs`**

Add this import at the top of `src/researcher/planner.rs`:

```rust
use crate::researcher::disclosure::DisclosureHint;
```

Add this function before the `#[cfg(test)]` block (or at end of file if no test block yet):

```rust
/// Heuristic classification of a query for disclosure summarization.
/// No LLM call — pure keyword matching on topic + intent.
pub fn classify_disclosure(topic: &str, intent: Option<&str>) -> DisclosureHint {
    let text = format!("{} {}", topic, intent.unwrap_or("")).to_lowercase();

    let is_fact = ["what is ", "what are ", "when did ", "who is ", "where is ",
                   "how many ", "define ", "definition of"]
        .iter().any(|p| text.contains(p));

    let is_howto = ["how to ", "implement", "build ", "create ", "setup ",
                    "configure", "install ", "integrate", "migrate", "developer-docs"]
        .iter().any(|p| text.contains(p));

    let is_news = ["news", "latest", "recent", "2025", "2026", "today",
                   "yesterday", "last week", "announced", "release"]
        .iter().any(|p| text.contains(p));

    if is_fact {
        DisclosureHint {
            summary_focus: "key facts and direct answer".into(),
            include_sources: false,
        }
    } else if is_howto {
        DisclosureHint {
            summary_focus: "what solves the problem and how to implement it".into(),
            include_sources: true,
        }
    } else if is_news {
        DisclosureHint {
            summary_focus: "what happened and the key outcome".into(),
            include_sources: true,
        }
    } else {
        DisclosureHint {
            summary_focus: "main findings and key points".into(),
            include_sources: true,
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cd /home/marius/work/claude/researcher
cargo test planner::tests 2>&1
```

Expected: all 5 tests pass

- [ ] **Step 5: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/researcher/planner.rs
git commit -m "feat(disclosure): query classifier in planner"
```

---

### Task 4: LLM summarization and `disclose()` entry point

**Files:**
- Modify: `src/researcher/disclosure.rs`

- [ ] **Step 1: Add `summarize_report` and `disclose`**

Add to `src/researcher/disclosure.rs` (before `#[cfg(test)]`):

```rust
async fn summarize_report(report: &str, hint: &DisclosureHint, llm: &LlmClient) -> Result<String> {
    let messages = vec![
        ChatMessage::system(
            "You are a research summarizer. Write a focused, direct summary in 3-5 sentences. \
             No bullet points. No section headings. No caveats unless they are critical blockers.",
        ),
        ChatMessage::user(format!(
            "Summarize this research focused on: {}\n\n{}",
            hint.summary_focus,
            // cap input to avoid blowing the context
            &report[..report.len().min(8_000)],
        )),
    ];
    llm.complete(messages).await
}

pub async fn disclose(
    report: &str,
    hint: &DisclosureHint,
    llm: &LlmClient,
    query: &str,
    sources: &[SourceEntry],
    queries_used: &[String],
) -> Result<DisclosedResult> {
    let report_path = save_report(report, query)?;
    let toc = extract_toc(report);
    let summary = summarize_report(report, hint, llm).await?;
    let sources_out = if hint.include_sources {
        Some(sources.to_vec())
    } else {
        None
    };
    Ok(DisclosedResult {
        summary,
        toc,
        source_count: sources.len(),
        queries_used: queries_used.to_vec(),
        report_path,
        sources: sources_out,
    })
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd /home/marius/work/claude/researcher
cargo check 2>&1
```

Expected: no errors

- [ ] **Step 3: Run all disclosure tests**

```bash
cd /home/marius/work/claude/researcher
cargo test disclosure 2>&1
```

Expected: all 7 tests pass (no new tests for async fns — those are integration-tested end-to-end)

- [ ] **Step 4: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/researcher/disclosure.rs
git commit -m "feat(disclosure): LLM summarization + disclose() entry point"
```

---

### Task 5: Wire disclosure into `mcp_server.rs`

**Files:**
- Modify: `src/mcp_server.rs`

- [ ] **Step 1: Add `disclosure_hint` to `ResearchInput`**

`ResearchInput` is defined at lines 44–65 in `src/mcp_server.rs`. Add the new field after `intent`:

```rust
// existing last field:
#[schemars(description = "Query intent: developer-docs | news | product-research | academic | general (default). Tunes planner query style. Orthogonal to domain_profile/domains.")]
pub intent: Option<String>,

// ADD THIS:
#[schemars(description = "Optional hint for the disclosure summary. When absent, the planner classifies automatically. Pass to control focus: e.g. {\"summary_focus\": \"what solves the problem and how\", \"include_sources\": true}")]
pub disclosure_hint: Option<crate::researcher::disclosure::DisclosureHint>,
```

- [ ] **Step 2: Add the threshold constant near the top of `mcp_server.rs`**

Add after the `use` imports (before the `struct` definitions, around line 40):

```rust
const DISCLOSURE_THRESHOLD: usize = 4_000;
```

- [ ] **Step 3: Replace the `research()` body with the gated version**

The current `research()` body (lines 142–178) ends with:

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

Replace the entire `research()` body with:

```rust
    async fn research(
        &self,
        Parameters(input): Parameters<ResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};
        use crate::researcher::disclosure::disclose;
        use crate::researcher::planner::classify_disclosure;
        use crate::llm::client::LlmClient;

        let mode = match input.mode.as_deref().unwrap_or("report") {
            "quick"   => ResearchMode::Quick,
            "summary" => ResearchMode::Summary,
            "deep"    => ResearchMode::Deep,
            _         => ResearchMode::Report,
        };

        // Clone what we need before input is partially moved into ResearchRequest
        let query = input.query.clone();
        let intent = input.intent.clone();
        let caller_hint = input.disclosure_hint.clone();

        let mut cfg = (*self.cfg).clone();
        if let Some(mq) = input.max_queries   { cfg.max_search_queries    = mq; }
        if let Some(ms) = input.max_sources   { cfg.max_sources_per_query = ms; }

        let request = ResearchRequest {
            topic: input.query,
            mode,
            domains: input.domains.unwrap_or_default(),
            domain_profile: input.domain_profile,
            target: ResearchTarget::default(),
            intent: input.intent,
        };

        match run(&cfg, &request, |ev| eprintln!("[researcher] {ev}"), None).await {
            Ok(r) => {
                let report = r.report.as_deref().unwrap_or("");
                if report.len() > DISCLOSURE_THRESHOLD {
                    let hint = caller_hint
                        .unwrap_or_else(|| classify_disclosure(&query, intent.as_deref()));
                    let llm = LlmClient::new(&cfg);
                    match disclose(report, &hint, &llm, &query, &r.sources, &r.queries).await {
                        Ok(d) => serde_json::to_string_pretty(&d)
                            .unwrap_or_else(|e| e.to_string()),
                        Err(e) => {
                            // Fallback to inline on disclosure failure — don't lose the result
                            eprintln!("[researcher] disclosure failed, returning inline: {e:#}");
                            serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                        }
                    }
                } else {
                    serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                }
            }
            Err(e) => format!("Error: {e:#}"),
        }
    }
```

- [ ] **Step 4: Verify it compiles**

```bash
cd /home/marius/work/claude/researcher
cargo check 2>&1
```

Expected: no errors

- [ ] **Step 5: Run all tests**

```bash
cd /home/marius/work/claude/researcher
cargo test 2>&1
```

Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/mcp_server.rs
git commit -m "feat(disclosure): threshold gate + disclosure_hint in research()"
```

---

### Task 6: Update `server_instructions` in `get_info()`

**Files:**
- Modify: `src/mcp_server.rs`

- [ ] **Step 1: Append navigation block to the `with_instructions` string**

In `get_info()` (around line 417), the `with_instructions(...)` call ends with the last example line:

```rust
             research(\"attention mechanism transformers\",\n\
               intent=\\\"academic\\\", domain_profile=\\\"academic\\\", mode=\\\"report\\\")\n\
             \".to_string(),
```

Replace that closing section with:

```rust
             research(\"attention mechanism transformers\",\n\
               intent=\\\"academic\\\", domain_profile=\\\"academic\\\", mode=\\\"report\\\")\n\
             \n\
             ── Research Results Navigation ────────────────────────────────\n\
             \n\
             When a result contains `report_path`, the full report is saved on disk.\n\
             Read `summary` first — if it answers your question, skip the file entirely.\n\
             Only drill down when you need details the summary omits.\n\
             \n\
             Navigation:\n\
               Full report:      read_file(\"<report_path>\")\n\
               Targeted section: run_command(\"sed -n '<line>,<end>p' <report_path>\")\n\
               Search within:    run_command(\"grep -n '<keyword>' <report_path>\")\n\
               Sources:          listed inline in `sources[]` when present\n\
             \n\
             For fact queries and quick lookups where summary suffices — do not open the file.\n\
             \".to_string(),
```

- [ ] **Step 2: Verify it compiles**

```bash
cd /home/marius/work/claude/researcher
cargo check 2>&1
```

Expected: no errors

- [ ] **Step 3: Run all tests**

```bash
cd /home/marius/work/claude/researcher
cargo test 2>&1
```

Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
cd /home/marius/work/claude/researcher
git add src/mcp_server.rs
git commit -m "feat(disclosure): navigation guidance in server_instructions"
```

---

## Self-Review

**Spec coverage:**
- ✅ Threshold gate (4000 chars) — Task 5
- ✅ `DisclosureHint` with `summary_focus` + `include_sources` — Task 1
- ✅ Caller-override via `disclosure_hint` on `ResearchInput` — Task 5
- ✅ Planner-derived default via `classify_disclosure()` — Task 3
- ✅ Save to `/tmp/researcher/<slug>-<ts>.md` — Task 2
- ✅ TOC extraction with line numbers, h1–h3 only — Task 1
- ✅ Focused LLM summary using `summary_focus` as lens — Task 4
- ✅ `sources` omitted when `include_sources: false` — Task 4
- ✅ `server_instructions` navigation block — Task 6
- ✅ Inline fallback when disclosure fails — Task 5

**Type consistency check:**
- `DisclosureHint` defined in Task 1, used in Tasks 3, 4, 5 — consistent
- `TocEntry { heading: String, line: usize }` — used in `extract_toc()` return and `DisclosedResult.toc` — consistent
- `disclose(report, hint, llm, query, sources, queries_used)` — defined in Task 4, called in Task 5 — consistent
- `classify_disclosure(topic, intent)` — defined in Task 3, called in Task 5 — consistent
- `save_report(report, query)` — defined in Task 2, called in Task 4 — consistent

**No placeholders found.**
