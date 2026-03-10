# Market Insight Tool — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `market_insight` MCP tool covering stocks, crypto, and macro — powered by web research, using the existing `ResearchTarget` pipeline pattern.

**Architecture:** New `AssetClass` enum and `ResearchTarget::Market { asset_class }` variant in `pipeline.rs`. Planner and publisher each get a `Market` branch with domain-specific prompts. MCP server gets a new `MarketInsightInput` struct and `market_insight` tool method — identical in structure to `research_company`.

**Tech Stack:** Rust, rmcp 1.1, regex crate (already a dependency), existing pipeline (`run()` in `researcher/pipeline.rs`).

**Spec:** `docs/superpowers/specs/2026-03-10-market-insight-design.md`

**Verification:** No test suite — use `cargo check` after each task to catch type errors, `cargo build --release` at the end.

---

## Chunk 1: Pipeline & Planner

### Task 1: Add `AssetClass` + `ResearchTarget::Market` to `pipeline.rs`

**Files:**
- Modify: `src/researcher/pipeline.rs:27-108`

The `AssetClass` enum goes right after `PersonMethod` (line 31). The `Market` variant is added to `ResearchTarget`. The `domains_for_target` function gets a new arm.

- [ ] **Step 1: Add `AssetClass` enum after `PersonMethod` (line 42, after the `impl FromStr for PersonMethod` block)**

Use `insert_code` after the `impl std::str::FromStr for PersonMethod` symbol:

```rust
#[derive(Debug, Clone, Default)]
pub enum AssetClass {
    Stock,
    Crypto,
    #[default]
    Macro,
}

impl std::str::FromStr for AssetClass {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "stock"  => Ok(AssetClass::Stock),
            "crypto" => Ok(AssetClass::Crypto),
            _        => Ok(AssetClass::Macro),
        }
    }
}
```

- [ ] **Step 2: Add `Market` variant to `ResearchTarget`**

Replace the `ResearchTarget` enum body:

```rust
pub enum ResearchTarget {
    #[default]
    Topic,
    Person { method: PersonMethod },
    Company,
    Market { asset_class: AssetClass },
}
```

- [ ] **Step 3: Add `Market` arm to `domains_for_target`**

Replace the `domains_for_target` function body (the closing `}` of the `match` needs the new arm before it). The full new body:

```rust
pub fn domains_for_target(target: &ResearchTarget) -> Vec<String> {
    match target {
        ResearchTarget::Topic => vec![],
        ResearchTarget::Person { method } => {
            let professional: &[&str] = &[
                "github.com", "wikipedia.org", "medium.com",
                "news.ycombinator.com", "youtube.com",
            ];
            let personal: &[&str] = &[
                "reddit.com", "youtube.com", "wikipedia.org",
            ];
            let domains: Vec<&str> = match method {
                PersonMethod::Company  => professional.to_vec(),
                PersonMethod::Personal => personal.to_vec(),
                PersonMethod::Both => {
                    let mut v = professional.to_vec();
                    for d in personal {
                        if !v.contains(d) { v.push(d); }
                    }
                    v
                }
            };
            domains.into_iter().map(String::from).collect()
        }
        ResearchTarget::Company => vec![
            "wikipedia.org", "techcrunch.com", "crunchbase.com",
            "trustpilot.com", "reddit.com",
        ].into_iter().map(String::from).collect(),
        ResearchTarget::Market { asset_class } => match asset_class {
            AssetClass::Stock => vec![
                "reuters.com", "ft.com", "seekingalpha.com", "marketwatch.com",
                "investopedia.com", "finance.yahoo.com", "fool.com", "cnbc.com",
            ],
            AssetClass::Crypto => vec![
                "coindesk.com", "cointelegraph.com", "decrypt.co", "theblock.co",
                "bitcoinmagazine.com", "cryptoslate.com", "reddit.com",
            ],
            AssetClass::Macro => vec![
                "reuters.com", "ft.com", "bloomberg.com", "cnbc.com",
                "wsj.com", "economist.com", "marketwatch.com",
            ],
        }.into_iter().map(String::from).collect(),
    }
}
```

