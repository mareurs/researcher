# Market Insight Tool — Design Spec

**Date:** 2026-03-10  
**Status:** Approved  

---

## Overview

Add a `market_insight` MCP tool to the researcher server. It covers stock market, market-moving news, and blockchain/crypto — powered entirely by web research (no price APIs), following the same `ResearchTarget` pattern as `research_person` and `research_company`.

---

## Input Schema

```rust
struct MarketInsightInput {
    query: String,                // ticker ("BTC", "NVDA") or topic ("AI chip stocks")
    asset_class: Option<String>,  // "stock" | "crypto" | "macro" | null → defaults to "macro"
    mode: Option<String>,         // "quick" | "summary" | "report" (default) | "deep"
}
```

- `asset_class` defaults to `"macro"` when omitted.
- `query` may be a bare ticker (uppercase, ≤5 chars, optional `$` prefix) or a free-form topic.
- Ticker detection: a small pure function in `planner.rs` expands known tickers to their full name for query enrichment (e.g. `"BTC"` → queries include `"Bitcoin"`, `"NVDA"` → `"Nvidia"`), preventing symbol ambiguity.

---

## Pipeline Integration

### New types in `pipeline.rs`

```rust
pub enum AssetClass {
    Stock,
    Crypto,
    Macro,
}

// Extended ResearchTarget:
pub enum ResearchTarget {
    Topic,
    Person { method: PersonMethod },
    Company,
    Market { asset_class: AssetClass },  // new
}
```

`AssetClass` and the `Market` variant live in `pipeline.rs` alongside the existing target types.

### Domain Lists (`domains_for_target`)

Soft hints only — not hard `site:` filters — so the planner can mix open-web queries when needed.

| Asset Class | Domains |
|---|---|
| `Stock` | reuters.com, ft.com, seekingalpha.com, marketwatch.com, investopedia.com, finance.yahoo.com, fool.com, cnbc.com |
| `Crypto` | coindesk.com, cointelegraph.com, decrypt.co, theblock.co, bitcoinmagazine.com, cryptoslate.com, reddit.com/r/CryptoCurrency |
| `Macro` | reuters.com, ft.com, bloomberg.com, cnbc.com, wsj.com, economist.com, marketwatch.com |

---

## Planner Prompts

The planner system prompt is specialised per asset class. Each drives queries toward the information that matters most:

**Stock** — earnings/guidance, analyst upgrades/downgrades, sector tailwinds/headwinds, macro events affecting the stock, short interest, institutional moves.

**Crypto** — on-chain metrics (TVL, active addresses, exchange flows), protocol upgrades, regulatory developments, whale activity, market sentiment/fear-greed index.

**Macro** — central bank signals, CPI/jobs/GDP releases, geopolitical risk, credit market stress, sector rotation signals.

All asset classes: queries written as plain search terms — no quotes, no dashes, no `site:` filters (same planner hygiene as existing targets).

---

## Publisher Report Structure

The publisher system prompt requests this section layout for `report` and `deep` modes:

```
## Summary
## Key Catalysts   — what is driving the price / narrative right now
## Sentiment       — analyst and community tone, recent rating changes
## Risks           — headwinds, regulatory exposure, macro sensitivity
## Sources
```

For `quick` mode: 5-bullet briefing — "what you need to know about X right now" — using the existing `ResearchMode::Quick` short-circuit (no LLM summarization pass).

---

## File Change Summary

| File | Change |
|---|---|
| `src/researcher/pipeline.rs` | Add `AssetClass` enum + `ResearchTarget::Market` variant; extend `domains_for_target()` |
| `src/researcher/planner.rs` | Add market branch to system prompt selection; ticker expansion function |
| `src/researcher/publisher.rs` | Add market report section template to system prompt |
| `src/mcp_server.rs` | Add `MarketInsightInput` struct + `market_insight()` tool method; update `get_info()` |

**No new files. No new dependencies. ~200 lines added.**

---

## Non-Goals

- No live price data or market data APIs (CoinGecko, Alpha Vantage, etc.)
- No charting or time-series output
- No portfolio tracking or alerts
- No auto-detection of asset class (caller must specify or accept `macro` default)
