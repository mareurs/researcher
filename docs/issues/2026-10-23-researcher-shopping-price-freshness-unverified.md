---
id: '2293612abc46f97e'
kind: bug
status: fixed
title: Shopping research cannot establish current offer freshness or live stock
owners:
- marius
tags:
- smoke-test
- researcher
closed: 2026-10-23
opened: 2026-10-23
severity: medium
---

# Shopping offer freshness remains unverified

## Summary
A Romanian shopping smoke test retrieved two MX Master 3S product listings but only one snippet-backed price, with no verified live stock, new condition, or second priced offer. This is an observed coverage/freshness limitation; the price has not been proven false.

## Reproduction
Call topic research for new Logitech MX Master 3S offers in Romania with domain_profile="shopping-ro", intent="product-research", mode="report", max_queries=2, max_sources=3.

## Evidence
Measured 2026-10-23. [Retained response and answer](../evals/research-scenario-smoke-20261023.json), shopping scenario: Altex source snippet quotes 374.99 RON; eMAG output is mostly navigation text and includes reviews/search pages and an MX Master 3 result. The report claims availability, but the final worker correctly leaves stock and condition unresolved.
A separate web-tool snapshot of the same Altex product URL showed 239.99 lei and "stoc epuizat"; that web tool labeled its page crawled five months ago. Conflicting retrieval surfaces establish neither current checkout price nor current stock.

## Root cause
Unknown. Search-snippet freshness and dynamic-page extraction are candidates, not confirmed causes. Distinguish presence of a listing from a verified purchasable offer.

## Fix

Source origin, observation timestamp, truncation and final redirected URL propagate through extraction, evidence selection, returned source entries and report references. Explicit domain filtering preserves requested .ro shops and rejects lookalike/redirect hosts. A snippet/provider extract does not establish current stock/price, and listings do not establish new condition. Verified quotations and independent property-level publication checks preserve those limits; unverifiable prose is repaired or replaced by attributed evidence.

## Tests added

Real HTTP fixtures distinguish fetched pages from fallback snippets and enforce redirected host constraints. A previously surviving snippet-as-page mutation is now killed. Final live shopping report passes independent full-evidence inspection: six eMAG product/price associations match; missing Altex evidence, stock and new condition are explicit. Source snapshot time is not described as publication time or guaranteed live inventory. Artifact: `docs/evals/researcher-quality-repair-20261023.json`.

## Resume

Resolved evidence-handling defect. Coverage remains honestly partial: the inspected run does not establish a second-retailer offer, live stock, or new condition. This is a source limitation, not a verified offer or a reason to fabricate one. Local configuration changes require the existing MCP client to reload.