- [ ] **Step 4: Run `cargo check`**

```bash
cargo check 2>&1 | head -30
```

Expected: errors only in `planner.rs` and `publisher.rs` — non-exhaustive match on `ResearchTarget` (correct — those are next tasks). No errors in `pipeline.rs` itself.

- [ ] **Step 5: Commit**

```bash
git add src/researcher/pipeline.rs
git commit -m "feat: add AssetClass enum and ResearchTarget::Market variant"
```

---

### Task 2: Add market planner prompt + ticker expansion to `planner.rs`

**Files:**
- Modify: `src/researcher/planner.rs`

Two changes: (1) add `expand_ticker()` helper, (2) add `Market` branch to the `system_prompt` match and update `domain_instruction` soft-hint check.

- [ ] **Step 1: Add `expand_ticker` function**

Insert before `generate_queries` (use `insert_code` before the `generate_queries` symbol, position `"before"`):

```rust
/// Expands well-known ticker symbols to their full name for better search quality.
/// E.g. "BTC" → "Bitcoin", "NVDA" → "Nvidia".
fn expand_ticker(topic: &str) -> String {
    const TICKERS: &[(&str, &str)] = &[
        ("BTC",   "Bitcoin"),
        ("ETH",   "Ethereum"),
        ("SOL",   "Solana"),
        ("BNB",   "Binance Coin"),
        ("XRP",   "Ripple XRP"),
        ("ADA",   "Cardano"),
        ("DOGE",  "Dogecoin"),
        ("AVAX",  "Avalanche"),
        ("NVDA",  "Nvidia"),
        ("AAPL",  "Apple"),
        ("MSFT",  "Microsoft"),
        ("GOOGL", "Google Alphabet"),
        ("META",  "Meta Platforms"),
        ("AMZN",  "Amazon"),
        ("TSLA",  "Tesla"),
        ("AMD",   "AMD Advanced Micro Devices"),
        ("INTC",  "Intel"),
        ("SPY",   "S&P 500 ETF"),
        ("QQQ",   "Nasdaq 100 ETF"),
    ];
    let mut result = topic.to_string();
    for (ticker, name) in TICKERS {
        if result.contains(name) {
            continue;
        }
        // Match ticker as whole word, with optional leading $
        let pattern = format!(r"\$?(?i)\b{ticker}\b");
        if let Ok(re) = regex::Regex::new(&pattern) {
            result = re.replace_all(&result, *name).into_owned();
        }
    }
    result
}
```

- [ ] **Step 2: Add `Market` branch to `system_prompt` match in `generate_queries`**

Replace the `system_prompt` match expression. The full new match:

