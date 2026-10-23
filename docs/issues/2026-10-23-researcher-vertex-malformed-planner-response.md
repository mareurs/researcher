---
id: a0cbb0d8a84f881a
kind: bug
status: fixed
title: researcher Vertex planner receives MALFORMED_RESPONSE and hides finish reason
owners:
- marius
tags:
- researcher
- vertex
closed: 2026-10-23
opened: 2026-10-23
owner: marius
severity: high
---

# BUG: researcher Vertex planner receives MALFORMED_RESPONSE

## Summary

Research fails even through a fresh, healthy MCP connection. Both Codex and Claude environment configurations reproduce the failure. No configuration or implementation changed.

## Class fit

Unclassified external model integration failure: a successful HTTP response contains an unsuccessful model finish reason, which researcher's response model discards. This differs from the separate process-lifetime failure closing the transport.

## Symptom (Effect)

Research returns text `Error: Vertex response contained no text candidate` inside an MCP result with `isError: false`.

## Reproduction

Launch the configured researcher binary, initialize MCP, call research(query="Rust official documentation", mode="quick", max_queries=1, max_sources=1). Reproduced using both `/home/marius/.codex/config.toml` and `/home/marius/.claude/.claude.json` researcher environments.

## Environment

Researcher source HEAD inspected: `f862f4f0ac9fbea357f035900b917748b6f20a73`. Probes used the existing release binary; it was not rebuilt, so source HEAD is not asserted to be the binary's build revision.

2026-10-23; Vertex model gemini-3.1-flash-lite, location eu, project sp-mrv-gemini-dev, base https://aiplatform.googleapis.com/v1beta1, max output tokens 4096.

## Ownership

This failure is independent of Codex: a freshly launched researcher subprocess reproduces it with either client's configuration. Vertex owns the observed MALFORMED_RESPONSE generation outcome; whether the trigger is model-side behavior or researcher's request/prompt is unresolved. Researcher owns preserving the finish reason in diagnostics and returning a proper MCP error instead of isError=false. Do not label the upstream cause as confirmed researcher prompt corruption or a Codex defect.

## Root cause

Controlled 2026-10-23 probes of the actual planning request isolated the leading Qwen-specific `/no_think\n` system-prompt directive as the trigger for this Vertex failure. With temperature 0.3 the original request returned HTTP 200 with `finishReason: MALFORMED_RESPONSE` and no parts; removing only the directive returned a valid search query with `STOP`. Changing only temperature to 1.0 still failed; removing the directive at temperature 1.0 also succeeded. The model's internal reason for this sensitivity remains unknown.

Researcher forwarded this provider-specific directive to Vertex, discarded `finishReason` and `finishMessage`, and converted pipeline errors to successful MCP text responses. Those are researcher-owned integration defects.

## Evidence

An ephemeral localhost forwarding proxy captured the actual researcher request and Vertex response without modifying persistent config. Request generationConfig: temperature 0.3, maxOutputTokens 4096. Response: candidates=[{content:{role:"model"},finishReason:"MALFORMED_RESPONSE"}], usageMetadata promptTokenCount=174, totalTokenCount=184, thoughtsTokenCount=10. Direct OAuth ADC refresh succeeded; a direct request to the same configured Vertex endpoint/model with prompt `Reply with OK.` returned `OK.` and finishReason STOP. Therefore credentials, endpoint access and model availability work for that control prompt.

## Hypotheses tried

Credentials invalid, missing model access, and empty configured token budget ruled out by the controls above. Codex-specific environment drift does not explain this failure because Claude configuration reproduces it too.

## Fix

`src/llm/vertex.rs::build_request` removes only a leading `/no_think\n` from the system instruction before sending to Vertex. Literal occurrences elsewhere and OpenAI/Qwen requests retain their previous content. Blocking and streaming Vertex responses preserve and check candidate finish reasons; malformed/blocked/unknown failure outcomes are errors even when partial text exists, with `finishReason` and optional `finishMessage` in diagnostics. Existing `MAX_TOKENS` partial-text behavior is retained.

MCP dispatch functions propagate `Result<String>` and the research tool maps failures to `CallToolResult::error`, setting `isError: true`.

## Verification status

Implemented and verified in the working tree on 2026-10-23; no fix commit has been created yet. Independent review applied four mutations across both fixes: all four were caught, zero survived; removing Vertex prefix normalization failed its serialized request assertion, and removing streaming finish validation incorrectly returned partial text and failed its SSE regression. Archive after the fix is committed on master under the repository's archive rules.

## Tests added

Vertex unit tests cover serialized prompt normalization without removing literal user/system text, malformed empty-response diagnostics, rejection of malformed partial text, and a local SSE server emitting partial text followed by `MALFORMED_RESPONSE`. The new regressions were observed failing before their respective fixes, then passing.

`tests/mcp_lifecycle.rs::research_failures_are_mcp_tool_errors` calls the real MCP stdio server for all six targets and asserts error flags and diagnostic text with controlled failing inputs.

Verified 2026-10-23: `cargo test` passed (38 CLI unit tests, 42 MCP unit tests, 3 integration tests; two existing live-ingestion tests and one subprocess fixture ignored by the parent runner). `cargo check`, `cargo build --release`, and `git diff --check` passed. Clippy with `-D warnings` is blocked by existing warnings in progressive disclosure, jobs scorer, planner, and search modules. The repository-wide formatting check also reports existing formatting outside this fix; `src/llm/vertex.rs` and the new integration test were formatted.

Live verification of the freshly built release binary: the original quick-research request returned `https://doc.rust-lang.org/stable/`. A local HTTP 200 Vertex fixture returning `MALFORMED_RESPONSE` plus `finishMessage` was propagated through the entire release MCP call with `isError: true` and both diagnostic fields preserved.

## Workarounds

N/A — the rebuilt release binary contains the integration fix. Existing MCP clients must reconnect/restart their researcher connection to load it.

## Resume

N/A — fix implemented and verified. Reconnect existing MCP clients to load the rebuilt release binary.

## References

Transport failure: [transport failure](2026-10-23-researcher-parent-thread-exit-closes-codex-transport.md).
