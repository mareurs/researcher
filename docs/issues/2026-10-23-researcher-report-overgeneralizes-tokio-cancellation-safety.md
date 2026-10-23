---
id: '4898aca75e3c7729'
kind: bug
status: fixed
title: Researcher report overgeneralizes Tokio I/O cancellation safety
owners:
- marius
tags:
- smoke-test
- researcher
closed: 2026-10-23
opened: 2026-10-23
severity: medium
---

# Researcher report overgeneralizes Tokio cancellation safety

## Summary
A technical smoke test returned a report broadly listing AsyncRead methods and AsyncWrite operations as cancellation-safe and omitted unsafe examples explicitly present in the cited official documentation.

## Reproduction
Call research(query="Tokio select macro cancellation safety safe unsafe operations official documentation", target="topic", mode="report", max_queries=1, max_sources=2, domains=["docs.rs"], intent="developer-docs", raw_json=true). Nondeterministic generation: this records one observed output, not guaranteed recurrence.

## Evidence
Measured 2026-10-23. Exact MCP response and final worker answer: [evaluation](../evals/research-scenario-smoke-20261023.json), technical scenario. Report Safe Operations includes "AsyncRead methods" and "AsyncWrite operations". Official source https://docs.rs/tokio/latest/tokio/macro.select.html lists only specific safe operations and explicitly marks read_exact, read_to_end, read_to_string and write_all unsafe. Report also used a tokio_with_wasm source despite an official Tokio brief.

## Root cause
Unknown. The report-vs-source mismatch is observed; no pipeline stage or truncation mechanism has been isolated. Worker summary stayed within the report and did not repeat the blanket I/O claim.

## Fix

Semantic-root extraction excludes navigation and retains short safety headings/method labels. Source selection supplies only excerpts verified verbatim against retrieved text. Publication preserves origin/time/truncation and checks bounded segments with an independently configured model. The local Vertex setup uses Flash Lite for writing and Pro for verification; per-claim subject/property comparisons catch anonymous-method association and scope changes. One repair is allowed, then attributed evidence replaces an unverifiable synthesis. Draft tokens are held until validation. Disclosure summaries are extracted from the completed report.

## Tests added

Regressions cover navigation/short headings/Unicode budgets, fabricated or missing excerpts, snippet provenance, stream withholding, failed or contradictory verdicts, nonempty property checks, bounded size/call budgets, independent model routing and authoritative source lists. Applied mutation review exposed and closed gaps. The final three-case live round passes independent inspection of every retained excerpt; technical safely returns a verbatim evidence report with all four safe and four unsafe I/O methods. Earlier failed rounds are retained. Evidence: `docs/evals/researcher-quality-repair-20261023.json`.

## Resume

Resolved locally and release binary rebuilt. Semantic checks remain model-dependent; the evidence fallback is an intentional successful safety behavior. Restart an existing MCP client process to load the new binary/environment. No remote publication or commit was performed.
