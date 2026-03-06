# Job Search Tool — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `search_jobs(query, mode)` MCP tool that fetches remote AI engineering jobs from Remotive, Adzuna, and SearXNG, scores them against a stored user profile, and optionally enriches top results with company briefs.

**Architecture:** New `src/jobs/` module (fetcher → scorer → publisher) that is completely additive — no changes to the existing research pipeline. Deep mode reuses `researcher::pipeline::run(ResearchTarget::Company)` for inline company briefs. Profile stored in `profiles.toml` under `[job-profile]`, parsed separately from the existing domain-profiles parser.

**Tech Stack:** Rust, reqwest (async HTTP), serde_json (Remotive/Adzuna JSON), existing `LlmClient::complete()`, existing `search_with_fallback()`, existing `researcher::pipeline::run()`, rmcp `#[tool]` macro.

**Verify after every task:** `cargo check` — must produce zero errors before committing.

---

### Task 1: Add `JobProfile` to config and `profiles.toml`

**Files:**
- Modify: `src/config.rs`
- Modify: `profiles.toml`
- Modify: `src/main.rs`
- Modify: `src/mcp_server.rs`

**Step 1: Read existing `load_profiles` and `Config` struct**

```
find_symbol("load_profiles", path="src/config.rs", include_body=true)
find_symbol("Config", path="src/config.rs")
```

**Step 2: Add `JobProfile` struct to `src/config.rs`**

Add before `load_profiles()`:

```rust
/// User profile for job search, loaded from the `[job-profile]` section of `profiles.toml`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct JobProfile {
    pub title: String,
    pub seniority: String,
    pub salary_floor: String,
    #[serde(default)]
    pub remote_only: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub preferred_company_size: String,
    #[serde(default)]
    pub avoid_industries: Vec<String>,
    #[serde(default)]
    pub about_me: String,
}
```

**Step 3: Add `load_job_profile()` to `src/config.rs`**

Add after the `JobProfile` struct. This parses `profiles.toml` as a raw TOML value, extracts `[job-profile]`, and deserializes it — without conflicting with the existing `load_profiles()` parser:

```rust
/// Load the `[job-profile]` section from `profiles.toml`.
/// Returns `None` if the file is missing, the section is absent, or it fails to parse.
pub fn load_job_profile() -> Option<JobProfile> {
    let content = std::fs::read_to_string("profiles.toml").ok()?;
    let table: toml::Table = toml::from_str(&content).ok()?;
    let section = table.get("job-profile")?;
    toml::Value::try_into(section.clone()).ok()
}
```

**Step 4: Add `job_profile` field to `Config`**

In the `Config` struct, after the `auth: AuthConfig` field, add:

```rust
    /// Job search profile loaded from profiles.toml. Not a CLI flag.
    #[clap(skip)]
    pub job_profile: Option<JobProfile>,
```

**Step 5: Populate `job_profile` in `main.rs`**

In `src/main.rs`, after `cfg.auth = ...`, add:
```rust
cfg.job_profile = config::load_job_profile();
```

**Step 6: Populate `job_profile` in `config_from_env` in `src/mcp_server.rs`**

In the `Config { ... }` literal inside `config_from_env()`, add:
```rust
        job_profile: config::load_job_profile(),
```

**Step 7: Add `[job-profile]` section to `profiles.toml`**

Append to the end of `profiles.toml`:

```toml

# ── Job search profile ────────────────────────────────────────────────────────
# Used by the search_jobs MCP tool. Edit to match your background.
[job-profile]
title = "Senior AI Engineer"
seniority = "senior"
salary_floor = "150000 USD"
remote_only = true
skills = ["Rust", "Python", "LLMs", "MLOps", "fine-tuning", "inference", "RAG", "transformers"]
preferred_company_size = "startup to mid-size"
avoid_industries = ["gambling", "crypto", "adult content"]
about_me = """
AI engineer with 5+ years building production ML systems. Specialised in LLM
inference optimisation, RAG pipelines, and Rust-based AI tooling. Looking for
technically deep roles with autonomy and meaningful AI impact.
"""
```

