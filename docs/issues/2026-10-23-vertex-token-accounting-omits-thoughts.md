---
id: '6ae2385b7bd22876'
kind: bug
status: fixed
title: Vertex output-token accounting omitted reasoning tokens
closed: 2026-10-23
opened: 2026-10-23
severity: medium
---

## Summary

The Vertex adapter counted only `candidatesTokenCount` as generated output, omitting `thoughtsTokenCount`. A retained Pro response reports1945 prompt,250 candidate,3567 reasoning and5762 total tokens. Existing accounting returned250 generated tokens instead of3817.

## Fix

Deserialize the separate reasoning counter and sum candidates plus reasoning for generated-token usage in both blocking and SSE paths. Missing reasoning fields default to zero. The counters are disjoint according to [Vertex GenerateContentResponse](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse).

## Verification

A real HTTP fixture exercises both complete and stream. Observed RED tuple `(1945,250,1945,250)` became GREEN `(1945,3817,1945,3817)`. Applied mutations independently reverted each callsite; each failed with only that path returning250. Both were restored and the source SHA-256 matched exactly. Full Rust suite and strict Clippy pass. Fixture counts are API-reported usage; this does not claim to measure dollar cost or every historical trace.
