# Domain Glossary

**ResearchMode** — Enum controlling pipeline depth: `Quick` (crawl only, no LLM report), `Summary` (bullet points), `Report` (default, full structured), `Deep` (2× queries+sources, detailed prompt). Lives in `src/researcher/pipeline.rs`.

**ResearchTarget** — Enum for what's being researched: `Topic` (default, generic), `Person { method: PersonMethod }`, `Company`. Controls which domains are automatically added and which report template is used in `write_report()`.

**PersonMethod** — Sub-enum of `ResearchTarget::Person`: `Company` (professional sources), `Personal` (social/lifestyle sources), `Both`. Parsed from string in MCP inputs.

**domain_profile** — Named key into `profiles.toml` (e.g. `"tech-news"`, `"llm-news"`). Resolved at runtime by looking up `cfg.profiles` map. Unioned with any explicit `domains` list.

**ProgressEvent** — Enum of pipeline milestones emitted via the `on_progress` callback: Planning, Queries, Crawling, Deduplicating, CrawlComplete, Summarizing, SummarizingComplete, WritingReport, Done.

**ScrapedSource** — Raw scraped page: url, title, query (which sub-question fetched it), content (text). Internal to crawler → summarizer path.

**SourceEntry** — Lightweight public-facing source: url, title, snippet (first 200 chars). Included in `ResearchResult.sources`.

**SourceSummary** — LLM-generated summary of one ScrapedSource, tagged with url/title/query. Input to `write_report()`.

**JobProfile** — User's job preferences loaded from `[job-profile]` section of profiles.toml. Required for `search_jobs` — tool returns error string if absent.

**ScoredJob** — A `JobListing` with LLM-assigned score (u8 0-10) and reason string. Only jobs scoring ≥ threshold (default 6) are included in output.

**token_tx** — `Option<mpsc::Sender<String>>` passed into `run()`. `None` → blocking complete(), `Some(tx)` → streaming. This is the single switch between streaming and non-streaming modes.

**strip_thinking** — Boolean flag (`STRIP_THINKING_TOKENS=true`) that strips `<think>...</think>` blocks from Qwen3 responses. Applied in both `client.rs` (complete path) and `stream.rs` (streaming path).