**Step 8: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 9: Commit**
```bash
git add src/config.rs src/main.rs src/mcp_server.rs profiles.toml
git commit -m "feat: add JobProfile config struct and job-profile section in profiles.toml"
```

---

### Task 2: Create `src/jobs/` module with `JobListing` and `fetch_jobs()`

**Files:**
- Create: `src/jobs/mod.rs`
- Create: `src/jobs/fetcher.rs`
- Modify: `src/main.rs` (add `mod jobs;`)
- Modify: `src/mcp_server.rs` (add `mod jobs;`)

**Step 1: Create `src/jobs/mod.rs`**

```rust
pub mod fetcher;
pub mod scorer;
pub mod publisher;
```

(scorer and publisher don't exist yet — that's fine, add them as empty stubs or leave the `pub mod` lines out until Tasks 3/4.)

Actually: create `mod.rs` with only the fetcher for now:
```rust
pub mod fetcher;
```

**Step 2: Create `src/jobs/fetcher.rs`**

```rust
use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::info;

use crate::config::{AuthConfig, Config, JobProfile};
use crate::search::search_with_fallback;

/// A single job listing from any source.
#[derive(Debug, Clone)]
pub struct JobListing {
    pub title: String,
    pub company: String,
    pub url: String,
    pub salary: Option<String>,
    pub description: String,
    pub source: String,
}

// ── Remotive ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RemotiveResponse {
    jobs: Vec<RemotiveJob>,
}

#[derive(Debug, Deserialize)]
struct RemotiveJob {
    title: String,
    company_name: String,
    url: String,
    #[serde(default)]
    salary: String,
    description: String,
}

async fn fetch_remotive(http: &Client, query: &str) -> Vec<JobListing> {
    let url = format!(
        "https://remotive.com/api/remote-jobs?search={}&limit=20",
        urlencoding::encode(query)
    );
    let Ok(resp) = http.get(&url).send().await else { return vec![] };
    let Ok(data) = resp.json::<RemotiveResponse>().await else { return vec![] };

    data.jobs.into_iter().map(|j| JobListing {
        title: j.title,
        company: j.company_name,
        url: j.url,
        salary: if j.salary.is_empty() { None } else { Some(j.salary) },
        description: truncate(&j.description, 400),
        source: "remotive".to_string(),
    }).collect()
}

// ── Adzuna ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AdzunaResponse {
    results: Vec<AdzunaJob>,
}

#[derive(Debug, Deserialize)]
struct AdzunaJob {
    title: String,
    company: AdzunaCompany,
    redirect_url: String,
    #[serde(default)]
    salary_min: Option<f64>,
    #[serde(default)]
    salary_max: Option<f64>,
    description: String,
}

#[derive(Debug, Deserialize)]
struct AdzunaCompany {
    display_name: String,
}

async fn fetch_adzuna(http: &Client, query: &str, app_id: &str, app_key: &str) -> Vec<JobListing> {
    let url = format!(
        "https://api.adzuna.com/v1/api/jobs/gb/search/1\
         ?app_id={app_id}&app_key={app_key}\
         &results_per_page=20\
         &what={}&where=remote\
         &content-type=application/json",
        urlencoding::encode(query)
    );
    let Ok(resp) = http.get(&url).send().await else { return vec![] };
    let Ok(data) = resp.json::<AdzunaResponse>().await else { return vec![] };

    data.results.into_iter().map(|j| JobListing {
        title: j.title,
        company: j.company.display_name,
        url: j.redirect_url,
        salary: match (j.salary_min, j.salary_max) {
            (Some(lo), Some(hi)) => Some(format!("${:.0}–${:.0}", lo, hi)),
            (Some(lo), None)     => Some(format!("${:.0}+", lo)),
            _                    => None,
        },
        description: truncate(&j.description, 400),
        source: "adzuna".to_string(),
    }).collect()
}

// ── SearXNG ───────────────────────────────────────────────────────────────────

async fn fetch_searxng(
    http: &Client,
    cfg: &Config,
    query: &str,
    profile: &JobProfile,
) -> Vec<JobListing> {
    let remote_clause = if profile.remote_only { " remote" } else { "" };
    let full_query = format!("{query}{remote_clause} job opening");

    let Ok(results) = search_with_fallback(
        http,
        &cfg.searxng_url,
        &full_query,
        cfg.search_results_per_query,
    ).await else { return vec![] };

    results.into_iter().map(|r| JobListing {
        title: r.title.clone(),
        company: extract_company_from_title(&r.title),
        url: r.url,
        salary: None,
        description: truncate(&r.snippet, 400),
        source: "searxng".to_string(),
    }).collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetch job listings from all configured sources, merge, and deduplicate by URL.
pub async fn fetch_jobs(
    http: &Client,
    cfg: &Config,
    query: &str,
    profile: &JobProfile,
) -> Vec<JobListing> {
    info!(%query, "fetching job listings");

    let (remotive, adzuna, searxng) = tokio::join!(
        fetch_remotive(http, query),
        async {
            match (
                std::env::var("ADZUNA_APP_ID").ok(),
                std::env::var("ADZUNA_APP_KEY").ok(),
            ) {
                (Some(id), Some(key)) => fetch_adzuna(http, query, &id, &key).await,
                _ => vec![],
            }
        },
        fetch_searxng(http, cfg, query, profile),
    );

    // Merge and deduplicate by URL
    let mut seen = std::collections::HashSet::new();
    let mut listings = Vec::new();
    for job in remotive.into_iter().chain(adzuna).chain(searxng) {
        if seen.insert(job.url.clone()) {
            listings.push(job);
        }
    }

    info!(count = listings.len(), "job listings fetched");
    listings
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    // Strip HTML tags crudely (descriptions from APIs often contain HTML)
    let clean: String = s.chars().fold(String::new(), |mut acc, c| {
        if c == '<' { acc.push(' '); }
        else if c != '>' { acc.push(c); }
        acc
    });
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.len() <= max { clean } else { format!("{}…", &clean[..max]) }
}

fn extract_company_from_title(title: &str) -> String {
    // "Senior AI Engineer at Acme Corp" → "Acme Corp"
    if let Some(pos) = title.to_lowercase().find(" at ") {
        title[pos + 4..].trim().to_string()
    } else {
        String::new()
    }
}
```

