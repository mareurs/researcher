# People & Company Research Tools — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `research_person(name, method)` and `research_company(name, country?)` MCP tools to the existing `researcher-mcp` binary, with target-aware LLM prompts and optional cookie-based auth for gated social platforms.

**Architecture:** Reuse the full existing pipeline (planner → crawler → summarizer → publisher). Add a `ResearchTarget` enum to `ResearchRequest` to carry the target type through prompt generation and report writing. Add `AuthConfig` to `Config` for per-domain cookie injection in the scraper. No new dependencies, no new binaries.

**Tech Stack:** Rust, reqwest (HeaderMap already available), rmcp `#[tool]` macro, existing `LlmClient` / `ResearchRequest` / pipeline.

**Verify after every task:** `cargo check` — must produce zero errors before committing.

---

### Task 1: Add `AuthConfig` to `Config`

**Files:**
- Modify: `src/config.rs`
- Modify: `src/mcp_server.rs` (the `config_from_env` function at the bottom)

**Step 1: Add `AuthConfig` struct and embed it in `Config`**

In `src/config.rs`, after the existing imports, add the struct and a new field to `Config`.

Add this struct anywhere before `Config`:
```rust
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub linkedin_cookie: Option<String>,
    pub fb_cookie:       Option<String>,
    pub instagram_cookie: Option<String>,
    pub twitter_cookie:  Option<String>,
}

impl AuthConfig {
    /// Return the Cookie header value for the given hostname, if configured.
    pub fn cookie_for_host(&self, host: &str) -> Option<&str> {
        if host.contains("linkedin.com") {
            self.linkedin_cookie.as_deref()
        } else if host.contains("facebook.com") {
            self.fb_cookie.as_deref()
        } else if host.contains("instagram.com") {
            self.instagram_cookie.as_deref()
        } else if host.contains("twitter.com") || host.contains("x.com") {
            self.twitter_cookie.as_deref()
        } else {
            None
        }
    }
}
```

Add a field to the `Config` struct (anywhere, e.g. after `profiles`):
```rust
    pub auth: AuthConfig,
```

**Step 2: Populate `AuthConfig` in `config_from_env` in `src/mcp_server.rs`**

In the `config_from_env()` function, add to the `Config { ... }` literal:
```rust
        auth: config::AuthConfig {
            linkedin_cookie:  std::env::var("LINKEDIN_COOKIE").ok(),
            fb_cookie:        std::env::var("FB_COOKIE").ok(),
            instagram_cookie: std::env::var("INSTAGRAM_COOKIE").ok(),
            twitter_cookie:   std::env::var("TWITTER_COOKIE").ok(),
        },
```

Also do the same in `src/main.rs` wherever `Config` is constructed/defaulted (search for `AuthConfig` after adding it and fix any `..Default::default()` if needed — the derive handles it).

**Step 3: Verify**
```bash
cargo check
```
Expected: 0 errors. `Config` may have warnings about unused `auth` field — that is fine for now.

**Step 4: Commit**
```bash
git add src/config.rs src/mcp_server.rs
git commit -m "feat: add AuthConfig with per-platform cookie fields"
```

---

### Task 2: Thread cookie auth through the scraper

**Files:**
- Modify: `src/scraper/html.rs`
- Modify: `src/researcher/crawler.rs`

**Step 1: Add `cookie` parameter to `fetch_and_extract`**

Replace the current signature:
```rust
pub async fn fetch_and_extract(
    http: &Client,
    url: &str,
    max_chars: usize,
) -> Result<String>
```

With:
```rust
pub async fn fetch_and_extract(
    http: &Client,
    url: &str,
    max_chars: usize,
    cookie: Option<&str>,
) -> Result<String>
```

Inside the function body, the `http.get(url)` builder chain currently ends at `.send()`. Insert the cookie injection before `.send()`:

```rust
    let mut req = http
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (compatible; Researcher/0.1)")
        .timeout(std::time::Duration::from_secs(15));

    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }

    let resp = req.send().await?;
```

(Remove the old single-expression builder.)

**Step 2: Update `crawl_query` in `src/researcher/crawler.rs` to resolve the cookie per URL**

