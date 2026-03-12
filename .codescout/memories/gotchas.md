# Gotchas & Known Issues

## Config Drift
- **Problem:** `config_from_env()` in `src/mcp_server.rs` is a hand-maintained duplicate of `Config`. Adding a new field to `Config` without also adding it to `config_from_env()` causes the MCP binary to silently use the wrong default.
  **Fix:** Always update BOTH files. Search for an existing env var name in `mcp_server.rs` to find the pattern.

## Thinking Token Suppression — Three Mechanisms
- **Problem:** Three overlapping mechanisms suppress thinking tokens: `/no_think` prompt prefix, `STRIP_THINKING_TOKENS` post-processing, and `disable_thinking` flag sending `chat_template_kwargs`. Changing one without awareness of the others can cause unexpected behavior.
  **Fix:** See conventions memory for which stages use which mechanism.

## Fast LLM Falls Back Silently
- **Problem:** If `LLM_FAST_BASE_URL` is empty, `LlmClient::new_fast()` uses the heavy backend URL/model but still sets `disable_thinking = true`. Planner/summarizer always disable thinking even on single-GPU setups.
  **Fix:** Expected behavior — just be aware the fast client always has thinking disabled.

## `shopping-ro` Profile Domain Filtering Bug
- **Problem:** `searxng.rs::is_non_english_domain()` filters TLDs like `.ro`, `.de`, `.fr` etc. The `shopping-ro` profile targets Romanian sites (olx.ro etc.) but SearXNG results from `.ro` domains get filtered out.
  **Fix:** Either add an exception to `is_non_english_domain()` for explicitly requested domains, or configure SearXNG to return results without TLD filtering for this profile. (Verify filter logic at `src/search/searxng.rs`.)

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

## RERANK_MIN_SCORE Not in CLAUDE.md
- **Problem:** `RERANK_MIN_SCORE` env var (default -5.0 logits) is implemented but missing from CLAUDE.md's env var reference table. Can't be discovered from docs alone.
  **Fix:** Set in `.env` if you want stricter reranking. Check `src/embeddings/reranker.rs` for current default.