**Step 3: Add `urlencoding` dependency to `Cargo.toml`**

Check if `urlencoding` is already in `Cargo.toml`:
```bash
grep urlencoding Cargo.toml
```

If absent, add to `[dependencies]`:
```toml
urlencoding = "2"
```

**Step 4: Add `mod jobs;` to both binaries**

In `src/main.rs`, add alongside the other `mod` declarations:
```rust
mod jobs;
```

In `src/mcp_server.rs`, add the same line.

**Step 5: Verify**
```bash
cargo check
```
Expected: 0 errors. May warn about unused `fetch_jobs` — that is fine.

**Step 6: Commit**
```bash
git add src/jobs/ src/main.rs src/mcp_server.rs Cargo.toml Cargo.lock
git commit -m "feat: add jobs module with JobListing struct and multi-source fetcher"
```

---

### Task 3: Add job scorer

**Files:**
- Create: `src/jobs/scorer.rs`
- Modify: `src/jobs/mod.rs`

**Step 1: Create `src/jobs/scorer.rs`**

```rust
use anyhow::Result;
use serde::Deserialize;
use tracing::info;

use crate::config::JobProfile;
use crate::llm::client::{ChatMessage, LlmClient};
use super::fetcher::JobListing;

/// A job listing with an LLM-assigned match score and reason.
#[derive(Debug, Clone)]
pub struct ScoredJob {
    pub listing: JobListing,
    pub score: u8,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
struct ScoreEntry {
    id: usize,
    score: u8,
    reason: String,
}

/// Score all listings against the user profile in a single LLM call.
/// Returns listings with score >= threshold, sorted descending by score.
pub async fn score_listings(
    llm: &LlmClient,
    listings: &[JobListing],
    profile: &JobProfile,
    threshold: u8,
) -> Result<Vec<ScoredJob>> {
    if listings.is_empty() {
        return Ok(vec![]);
    }

    info!(count = listings.len(), "scoring job listings against profile");

    // Build numbered listing digest (title + company + salary + description snippet)
    let listings_text = listings.iter().enumerate().map(|(i, j)| {
        let salary = j.salary.as_deref().unwrap_or("not listed");
        format!(
            "{id}. {title} @ {company} | salary: {salary}\n   {desc}",
            id = i + 1,
            title = j.title,
            company = j.company,
            desc = j.description,
        )
    }).collect::<Vec<_>>().join("\n\n");

    let profile_text = format!(
        "Title: {}\nSeniority: {}\nSalary floor: {}\nRemote only: {}\n\
         Skills: {}\nPreferred company size: {}\nAvoid industries: {}\n\n\
         About me:\n{}",
        profile.title,
        profile.seniority,
        profile.salary_floor,
        profile.remote_only,
        profile.skills.join(", "),
        profile.preferred_company_size,
        profile.avoid_industries.join(", "),
        profile.about_me,
    );

    let prompt = format!(
        "You are a job-match evaluator. Score each job listing against this candidate profile.\n\n\
         ## Candidate Profile\n{profile_text}\n\n\
         ## Job Listings\n{listings_text}\n\n\
         Return a JSON array (no markdown, no explanation) with one object per listing:\n\
         [{{\"id\": 1, \"score\": 8, \"reason\": \"one line\"}}, ...]\n\
         Score 1-10. Score 1-5 = poor fit. Score 6-7 = decent fit. Score 8-10 = strong fit.\n\
         Penalise missing salary if salary_floor is set. Penalise industries in avoid_industries.\n\
         Use the reason field to explain the score in one concrete sentence."
    );

    let messages = vec![
        ChatMessage::system(
            "You are a precise job-match evaluator. Return only valid JSON arrays. \
             No markdown fences, no explanation outside the JSON."
        ),
        ChatMessage::user(prompt),
    ];

    let response = llm.complete(messages).await?;

    // Parse JSON — find the array even if the model adds surrounding text
    let json_start = response.find('[').unwrap_or(0);
    let json_end   = response.rfind(']').map(|i| i + 1).unwrap_or(response.len());
    let json_slice = &response[json_start..json_end];

    let scores: Vec<ScoreEntry> = serde_json::from_str(json_slice)
        .unwrap_or_default();

    // Build result: join scores back to listings by id (1-indexed)
    let mut result: Vec<ScoredJob> = scores.into_iter().filter_map(|s| {
        let idx = s.id.checked_sub(1)?;
        let listing = listings.get(idx)?.clone();
        if s.score >= threshold { Some(ScoredJob { listing, score: s.score, reason: s.reason }) }
        else { None }
    }).collect();

    result.sort_by(|a, b| b.score.cmp(&a.score));

    info!(kept = result.len(), "scoring complete");
    Ok(result)
}
```