```rust
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
             Use their name in every query. Be specific and targeted.\n\
             IMPORTANT: Write queries as plain search terms only — no dashes, no hyphens between concepts, \
             no possessives with apostrophes, no special operators. \
             Good: 'Andrej Karpathy OpenAI career history' \
             Bad: 'Andrej Karpathy - OpenAI role' or 'Karpathy\\'s projects'"
        )
    }
    ResearchTarget::Company => {
        "You are a research planning assistant specializing in company research. \
         Generate focused search queries covering: what the company does, its size and funding stage, \
         recent news and launches, culture and values, and strategic priorities. \
         Use the company name in every query. \
         Write queries as plain search terms — no dashes between concepts, no special operators.".to_string()
    }
    ResearchTarget::Topic => {
        "You are a research planning assistant. Your job is to decompose a research \
         topic into specific, focused search queries that together will provide \
         comprehensive coverage of the topic. Each query should target a different \
         angle or subtopic. Be specific and use natural language search terms. \
         IMPORTANT: If the topic contains ambiguous terms that could refer to multiple \
         things (e.g. a programming language name that is also a common word or product), \
         always include disambiguating context in every query — for example, prefer \
         'Rust programming language async' over just 'Rust async'.".to_string()
    }
    ResearchTarget::Market { asset_class } => {
        use crate::researcher::pipeline::AssetClass;
        let focus = match asset_class {
            AssetClass::Stock => "recent earnings and analyst guidance, analyst upgrades and downgrades, \
                sector tailwinds and headwinds, macro events affecting the stock, \
                institutional positioning and short interest",
            AssetClass::Crypto => "on-chain metrics such as TVL active addresses and exchange flows, \
                protocol upgrades and ecosystem news, regulatory developments, \
                whale activity and exchange reserves, market sentiment and fear greed index",
            AssetClass::Macro => "central bank signals and interest rate outlook, \
                inflation employment and GDP data, geopolitical risk events, \
                credit market stress indicators, sector rotation and risk sentiment",
        };
        format!(
            "You are a research planning assistant specializing in financial market research. \
             Generate focused search queries covering: {focus}. \
             Use the asset name or topic in every query. Be specific and targeted. \
             Write queries as plain search terms only — no dashes, no hyphens, no special operators."
        )
    }
};
```

- [ ] **Step 3: Update `is_profile_target` to include `Market` (soft-hint domains)**

In `generate_queries`, find the `is_profile_target` let binding and replace it:

```rust
let is_profile_target = matches!(
    target,
    ResearchTarget::Person { .. } | ResearchTarget::Company | ResearchTarget::Market { .. }
);
```

- [ ] **Step 4: Apply ticker expansion in the user message for Market queries**

In `generate_queries`, the user `ChatMessage` currently uses `{topic}`. Wrap it to expand tickers for market targets. Find the `messages` variable assignment and replace it:

```rust
let display_topic = if matches!(target, ResearchTarget::Market { .. }) {
    expand_ticker(topic)
} else {
    topic.to_string()
};

let messages = vec![
    ChatMessage::system(system_prompt),
    ChatMessage::user(format!(
        "Research topic: {display_topic}\n\n\
         Generate exactly {max_queries} distinct search queries to research this topic \
         comprehensively. Each query should be on its own line, with no numbering, \
         bullets, quotes, or extra formatting — just the raw query text.{domain_instruction}",
    )),
];
```

- [ ] **Step 5: Run `cargo check`**

```bash
cargo check 2>&1 | head -30
```

Expected: error only in `publisher.rs` (non-exhaustive match). `planner.rs` should be clean.

- [ ] **Step 6: Commit**

```bash
git add src/researcher/planner.rs
git commit -m "feat: add market planner prompt and ticker expansion"
```

---

## Chunk 2: Publisher & MCP Tool

### Task 3: Add market report sections to `publisher.rs`

**Files:**
- Modify: `src/researcher/publisher.rs`

Add a `Market` branch to the `prompt` match in `write_report`. Mode-aware: `Summary` → bullets, everything else → structured sections.

- [ ] **Step 1: Add `Market` arm to the `prompt` match**

Replace the entire `prompt` match expression with the version below (existing arms unchanged, new arm added at the end):

