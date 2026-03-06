# Job Search Tool — Design

**Date:** 2026-03-06  
**Status:** Approved  

## Overview

Add a `search_jobs(query, mode)` MCP tool that finds remote AI engineering jobs matching a stored user profile, scores them by fit, and optionally enriches each result with a full company brief (reusing the existing `research_company` pipeline).

## Tool Signature

```
search_jobs(
    query:  String,              // e.g. "LLM inference optimization"
    mode:   "list" | "deep"      // default: "list"
)
```

## User Profile

Stored in `profiles.toml` under `[job-profile]`, read at call time:

```toml
[job-profile]
title = "Senior AI Engineer"
seniority = "senior"
salary_floor = "150000 USD"
remote_only = true
skills = ["Rust", "Python", "LLMs", "MLOps", "fine-tuning", "inference", "RAG"]
preferred_company_size = "startup to mid-size"
avoid_industries = ["gambling", "crypto"]
about_me = """
5+ years building production ML systems. Specialised in LLM inference,
RAG pipelines, and Rust-based AI tooling. Looking for technically deep
roles with autonomy and meaningful AI impact.
"""
```

Hard filters (always applied): `remote_only`, `avoid_industries`.

## Job Sources

Three tiers, run in parallel, results merged and deduplicated by URL:

**Tier 1 — Structured APIs:**
- **Remotive** — free JSON feed (`remotive.com/api/remote-jobs`), clean structured data
- **Adzuna** — free API (`ADZUNA_APP_ID` + `ADZUNA_APP_KEY` env vars), salary data included

**Tier 2 — SearXNG (existing engine):**
- Queries like `"AI engineer" "remote" site:linkedin.com/jobs`, `"LLM engineer" remote job`
- Catches boards not in Tier 1 (Greenhouse, Lever, company career pages)

**Tier 3 — Authenticated scraping (optional, cookie pattern from research_person):**
- `LINKEDIN_JOBS_COOKIE` — LinkedIn Jobs search
- `INDEED_COOKIE` — Indeed job search

Falls back gracefully: if Adzuna keys absent, uses SearXNG only.

## Matching & Scoring

Single LLM call over all fetched listings:

**Input:** full profile + all listings as a numbered list (title, company, salary, 2-line description)

**Output:** JSON array scored 1–10 with one-line reason per listing:
```json
[
  { "id": 1, "score": 9, "reason": "Rust + LLM inference focus, senior IC, $180k" },
  { "id": 2, "score": 4, "reason": "Python ML but CV-focused, no salary listed" }
]
```

Listings below threshold (default: score ≥ 6) are dropped. Results re-sorted by score descending.

## Output Format

### `list` mode

```markdown
# Job Search: "LLM inference engineer" — 8 matches

| # | Title | Company | Salary | Match | Apply |
|---|-------|---------|--------|-------|-------|
| 1 | Staff AI Engineer | Mistral AI | $180k | ⭐ 9/10 | [link] |

## 1. Staff AI Engineer — Mistral AI ⭐ 9/10
**Why it fits:** Rust + LLM inference focus, senior IC, $180k, fully remote
**Role:** 2-3 sentence summary
**Apply:** https://...
```

### `deep` mode

Same as `list` but each card includes an inline `research_company` brief:

```markdown
## 1. Staff AI Engineer — Mistral AI ⭐ 9/10
**Why it fits:** ...
**Role:** ...

### Company: Mistral AI
**What They Do:** ...
**Size & Stage:** Series B, ~150 people
**Recent News:** ...
**Apply:** https://...
```

`deep` mode runs `research_company` for top-N companies (default: 5) in parallel using the existing `researcher::pipeline::run(ResearchTarget::Company)`.

## Code Structure

New `src/jobs/` module — completely additive, no changes to existing pipeline:

| File | Purpose |
|------|---------|
| `src/jobs/mod.rs` | Module declarations |
| `src/jobs/fetcher.rs` | `JobListing` struct + `fetch_jobs()` — Remotive, Adzuna, SearXNG in parallel |
| `src/jobs/scorer.rs` | `score_listings()` — single LLM call, returns `Vec<ScoredJob>` |
| `src/jobs/publisher.rs` | `write_job_report()` — renders list/deep markdown |

**Config changes:**
- `JobProfile` struct in `src/config.rs`, parsed from `profiles.toml` `[job-profile]` section
- 4 new env vars: `ADZUNA_APP_ID`, `ADZUNA_APP_KEY`, `LINKEDIN_JOBS_COOKIE`, `INDEED_COOKIE`

**MCP changes (`src/mcp_server.rs`):**
- `JobSearchInput` struct
- `search_jobs` `#[tool]` method

## Design Decisions

- **No pipeline reuse for fetching** — job boards return many structured listings per page, not articles. A dedicated fetcher with `JobListing` structs is cleaner than forcing the article-scraper pipeline to handle them.
- **Single scoring LLM call** — batch scoring is cheap and avoids one LLM call per listing.
- **`deep` mode reuses `research_company`** — free enrichment, no extra code.
- **Completely additive** — zero changes to existing `research`, `research_person`, `research_company` tools.
