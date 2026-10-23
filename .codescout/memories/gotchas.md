# Gotchas & Known Issues

## Config Drift
- **Problem:** `config_from_env()` in `src/mcp_server.rs` is a hand-maintained duplicate of `Config`. Adding a new field to `Config` without also adding it to `config_from_env()` causes the MCP binary to silently use the wrong default.
  **Fix:** Always update BOTH files. Search for an existing env var name in `mcp_server.rs` to find the pattern.

## Thinking Token Suppression — Three Mechanisms
- **Problem:** Three overlapping mechanisms suppress thinking tokens: `/no_think` prompt prefix, `STRIP_THINKING_TOKENS` post-processing, and `disable_thinking` flag sending `chat_template_kwargs`. Changing one without awareness of the others can cause unexpected behavior.
  **Fix:** See conventions memory for which stages use which mechanism.

## Fast LLM Fallback
Verified 2026-10-23: new_fast uses the primary backend/model when both fast base URL and fast provider are empty. Thinking suppression is enabled for an explicitly selected fast backend, not for this primary fallback. Vertex strips the Qwen-only leading /no_think control prefix.

## Explicit Domains and Redirects — Fixed 2026-10-23
Every search backend now checks parsed hostname boundaries, including explicitly requested .ro sites. SearXNG language heuristics do not discard explicit domain results. Pipeline propagates request/profile hard domains; target fallback hints remain soft. Fetched evidence records final redirect URL and cannot cross hard domain constraints. HTTP regression fixtures and applied mutation review cover provenance/host handling.

## Job Search Uncapped Concurrency in Deep Mode
- **Problem:** `write_job_report()` deep mode calls `run()` recursively for top 5 companies via `join_all` — no rate limiting or concurrency cap. Can saturate the LLM backend.
  **Fix:** Wrap with a semaphore or process sequentially if LLM backend struggles.

## Job Scoring Prompt Unbounded Size
- **Problem:** `score_listings()` sends ALL job listings in a single LLM call. With many listings, prompt can be very long → truncation → silently wrong scores.
  **Fix:** Add batching if listing count is high. No fix currently in code.

## `crawl_all()` is Sequential by Design
- **Problem:** 4 sub-queries run sequentially (not in parallel). This adds latency on slow search backends.
  **Fix:** Parallelizing requires `Arc<Mutex<HashSet<String>>>` for visited URL sharing. Not a bug, but a deliberate tradeoff. See `src/researcher/crawler.rs` comment.

## `write_code_report()` Dead Code
- **Problem:** `write_code_report()` has `#[allow(dead_code)]` attribute but is called from the `research_code` MCP tool. The compiler warning was suppressed rather than fixed.
  **Fix:** Investigate whether the dead-code lint is from a module visibility issue; remove the allow attribute once resolved.

## Python Infrastructure is a Skeleton
- **Problem:** `src/model/`, `src/training/`, `src/data/`, `src/eval/`, `src/export/` are empty `__init__.py` stubs. `tests/model/test_cross_encoder.py` imports `src.model.cross_encoder` which doesn't exist. Tests will fail if run.
  **Context:** This is groundwork for planned fine-tuning of a custom cross-encoder reranker. Not production code.

## Docker Stack Split (CLAUDE.md is Outdated)
- **Problem:** CLAUDE.md describes a single `docker compose --profile local-llm up` command. The actual setup uses two separate stacks: `infra/docker-compose.yml` (AI infra) and root `docker-compose.yml` (researcher app), coordinated via the external `ai-infra-net` Docker network.
  **Fix:** Use Makefile targets or start stacks manually in order (infra first, then app).

## Rerank Score Units and Runtime
Verified 2026-10-23: ranking uses within-batch percentile relevance so raw model scale cannot overwhelm the blend. RERANK_MIN_SCORE defaults to -inf (disabled), in Config and config_from_env; explicit thresholds stay in raw-model units. README/CLAUDE document it. Healthy local TEI is port30083; the unavailable Codex/project48083 setting was corrected. Do not introduce model-specific calibrated defaults.

## Secret Scans: codescout `grep` Silently Skips Dotfiles
- **Problem:** codescout's `grep` tool does not match files whose names begin with `.` (e.g. `.mcp.json.bak-*`). A pre-commit credential sweep run with it returns "0 matches" while live API keys sit in tracked dotfiles. This bit us on 2026-07-13: the scan came back clean, a push went out, and four live keys (Gemini, Brave, Tavily, Exa) landed in the then-public repo.
  **Fix:** Never clear a commit for secrets using codescout `grep`. Use `git grep -I -n -E '<patterns>' $(git rev-list <range>)` — it searches real tracked blobs across history, dotfiles included. Scan the *pushed range*, not just the working tree.

## `.mcp.json` Backups Carry Live Keys
- **Problem:** `.gitignore` had `.mcp.json` but not `.mcp.json.bak-*`, so normalizer backups (`.mcp.json.bak-norm2`, `.mcp.json.bak-mcpnorm-*`) holding real `LLM_API_KEY` / `BRAVE_API_KEY` / `TAVILY_API_KEY` / `EXA_API_KEY` were tracked. Now ignored via `.mcp.json*` + `!.mcp.json.example`.
  **Fix:** `config/searxng/settings.yml` carried a live Brave `api_key` in a tracked file too. It is now gitignored; only `config/searxng/settings.yml.example` (placeholder) is tracked, and `make searxng-config` (a prerequisite of `make infra-up`) bootstraps the real file from it. Docker bind-mounts `settings.yml`, so if it were missing Docker would silently create a *directory* there — hence the bootstrap target. Put live keys in `settings.yml` only, never in the template.

## Tooling State Keeps Creeping Back Into Git
- **Problem:** `.buddy/` and `.codescout/` runtime files (session narratives, debug logs, `embeddings/project.db`, `.fastembed_cache/` ONNX blobs) get re-tracked whenever a broad `git add` runs with the ignore rules missing. Commit 9b8d460 ("checkpoint") clobbered the ignore block added in 516fd5d, re-tracking 249 files.
  **Fix:** Ignore rules restored in `.gitignore` under "Tooling state". `.codescout/memories/`, `project.toml` and `system-prompt.md` are deliberate project artifacts and STAY tracked — never blanket-ignore all of `.codescout/`.

## Grounding, Measurement and Evidence Audits — 2026-10-23
LLM_VERIFIER_MODEL is optional and must be served by the publisher backend. Local Vertex uses gemini-3.1-pro-preview for property checks and Flash Lite for other stages. Model checks are not proof of universal entailment: final live audit assessed3 complete reports, all grounded, with1 explicit evidence fallback and a partial shopping comparison. Retained artifacts and measured timings: docs/evals/researcher-quality-repair-20261023.json. Vertex thoughtsTokenCount is separate from candidatesTokenCount and must be included in generated-token accounting. Async unit tests that replace TOKEN_CACHE share TEST_TOKEN_LOCK and restore prior cache values. Saved-report audits use the companion skill's capture_evidence.py raw-byte helper with hashes; never reconstruct Markdown from rendered reads.
