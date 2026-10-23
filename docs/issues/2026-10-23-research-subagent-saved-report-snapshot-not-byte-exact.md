---
id: da5c40d691037ec8
kind: bug
status: fixed
title: Research subagent reformats saved report instead of retaining an exact audit snapshot
owners:
- marius
tags:
- smoke-test
- researcher
closed: 2026-10-23
opened: 2026-10-23
severity: medium
---

# Saved-report audit snapshot is not byte exact

## Summary
The research worker read a saved report but rewrote its audit copy through Markdown rendering, changing list indentation and escaping a backslash. Its answer remained grounded; its audit-copy fidelity contract failed.

## Reproduction
One gpt-6-luna/low worker using the evidence-audit research-subagent prompt for ESA Euclid goals/instruments, with default report/toc output. Inspect the original report path from the captured MCP envelope and compare it to the worker snapshot with diff -u.

## Evidence
Measured 2026-10-23. [Evaluation](../evals/research-scenario-smoke-20261023.json), research scenario retains both original saved_report and worker_saved_snapshot, with snapshot_byte_match=false. Whitespace was normalized and the LaTeX backslash in approx doubled. The original report was independently copied and used for the final grounding audit.

## Root cause
Observed transformed copy, not a byte copy. The skill says to preserve report content but does not require copying original bytes or checking a hash. The exact worker copy implementation was not separately instrumented.

## Fix

Added `research-subagent/scripts/capture_evidence.py` to the canonical companion skill and refreshed the installed Codex copy. Workers now copy saved reports as raw bytes before synthesis and retain a byte-count/SHA-256 manifest. Source and installed skill/helper hashes match.

## Tests added

Two helper regressions pass: nested Markdown/backslashes/Unicode/CRLF remain byte-identical (including verified hash and byte count); missing input and duplicate destinations fail explicitly. Independent Luna/low saved-report forward test produced an exact 199-byte copy with matching SHA-256 and a grounded answer. Artifacts: `/tmp/researcher-snapshot-forward-20261023/`.

## Resume

Resolved locally. Source plugin changes are uncommitted; no marketplace release or remote push was performed.