**Step 2: Update `src/jobs/mod.rs`**

```rust
pub mod fetcher;
pub mod scorer;
```

**Step 3: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 4: Commit**
```bash
git add src/jobs/scorer.rs src/jobs/mod.rs
git commit -m "feat: add job scorer with single-LLM-call batch scoring"
```

---

### Task 4: Add job report publisher

**Files:**
- Create: `src/jobs/publisher.rs`
- Modify: `src/jobs/mod.rs`

**Step 1: Create `src/jobs/publisher.rs`**

```rust
use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};
use super::scorer::ScoredJob;

/// Render a job search report in list or deep mode.
/// Deep mode calls research_company for each top-N company in parallel.
pub async fn write_job_report(
    cfg: &Config,
    jobs: &[ScoredJob],
    query: &str,
    deep: bool,
) -> Result<String> {
    if jobs.is_empty() {
        return Ok(format!("# Job Search: \"{query}\"\n\nNo matching jobs found."));
    }

    info!(count = jobs.len(), deep, "writing job report");

    // --- Company briefs (deep mode) ---
    let company_briefs: Vec<Option<String>> = if deep {
        let top_n = jobs.len().min(5);
        let futs: Vec<_> = jobs[..top_n].iter().map(|j| {
            let company = j.listing.company.clone();
            let cfg = cfg.clone();
            async move {
                if company.is_empty() { return None; }
                let request = ResearchRequest {
                    topic: company,
                    mode: ResearchMode::Report,
                    domains: vec![],
                    domain_profile: None,
                    target: ResearchTarget::Company,
                };
                match run(&cfg, &request, |_| {}, None).await {
                    Ok(r) => Some(r.report),
                    Err(_) => None,
                }
            }
        }).collect();

        // Run all company research in parallel
        let mut briefs = Vec::new();
        for fut in futs {
            briefs.push(fut.await);
        }
        // Pad with None for jobs beyond top_n
        while briefs.len() < jobs.len() {
            briefs.push(None);
        }
        briefs
    } else {
        vec![None; jobs.len()]
    };

    // --- Markdown table ---
    let mut out = format!("# Job Search: \"{query}\" — {} matches\n\n", jobs.len());

    out.push_str("| # | Title | Company | Salary | Match | Apply |\n");
    out.push_str("|---|-------|---------|--------|-------|-------|\n");
    for (i, j) in jobs.iter().enumerate() {
        let salary = j.listing.salary.as_deref().unwrap_or("—");
        out.push_str(&format!(
            "| {} | {} | {} | {} | ⭐ {}/10 | [link]({}) |\n",
            i + 1, j.listing.title, j.listing.company, salary, j.score, j.listing.url
        ));
    }
    out.push('\n');

    // --- Per-job cards ---
    for (i, (job, brief)) in jobs.iter().zip(company_briefs.iter()).enumerate() {
        let salary = job.listing.salary.as_deref().unwrap_or("not listed");
        out.push_str(&format!(
            "## {}. {} — {} ⭐ {}/10\n\n\
             **Why it fits:** {}\n\
             **Salary:** {}\n\
             **Role:** {}\n\
             **Apply:** {}\n",
            i + 1,
            job.listing.title,
            job.listing.company,
            job.score,
            job.reason,
            salary,
            job.listing.description,
            job.listing.url,
        ));

        if let Some(b) = brief {
            out.push_str("\n### Company Brief\n\n");
            out.push_str(b);
        }

        out.push('\n');
    }

    Ok(out)
}
```

