# Domain Glossary

**ResearchTarget** — enum controlling which domain list, prompt templates, and report format are used: `Topic` (default web research), `Person { method }` (LinkedIn/X/GitHub/social), `Company` (LinkedIn/Crunchbase/Glassdoor), `Market { asset_class }` (financial research). Lives in `src/researcher/pipeline.rs`.

**ResearchMode** — enum controlling pipeline depth: `Quick` (crawl only, no summarize/write), `Summary` (bullet list report), `Report` (full markdown, default), `Deep` (2× queries/sources). Lives in `src/researcher/pipeline.rs`.

**fast_stages** — comma-separated list of pipeline stage names (`planner`, `summarizer`, `publisher`) that should use the fast LLM. Set via `LLM_FAST_STAGES` env or overridden per-request. Default: `planner,summarizer`.

**heavy LLM / fast LLM** — two LlmClient instances. Heavy uses `LLM_BASE_URL`+`LLM_MODEL` (e.g. Qwen3.5-9B). Fast uses `LLM_FAST_BASE_URL`+`LLM_FAST_MODEL` (e.g. Qwen3.5-4B). If `LLM_FAST_BASE_URL` is empty, fast falls back to heavy backend but still disables thinking tokens.

**disable_thinking** — LlmClient flag that sends `chat_template_kwargs: {"enable_thinking": false}` in the request body. llama.cpp-specific extension; harmless on other backends. Always true for the fast client.

**domain profile** — named list of domains from `profiles.toml` (e.g. `shopping-ro`, `tech-news`). Passed as `domain_profile` in ResearchRequest. Loaded at startup by `load_profiles()` into `Config.profiles`.

**TEI** — Text Embeddings Inference (Hugging Face server). Hosts embedding model (BAAI/bge-large-en-v1.5) for dedup and cross-encoder (`cross-encoder/ms-marco-MiniLM-L-6-v2`) for reranking. Runs on CPU in the infra stack.

**JudgedSummary** — LLM output struct `{ relevant: bool, confidence: f32, summary: String }` from `summarize_source()`. Irrelevant sources (relevant=false) are dropped before report writing.

**snippet fallback** — when `fetch_and_extract()` fails for a URL, `crawl_query()` uses the search snippet as `content` with `raw_html_len = 0`. This sentinel is used by quality filter (relaxed word threshold: 8 vs 100).

**on_progress** — `impl Fn(ProgressEvent)` closure passed to `run()`. In CLI: prints to stderr. In HTTP SSE: serializes to SSE JSON event. In MCP: eprintln! to stderr. Decouples pipeline from transport.

**combined rerank score** — `cross_encoder_score × 0.7 + domain_authority × 0.2 + quality_heuristic × 0.1`. Sources below `RERANK_MIN_SCORE` (-5.0 logits default) are dropped before scoring.

**config_from_env()** — manual env-var reader in `src/mcp_server.rs` that duplicates `Config` construction for the MCP binary (which can't use clap). Must be kept in sync with `Config` manually — there is no compile-time check.

**`/no_think`** — Qwen3 chat template directive prepended to system prompts for planner and summarizer. Separate from STRIP_THINKING_TOKENS (post-hoc strip) and disable_thinking (request-level flag).

**PersonMethod** — `Company | Personal | Both` — controls which social platforms are searched for person research.

**AssetClass** — `Stock | Crypto | Macro` — controls domain list and prompt framing for market research.