`crawl_query` already receives `cfg: &Config`. In the closure that calls `fetch_and_extract`, extract the cookie from `cfg.auth` before the async move:

Find the section that builds `futs` (around line 54-59). It currently does:
```rust
let futs = fresh.iter().map(|(url, title)| {
    let http = http.clone();
    let url = url.clone();
    let max_chars = cfg.max_page_chars;
    async move {
        match fetch_and_extract(&http, &url, max_chars).await {
```

Replace with:
```rust
let futs = fresh.iter().map(|(url, title)| {
    let http = http.clone();
    let url = url.clone();
    let max_chars = cfg.max_page_chars;
    // Resolve cookie for this URL's host before entering async move
    let cookie: Option<String> = url::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .and_then(|host| cfg.auth.cookie_for_host(&host).map(|c| c.to_string()));
    async move {
        match fetch_and_extract(&http, &url, max_chars, cookie.as_deref()).await {
```

**Step 3: Add `url` as a dependency if not already present**

Check `Cargo.toml` — if `url` crate is not listed, add it:
```toml
url = "2"
```

If the `url` crate is unavailable, use a simpler host extraction approach instead:
```rust
    let cookie: Option<String> = {
        let host = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
        cfg.auth.cookie_for_host(host).map(|c| c.to_string())
    };
```

**Step 4: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 5: Commit**
```bash
git add src/scraper/html.rs src/researcher/crawler.rs Cargo.toml
git commit -m "feat: inject per-domain auth cookies in scraper"
```

---

### Task 3: Add `ResearchTarget` to the pipeline

**Files:**
- Modify: `src/researcher/pipeline.rs`

**Step 1: Add `PersonMethod` and `ResearchTarget` enums**

After the existing `ResearchMode` enum (line ~22), add:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum PersonMethod {
    Company,
    Personal,
    Both,
}

impl std::str::FromStr for PersonMethod {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "company"  => Ok(PersonMethod::Company),
            "personal" => Ok(PersonMethod::Personal),
            _          => Ok(PersonMethod::Both),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResearchTarget {
    Topic,
    Person { method: PersonMethod },
    Company,
}

impl Default for ResearchTarget {
    fn default() -> Self { ResearchTarget::Topic }
}
```

**Step 2: Add `target` field to `ResearchRequest`**

In the `ResearchRequest` struct, add:
```rust
    pub target: ResearchTarget,
```

Update `ResearchRequest::new()` to set `target: ResearchTarget::default()`.

**Step 3: Add domain-set helpers**

After `impl ResearchRequest`, add a free function:

```rust
pub fn domains_for_target(target: &ResearchTarget) -> Vec<String> {
    match target {
        ResearchTarget::Topic => vec![],
        ResearchTarget::Person { method } => {
            let professional = vec![
                "linkedin.com", "twitter.com", "x.com", "github.com",
                "medium.com", "scholar.google.com",
            ];
            let personal = vec![
                "facebook.com", "instagram.com", "twitter.com", "x.com",
                "reddit.com", "tiktok.com",
            ];
            let domains = match method {
                PersonMethod::Company  => professional,
                PersonMethod::Personal => personal,
                PersonMethod::Both => {
                    let mut v = professional;
                    v.extend_from_slice(&personal);
                    v
                }
            };
            domains.into_iter().map(String::from).collect()
        }
        ResearchTarget::Company => vec![
            "linkedin.com", "crunchbase.com", "bloomberg.com",
            "glassdoor.com", "trustpilot.com", "wikipedia.org",
        ].into_iter().map(String::from).collect(),
    }
}
```

**Step 4: Use `domains_for_target` in `run()`**

In the `run()` function, after the `domains` resolution block (around line 67-72), add:

```rust
    // If no explicit domains provided and target is not Topic, use target defaults
    if domains.is_empty() {
        domains = domains_for_target(&request.target);
    }
```

**Step 5: Thread `target` into `generate_queries` and `write_report`**

In `run()`, find the `generate_queries(...)` call and add `&request.target` as a parameter (you'll update the function signature in Task 4).

Find the `write_report(...)` call and add `&request.target` as a parameter (you'll update in Task 5).

For now, just note the call sites — do the actual signature changes in Tasks 4 and 5.

**Step 6: Verify**
```bash
cargo check
```
Expected: errors about `generate_queries` / `write_report` mismatched args — that's fine, Tasks 4 and 5 fix those.

**Step 7: Commit**
```bash
git add src/researcher/pipeline.rs
git commit -m "feat: add ResearchTarget enum and domain-set helpers"
```

---

### Task 4: Make `generate_queries` target-aware

**Files:**
- Modify: `src/researcher/planner.rs`

**Step 1: Add `target` parameter to `generate_queries`**

New signature:
```rust
pub async fn generate_queries(
    llm: &LlmClient,
    topic: &str,
    max_queries: usize,
    domains: &[String],
    target: &crate::researcher::pipeline::ResearchTarget,
) -> Result<Vec<String>>
```

**Step 2: Add a target-aware system prompt**

Replace the fixed system prompt string with a match:

```rust
    use crate::researcher::pipeline::{ResearchTarget, PersonMethod};