**Step 2: Update `src/jobs/mod.rs`**

```rust
pub mod fetcher;
pub mod scorer;
pub mod publisher;
```

**Step 3: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 4: Commit**
```bash
git add src/jobs/publisher.rs src/jobs/mod.rs
git commit -m "feat: add job report publisher with list and deep modes"
```

---

### Task 5: Add `search_jobs` MCP tool

**Files:**
- Modify: `src/mcp_server.rs`
- Modify: `src/config.rs` (add new cookie fields to `AuthConfig`)

**Step 1: Add `LINKEDIN_JOBS_COOKIE` and `INDEED_COOKIE` to `AuthConfig`**

In `src/config.rs`, update `AuthConfig`:
```rust
pub struct AuthConfig {
    pub linkedin_cookie:       Option<String>,
    pub fb_cookie:             Option<String>,
    pub instagram_cookie:      Option<String>,
    pub twitter_cookie:        Option<String>,
    pub linkedin_jobs_cookie:  Option<String>,
    pub indeed_cookie:         Option<String>,
}
```

Update `cookie_for_host` to add two new match arms:
```rust
} else if host.contains("linkedin.com") {
    // Use jobs-specific cookie if set, fall back to general linkedin cookie
    self.linkedin_jobs_cookie.as_deref().or(self.linkedin_cookie.as_deref())
```