```rust
let prompt = match target {
    ResearchTarget::Person { method } => {
        // ... (keep existing body unchanged)
    }
    ResearchTarget::Company => {
        // ... (keep existing body unchanged)
    }
    ResearchTarget::Topic => {
        // ... (keep existing body unchanged)
    }
    ResearchTarget::Market { asset_class } => {
        use crate::researcher::pipeline::AssetClass;
        let asset_label = match asset_class {
            AssetClass::Stock  => "stock / equity",
            AssetClass::Crypto => "cryptocurrency / blockchain asset",
            AssetClass::Macro  => "macroeconomic topic",
        };
        match mode {
            ResearchMode::Summary => format!(
                "Write a concise bullet-point market brief on {topic} ({asset_label}):\n\
                 - 5-8 bullets covering catalysts, sentiment, and risks\n\
                 - Each bullet: one concrete fact, number, or dated event\n\
                 - No section headers, no introduction\n\n\
                 {sources_text}"
            ),
            _ => format!(
                "You are a financial research analyst preparing a market insight brief on: \
                 {topic} ({asset_label}).\n\
                 Using the research below, write a concise markdown report with these exact sections:\n\n\
                 ## Summary\n2-3 sentences on what is happening and why it matters.\n\
                 ## Key Catalysts\nWhat is driving price action or the current narrative — \
                 earnings, upgrades, protocol news, macro events. Be specific.\n\
                 ## Sentiment\nAnalyst and community tone. Rating changes, price targets, \
                 social sentiment signals.\n\
                 ## Risks\nKey headwinds, regulatory exposure, macro sensitivity, \
                 competitive threats.\n\n\
                 Include numbers, dates, and named events. Cite sources inline with [N] notation.\n\n\
                 {sources_text}"
            ),
        }
    }
};
```

> **Note:** Use `replace_symbol` on `write_report` to replace the full function body — do not use `edit_file` for this multi-arm match change.

- [ ] **Step 2: Run `cargo check`**

```bash
cargo check 2>&1 | head -30
```

Expected: zero errors. All `ResearchTarget` match arms now exhaustive across all three files.

- [ ] **Step 3: Commit**

```bash
git add src/researcher/publisher.rs
git commit -m "feat: add market report section template to publisher"
```

---

### Task 4: Add `market_insight` tool to `mcp_server.rs`

**Files:**
- Modify: `src/mcp_server.rs`

Three changes: new input struct, new tool method, updated `get_info()` instructions.

- [ ] **Step 1: Add `MarketInsightInput` struct**

Insert after `CodeResearchInput` (after line 103). Use `insert_code` after the `CodeResearchInput` symbol, position `"after"`:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MarketInsightInput {
    /// Ticker symbol (e.g. "BTC", "NVDA", "$AAPL") or free-form topic
    /// (e.g. "AI chip stocks", "Ethereum staking post-Shanghai")
    pub query: String,
    /// Asset class for domain and prompt selection.
    /// "stock" | "crypto" | "macro" (default: "macro")
    pub asset_class: Option<String>,
    /// Output depth: "quick" | "summary" | "report" (default) | "deep"
    pub mode: Option<String>,
}
```

- [ ] **Step 2: Add `market_insight` tool method to `impl ResearcherServer`**

Use `insert_code` after the `research_code` method, position `"after"`. The method follows the exact same structure as `research_company`:

```rust
#[tool(description = "Market insight research: stocks, crypto, and macro. \
    query: ticker (BTC, NVDA) or topic (AI chip stocks). \
    asset_class: stock | crypto | macro (default: macro). \
    mode: quick | summary | report (default) | deep.")]
