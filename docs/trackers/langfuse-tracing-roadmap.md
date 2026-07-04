---
id: '53796e12647f711e'
kind: tracker
status: draft
title: Langfuse Tracing Roadmap
owners: []
tags:
- tracing
- langfuse
- observability
- roadmap
topic: null
time_scope: null
---

# Langfuse Tracing Roadmap

## Why this initiative exists

The researcher pipeline (`run()` in `src/researcher/pipeline.rs`) is a 10-stage
orchestrator — plan → crawl → filter → dedup → rerank → summarize → write —
with LLM calls at the planner, summarizer (×M), and publisher stages. Today it
has **zero structured observability** beyond `tracing` log lines. We cannot see,
per run: what each LLM call received/returned, where latency and token cost go,
or how a change to prompts/models/params shifts output quality.

The goal is a **faithful, hierarchical trace of every run** sent to Langfuse, so
we can (1) debug pipeline runs, (2) track cost & latency per stage/model, and
(3) build eval datasets and quality scores. The throughline is understanding the
whole flow well enough to **improve — eventually self-improve — it**.

**Backend decision:** reuse the existing self-hosted Langfuse v3 instance owned
by `llm-proxy` (`/home/marius/agents/llm-proxy/docker-compose.langfuse.yml`,
`localhost:3000`), but under a **new, isolated Langfuse project** with its own
API keys. No new infra.

**Instrumentation decision:** app-level Langfuse REST client in Rust, adapted &
extended from the proven `llm-proxy/src/langfuse.rs` (flat trace+generation →
nested trace+span+generation). Chosen over OTel-native (heavy, finicky gen-ai
mapping) and proxy-level capture (misses non-LLM stages). See the design spec:
`docs/superpowers/specs/2026-07-04-langfuse-tracing-design.md`.

## Phase descriptions

### Phase 1 — v1: full-pipeline trace capture (in progress)

One trace per `run()`; nested spans per stage; generations for every LLM call
with model/usage/IO; config-gated (empty env ⇒ no-op); fire-and-forget so it
never blocks or fails a run. This is the scope of the design spec.

**Acceptance:** a real research query produces, in the new Langfuse project, a
single trace whose tree matches the pipeline (plan → crawl → filter →
[dedup] → [rerank] → summarize → write), with LLM generations carrying token
usage and truncated input/output, and stage spans carrying count metadata.
With Langfuse env unset, behaviour and performance are unchanged.

### Phase 2 — depth & self-improvement (backlog)

Each item below is a discrete follow-up. Full detail:

**T-7 · Per-query & per-URL crawl/scrape sub-spans.**
- *Why:* scraping is usually the slow path; v1 collapses crawl into one span, so
  we can't see which query or URL cost the time or failed.
- *Shape:* thread `Option<&ResearchTrace>` + parent span id into `crawl_all` /
  `crawl_query` in `src/researcher/crawler.rs`; emit a span per query
  (backend, result count) and a child span/event per URL fetch (status, bytes,
  ms, ok/failed).
- *Open questions:* span vs event grain per URL; volume control when a query
  fans out to many URLs.
- *Acceptance:* crawl span shows per-query children, each with per-URL fetch
  timings and outcomes.

**T-8 · LLM-judge auto-scoring → Langfuse `score-create` (the self-improve lever).**
- *Why:* the direct enabler of "self-improve" — attach objective-ish quality
  signals to each run so regressions and improvements are measurable over time.
- *Shape:* after `write-report`, run a judge LLM call scoring the report on
  relevance / coverage / faithfulness-to-sources (and citation validity);
  attach as trace-level scores via the `score-create` event (the v1 client
  already exposes the `.score()` hook). Gate behind its own env flag so it's
  opt-in and its cost is visible.
- *Open questions:* judge model choice (fast vs heavy); rubric definition;
  whether to also score individual summaries; cost budget.
- *Acceptance:* runs carry numeric scores in Langfuse; scores are filterable and
  chartable across runs.

**T-9 · `sessionId` grouping.**
- *Why:* relate multiple runs — an MCP session, a job-search deep dive that fans
  out to N company briefs, or a manual research campaign.
- *Shape:* thread an optional session id into `ResearchRequest` / the trace
  root; MCP binary derives one per stdio session; job deep-mode groups its
  child company runs under the parent run's session.
- *Acceptance:* related runs share a session in the Langfuse UI.

**T-10 · One-click trace URL logging.**
- *Why:* today we log `trace_id`; a clickable URL is faster.
- *Shape:* optional `LANGFUSE_PROJECT_ID` env; when set, `run()` logs
  `{host}/project/{id}/traces/{trace_id}` at info.
- *Acceptance:* the log line is a working link.

**T-11 · Langfuse datasets integration.**
- *Why:* turn captured runs into offline eval sets for prompt/model A-B testing.
- *Shape:* export selected traces (or push at capture time) as dataset items;
  builds on T-8 scores as ground-truth-ish labels.
- *Open questions:* selection criteria; manual vs automatic curation.
- *Acceptance:* a dataset in Langfuse populated from real runs, usable in an
  eval run.

**T-12 · Optional llm-proxy routing (approach C as complement).**
- *Why:* if LLM traffic later routes through `llm-proxy`, avoid double-logging
  and get proxy-side generation capture for free.
- *Shape:* propagate trace/parent ids to the proxy via request headers/metadata
  so proxy-logged generations nest under the app trace instead of duplicating.
- *Open questions:* header contract with llm-proxy; dedupe strategy.
- *Acceptance:* generations appear once, correctly nested, whether or not calls
  transit the proxy.

## History

### 2026-07-04 — roadmap created

Brainstormed the tracing design; chose reuse-instance/new-project + app-level
Rust client (approach A). Phase 1 (v1) scoped into the design spec; Phase 2
items T-7..T-12 captured here. Per-URL scrape spans (T-7) and LLM-judge scoring
(T-8) explicitly deferred out of v1 by decision.