Wait — this conflicts with the existing `linkedin.com` match. Instead, keep the existing arm and let jobs fetcher handle its own cookie separately, OR just reuse the same `linkedin_cookie` for jobs too. **Simplest approach:** reuse `linkedin_cookie` for LinkedIn Jobs (same session cookie works for both). Skip adding `linkedin_jobs_cookie` entirely — just use the existing `LINKEDIN_COOKIE`.

So actually **no changes to `AuthConfig`** — the existing cookies work.

**Step 2: Add `JobSearchInput` struct and `search_jobs` method to `src/mcp_server.rs`**

After the existing `CompanyResearchInput` struct, add:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JobSearchInput {
    #[schemars(description = "Job search query, e.g. 'LLM inference engineer' or 'AI research Rust'")]
    pub query: String,

    #[schemars(description = "Output mode: 'list' (ranked shortlist, default) or 'deep' (shortlist + company briefs for top 5)")]
    pub mode: Option<String>,
}
```

Inside the `#[tool_router]` impl block, after `research_company`, add:

```rust
    #[tool(description = "Search for remote AI engineering jobs matching your stored job profile (profiles.toml [job-profile]). Returns a ranked markdown report. mode='list' for a quick shortlist, mode='deep' for full inline company briefs on top 5 matches. Sources: Remotive, Adzuna (if ADZUNA_APP_ID/KEY set), SearXNG.")]
    async fn search_jobs(
        &self,
        Parameters(input): Parameters<JobSearchInput>,
    ) -> String {
        use crate::jobs::{fetcher::fetch_jobs, scorer::score_listings, publisher::write_job_report};
        use crate::llm::client::LlmClient;

        let profile = match &self.cfg.job_profile {
            Some(p) => p.clone(),
            None => return "Error: no [job-profile] section found in profiles.toml. \
                            Add one to enable job search.".to_string(),
        };

        let deep = input.mode.as_deref() == Some("deep");

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let llm = LlmClient::new(&self.cfg);

        let listings = fetch_jobs(&http, &self.cfg, &input.query, &profile).await;

        let scored = match score_listings(&llm, &listings, &profile, 6).await {
            Ok(s) => s,
            Err(e) => return format!("Error scoring listings: {e:#}"),
        };

        match write_job_report(&self.cfg, &scored, &input.query, deep).await {
            Ok(report) => report,
            Err(e) => format!("Error writing report: {e:#}"),
        }
    }
```

**Step 3: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 4: Build release binary**
```bash
cargo build --release --bin researcher-mcp
```
Expected: compiles cleanly.

**Step 5: Commit**
```bash
git add src/mcp_server.rs src/config.rs
git commit -m "feat: add search_jobs MCP tool"
```

---

### Task 6: Update `CLAUDE.md` and add env vars

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Add new env vars to the Env Vars Reference table in `CLAUDE.md`**

Add these rows:
```
| `ADZUNA_APP_ID`   | `` | Adzuna API app ID (free tier at developer.adzuna.com) |
| `ADZUNA_APP_KEY`  | `` | Adzuna API key |
```

(LinkedIn and Indeed cookies already documented from previous feature.)

**Step 2: Commit**
```bash
git add CLAUDE.md
git commit -m "docs: add Adzuna API env vars to CLAUDE.md"
```

---

## Manual Smoke Test

After all tasks complete:

```bash
# Verify both binaries compile
cargo build --release

# Test profile loading (quick sanity check)
RUST_LOG=info cargo run --bin researcher -- --query "test" 2>&1 | head -5

# Full MCP test via Claude Code — invoke with:
# search_jobs("LLM inference engineer", mode: "list")
# search_jobs("AI research Rust", mode: "deep")
```
