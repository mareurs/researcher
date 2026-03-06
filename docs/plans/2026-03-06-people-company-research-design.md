# People & Company Research Tools — Design

**Date:** 2026-03-06  
**Status:** Approved  

## Overview

Add two new MCP tools — `research_person` and `research_company` — for meeting prep and due diligence. Given a person or company name, each tool runs the existing research pipeline with target-specific source domains, LLM prompts, and report sections, returning a markdown brief.

## Tool Signatures

```
research_person(
    name:   String,                              // e.g. "Maria Ionescu"
    method: "company" | "personal" | "both"     // default: "both"
)

research_company(
    name:    String,              // e.g. "Acme Corp"
    country: Option<String>,      // optional geo-narrowing, e.g. "Romania"
)
```

## Source Sets

### research_person — "company" method
`linkedin.com, twitter.com, x.com, github.com, medium.com, scholar.google.com` + open news search

### research_person — "personal" method
`facebook.com, instagram.com, twitter.com, x.com, reddit.com, tiktok.com` + open search

### research_person — "both"
Union of professional + personal domain sets.

### research_company
`linkedin.com, crunchbase.com, bloomberg.com, glassdoor.com, trustpilot.com, wikipedia.org` + open news search

## Authentication (Option B)

Optional cookie-based auth per platform. When set, injected as `Cookie:` header on matching domains. No headless browser required — just authenticated HTTP via reqwest.

**Env vars:**
```
LINKEDIN_COOKIE=li_at=AQEDATk...
FB_COOKIE=c_user=123456; xs=abc...
INSTAGRAM_COOKIE=sessionid=abc...
TWITTER_COOKIE=auth_token=abc...
```

Falls back to unauthenticated scraping when cookies are absent.

## Report Sections

### research_person — "company"
1. **Identity** — current role, company, location, tenure
2. **Career Path** — previous roles, trajectory, expertise areas
3. **Public Voice** — articles, posts, talks, opinions they've shared
4. **Conversation Hooks** — recent wins, projects, things worth referencing
5. **How to Position Your Work** — what they likely care about given their role

### research_person — "personal"
1. **Interests & Hobbies** — sports, travel, food, culture (from public posts)
2. **Online Presence** — active platforms, posting style
3. **Personal Conversation Starters** — topics to build rapport

### research_person — "both"
All sections from both above, combined in one report.

### research_company
1. **What They Do** — product, market, business model
2. **Size & Stage** — headcount, funding, revenue signals
3. **Recent News** — launches, press, funding rounds
4. **Culture & Values** — glassdoor, about page, leadership tone
5. **Strategic Context** — what they're optimizing for, problems they're solving

## Code Changes

| File | Change |
|------|--------|
| `src/config.rs` | Add `AuthConfig` struct (4 optional cookie fields); embed in `Config` |
| `src/scraper/html.rs` | Add `extra_headers: Option<HeaderMap>` to `fetch_and_extract()` |
| `src/researcher/pipeline.rs` | Add `ResearchTarget` enum (`Topic`, `Person { method }`, `Company`); add to `ResearchRequest`; add default domain sets per target |
| `src/researcher/planner.rs` | Target-aware prompt in `generate_queries()` |
| `src/researcher/publisher.rs` | Target-aware prompt + section structure in `write_report()` |
| `src/mcp_server.rs` | Add `research_person` and `research_company` `#[tool]` methods |

## Design Decisions

- **No new dependencies** — reuses reqwest, the full existing pipeline (crawler, summarizer, embeddings, search fallback) unchanged.
- **No new binaries** — both tools added to the existing `researcher-mcp` binary.
- **Graceful degradation** — works without cookies; auth is purely additive.
- **Domain hint reuse** — source sets map directly to the existing `domains: Vec<String>` field on `ResearchRequest`; no new crawling mechanism.