    let system_prompt = match target {
        ResearchTarget::Person { method } => {
            let focus = match method {
                PersonMethod::Company  => "professional background, career, expertise, public work, and thought leadership",
                PersonMethod::Personal => "personal interests, hobbies, lifestyle, and online presence",
                PersonMethod::Both     => "both professional background and personal interests/hobbies",
            };
            format!(
                "You are a research planning assistant specializing in people research. \
                 Generate focused search queries to build a profile of a person — covering their {focus}. \
                 Use their name in every query. Be specific and targeted."
            )
        }
        ResearchTarget::Company => {
            "You are a research planning assistant specializing in company research. \
             Generate focused search queries covering: what the company does, its size and funding stage, \
             recent news and launches, culture and values, and strategic priorities. \
             Use the company name in every query.".to_string()
        }
        ResearchTarget::Topic => {
            "You are a research planning assistant. Your job is to decompose a research \
             topic into specific, focused search queries that together will provide \
             comprehensive coverage of the topic. Each query should target a different \
             angle or subtopic. Be specific and use natural language search terms.".to_string()
        }
    };
```

Replace the hardcoded `ChatMessage::system("You are a research planning assistant...")` with `ChatMessage::system(system_prompt)`.

**Step 3: Update the call site in `pipeline.rs`**

In `run()`, find:
```rust
    let queries = generate_queries(&llm, topic, max_queries, &domains).await?;
```
Change to:
```rust
    let queries = generate_queries(&llm, topic, max_queries, &domains, &request.target).await?;
```

**Step 4: Verify**
```bash
cargo check
```
Expected: 0 errors (or only write_report mismatch if Task 5 not done yet).

**Step 5: Commit**
```bash
git add src/researcher/planner.rs src/researcher/pipeline.rs
git commit -m "feat: target-aware planner prompts for person/company research"
```

---

### Task 5: Make `write_report` target-aware

**Files:**
- Modify: `src/researcher/publisher.rs`

**Step 1: Add `target` parameter to `write_report`**

New signature:
```rust
pub async fn write_report(
    llm: &LlmClient,
    topic: &str,
    summaries: &[SourceSummary],
    mode: &ResearchMode,
    target: &crate::researcher::pipeline::ResearchTarget,
    token_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String>
```

**Step 2: Add target-specific prompt variants**

The existing `match mode { ... }` builds the user prompt. Wrap it so that for non-Topic targets, we use a dedicated prompt instead:

```rust
    use crate::researcher::pipeline::{ResearchTarget, PersonMethod};

    let prompt = match target {
        ResearchTarget::Person { method } => {
            let sections = match method {
                PersonMethod::Company => "\
                    ## Identity\nCurrent role, company, location, tenure.\n\
                    ## Career Path\nPrevious roles, trajectory, expertise areas.\n\
                    ## Public Voice\nArticles, posts, talks, opinions they've shared publicly.\n\
                    ## Conversation Hooks\nRecent wins, projects, interesting things worth referencing in a meeting.\n\
                    ## How to Position Your Work\nWhat they likely care about given their role and background.",
                PersonMethod::Personal => "\
                    ## Interests & Hobbies\nSports, travel, food, culture — from public posts and profiles.\n\
                    ## Online Presence\nWhich platforms they're active on, posting style and tone.\n\
                    ## Personal Conversation Starters\nSpecific topics to build rapport and make a personal connection.",
                PersonMethod::Both => "\
                    ## Identity\nCurrent role, company, location, tenure.\n\
                    ## Career Path\nPrevious roles, trajectory, expertise areas.\n\
                    ## Public Voice\nArticles, posts, talks, opinions.\n\
                    ## Conversation Hooks\nRecent wins, projects, things worth referencing.\n\
                    ## How to Position Your Work\nWhat they care about given their role.\n\
                    ## Interests & Hobbies\nSports, travel, food, culture — from public profiles.\n\
                    ## Personal Conversation Starters\nTopics to build personal rapport.",
            };
            format!(
                "You are preparing a meeting-prep brief on a person named {topic}.\n\
                 Using the research below, write a concise markdown report with these exact sections:\n\n\
                 {sections}\n\n\
                 Be specific — include names, dates, companies, post topics. Avoid vague generalities.\n\
                 Cite sources inline with [N] notation.\n\n\
                 {sources_text}"
            )
        }
        ResearchTarget::Company => {
            format!(
                "You are preparing a meeting-prep brief on a company named {topic}.\n\
                 Using the research below, write a concise markdown report with these exact sections:\n\n\
                 ## What They Do\nProduct, market, business model in 2-3 sentences.\n\
                 ## Size & Stage\nHeadcount, funding rounds, revenue signals.\n\
                 ## Recent News\nLaunches, press mentions, funding, leadership changes.\n\
                 ## Culture & Values\nGlassdoor signals, about-page tone, leadership style.\n\
                 ## Strategic Context\nWhat they're optimizing for right now, key challenges, opportunities.\n\n\
                 Be specific — include numbers, dates, and named people. Cite sources with [N].\n\n\
                 {sources_text}"
            )
        }
        ResearchTarget::Topic => {
            // existing match mode { ... } logic unchanged
            match mode {
                ResearchMode::Summary => format!(/* existing Summary prompt */
                    "Write a concise bullet-point summary:\n\
                     - 5-8 key findings as bullet points\n\
                     - Each bullet: one concrete fact, number, or conclusion\n\
                     - Prioritize actionable facts, numbers, and dates\n\
                     - No introduction, no conclusion, no section headers\n\n\
                     Topic: {topic}\n\n{sources_text}"
                ),
                ResearchMode::Deep => format!(
                    "Write a thorough, detailed research report on: {topic}\n\
                     - Begin with an executive summary (2-3 paragraphs)\n\
                     - Cover all major angles with dedicated ## sections\n\
                     - Include specific facts, numbers, dates, and source references\n\
                     - Conclude with key takeaways and open questions\n\
                     - Use markdown formatting throughout\n\n\
                     {sources_text}"
                ),
                _ => format!(
                    "Research topic: {topic}\n\n\
                     You have gathered the following research from multiple sources:\n\n\
                     {sources_text}\n\n\
                     Write a comprehensive markdown research report on '{topic}' that:\n\
                     1. Starts with an executive summary\n\
                     2. Has clearly organized sections\n\
                     3. Synthesizes findings across sources\n\
                     4. Cites sources inline with [N] notation\n\
                     5. Ends with a 'Sources' section listing all URLs\n\
                     6. Includes a 'Key Takeaways' section\n\n\
                     Be thorough and analytical."
                ),
            }
        }
    };
```

Note: `sources_text` is built before this block (same as existing code) — keep that unchanged.

**Step 3: Update the call site in `pipeline.rs`**

In `run()`, find the `write_report(...)` call and add `&request.target`:
```rust
    let raw_report = write_report(
        &llm,
        topic,
        &summaries,
        &request.mode,
        &request.target,
        token_tx,
    ).await?;
```

**Step 4: Verify**
```bash
cargo check
```
Expected: 0 errors.

**Step 5: Commit**
```bash
git add src/researcher/publisher.rs src/researcher/pipeline.rs
git commit -m "feat: target-aware report sections for person/company research"
```

---

### Task 6: Add `research_person` and `research_company` MCP tools

**Files:**
- Modify: `src/mcp_server.rs`

**Step 1: Add input structs**

After the existing `ResearchInput` struct, add:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PersonResearchInput {
    #[schemars(description = "Full name of the person to research, e.g. 'Maria Ionescu'")]
    pub name: String,

    #[schemars(description = "Research focus: 'company' (professional background), 'personal' (interests/lifestyle), or 'both' (default)")]
    pub method: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompanyResearchInput {
    #[schemars(description = "Company name to research, e.g. 'Acme Corp'")]
    pub name: String,

    #[schemars(description = "Optional country to narrow results, e.g. 'Romania'")]
    pub country: Option<String>,
}
```

**Step 2: Add the two tool methods to `impl ResearcherServer`**

Inside the `#[tool_router]` impl block, after the existing `research` method, add:

```rust
    #[tool(description = "Research a person for meeting prep. Returns a markdown brief covering professional background, career, public voice, conversation hooks, and/or personal interests — depending on 'method'. Sources: LinkedIn, Twitter/X, GitHub, Facebook, Instagram, Reddit, and more.")]
    async fn research_person(
        &self,
        Parameters(input): Parameters<PersonResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{
            run, ResearchMode, ResearchRequest, ResearchTarget, PersonMethod,
        };

        let method: PersonMethod = input.method
            .as_deref()
            .unwrap_or("both")
            .parse()
            .unwrap_or(PersonMethod::Both);

        let request = ResearchRequest {
            topic: input.name,
            mode: ResearchMode::Report,
            domains: vec![], // populated from target in run()
            domain_profile: None,
            target: ResearchTarget::Person { method },
        };

        match run(
            &self.cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(description = "Research a company for meeting prep. Returns a markdown brief covering what they do, size & stage, recent news, culture, and strategic context. Sources: LinkedIn, Crunchbase, Bloomberg, Glassdoor, Trustpilot, Wikipedia, and news.")]
    async fn research_company(
        &self,
        Parameters(input): Parameters<CompanyResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{
            run, ResearchMode, ResearchRequest, ResearchTarget,
        };

        let topic = match &input.country {
            Some(c) => format!("{} {}", input.name, c),
            None    => input.name.clone(),
        };

        let request = ResearchRequest {
            topic,
            mode: ResearchMode::Report,
            domains: vec![],
            domain_profile: None,
            target: ResearchTarget::Company,
        };

        match run(
            &self.cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
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
git add src/mcp_server.rs
git commit -m "feat: add research_person and research_company MCP tools"
```

---

### Task 7: Update `.env.example` and `CLAUDE.md`

**Files:**
- Modify: `.env.example` (if it exists, otherwise skip)
- Modify: `CLAUDE.md`

**Step 1: Add cookie vars to `.env.example`**

```bash
# Optional: session cookies for authenticated scraping
# Export from browser DevTools → Application → Cookies
LINKEDIN_COOKIE=
FB_COOKIE=
INSTAGRAM_COOKIE=
TWITTER_COOKIE=
```

**Step 2: Add new env vars to `CLAUDE.md` env table**

Add four rows to the Env Vars Reference table:

| Variable | Default | Notes |
|----------|---------|-------|
| `LINKEDIN_COOKIE` | `` | Cookie header value for linkedin.com |
| `FB_COOKIE` | `` | Cookie header value for facebook.com |
| `INSTAGRAM_COOKIE` | `` | Cookie header value for instagram.com |
| `TWITTER_COOKIE` | `` | Cookie header value for twitter.com / x.com |

**Step 3: Commit**
```bash
git add .env.example CLAUDE.md
git commit -m "docs: add auth cookie env vars to env.example and CLAUDE.md"
```

---

## Manual Smoke Test

After all tasks are complete, test locally (requires LLM + SearXNG running):

```bash
# Test person research (company focus)
RUST_LOG=info cargo run --bin researcher -- \
  --query "Elon Musk" --output /tmp/person-test.md

# Verify research_company via binary is working by checking compile only:
cargo build --release --bin researcher-mcp
```

To test MCP tools directly, update your `.mcp.json` to point to the new binary and invoke from Claude Code:
```
research_person("Elon Musk", method: "company")
research_company("SpaceX", country: "USA")
```