async fn market_insight(
    &self,
    Parameters(input): Parameters<MarketInsightInput>,
) -> String {
    use crate::researcher::pipeline::{
        run, AssetClass, ResearchMode, ResearchRequest, ResearchTarget,
    };

    let asset_class: AssetClass = input
        .asset_class
        .as_deref()
        .unwrap_or("macro")
        .parse()
        .unwrap_or_default();

    let mode: ResearchMode = input
        .mode
        .as_deref()
        .unwrap_or("report")
        .parse()
        .unwrap_or_default();

    let target = ResearchTarget::Market { asset_class };
    let domains = crate::researcher::pipeline::domains_for_target(&target);

    let request = ResearchRequest {
        topic: input.query.clone(),
        mode,
        domains,
        domain_profile: None,
        target,
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

- [ ] **Step 3: Update `get_info()` instructions to list the new tool**

Replace the `with_instructions(...)` string — append the new bullet before the closing `"`:

```
• market_insight(query, asset_class?, mode?)
  Stock, crypto, and macro market research.
  query: ticker (BTC, NVDA, $AAPL) or topic (AI chip stocks).
  asset_class: stock | crypto | macro (default: macro).
  mode: quick | summary | report (default) | deep.
```

The full updated `get_info` `with_instructions` block:

```rust
.with_instructions(
    "AI research agent — 6 tools:\n\
     \n\
     • research(query, mode?, domain_profile?, domains?, max_queries?, max_sources?)\n\
       General web research. Modes: quick=snippets, summary=bullets, report=full markdown (default), deep=thorough.\n\
       Profiles: shopping-ro, tech-news, llm-news, academic, news, travel. Or pass domains:[\"example.com\"] to pin sites.\n\
     \n\
     • research_person(name, method?)\n\
       Meeting prep brief on any person. Covers career, public voice, interests, conversation hooks.\n\
       method: professional | personal | both (default). Sources: LinkedIn, X, GitHub, Facebook, Instagram, Reddit.\n\
     \n\
     • research_company(name, country?)\n\
       Meeting prep brief on a company. Covers what they do, size/stage, news, culture, strategy.\n\
       Sources: LinkedIn, Crunchbase, Bloomberg, Glassdoor, Trustpilot, Wikipedia.\n\
     \n\
     • search_jobs(query, mode?)\n\
       Find remote AI engineering jobs matching your profiles.toml [job-profile].\n\
       mode: list=shortlist (default), deep=full company briefs on top 5. Sources: Remotive, Adzuna, SearXNG.\n\
     \n\
     • research_code(framework, version?, aspects?, repo?, query?)\n\
       Research a library/framework: bugs, breaking changes, releases, community sentiment.\n\
       aspects: bugs | changelog | community | releases (default: bugs+changelog+community).\n\
       query: keyword to narrow, e.g. \"middleware timeout\". repo: GitHub slug e.g. \"tokio-rs/axum\".\n\
     \n\
     • market_insight(query, asset_class?, mode?)\n\
       Stock, crypto, and macro market research. Web research only — no price APIs.\n\
       query: ticker (BTC, NVDA, $AAPL) or topic (AI chip stocks, Ethereum staking).\n\
       asset_class: stock | crypto | macro (default: macro).\n\
       mode: quick | summary | report (default) | deep.\n\
     ".to_string(),
)
```

- [ ] **Step 4: Run `cargo check`**

```bash
cargo check 2>&1 | head -30
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
git add src/mcp_server.rs
git commit -m "feat: add market_insight MCP tool"
```

---

### Task 5: Full build and smoke test

- [ ] **Step 1: Build release binary**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: `Finished release [optimized] target(s) in ...`

- [ ] **Step 2: Rebuild Docker image (updates the HTTP server)**

```bash
docker compose build researcher && docker compose up -d researcher
```

- [ ] **Step 3: Restart MCP server**

In Claude Code: `/mcp` → restart researcher, or restart the session. The new `market_insight` tool should appear in the tool list.

- [ ] **Step 4: Smoke test — crypto quick**

```
market_insight(query="BTC", asset_class="crypto", mode="quick")
```

Expected: sources from coindesk.com, cointelegraph.com, or similar crypto domains. Queries in the response should reference "Bitcoin" not "BTC".

- [ ] **Step 5: Smoke test — stock report**

```
market_insight(query="NVDA", asset_class="stock", mode="summary")
```

Expected: sources from reuters.com, seekingalpha.com, marketwatch.com, or similar. Report contains bullets about Nvidia (not Nintendo or other NVDA ambiguity).

- [ ] **Step 6: Smoke test — macro report**

```
market_insight(query="Federal Reserve interest rate outlook 2025", mode="report")
```

Expected: full report with Summary / Key Catalysts / Sentiment / Risks sections.

- [ ] **Step 7: Final commit if any fixups were needed**

```bash
git add -p
git commit -m "fix: market_insight smoke test fixups"
```
