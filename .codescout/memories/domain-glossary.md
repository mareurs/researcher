# Domain Glossary

**ScrapedSource** — a single scraped web page: `{ url, title, content: String }`. The primary data unit flowing through the pipeline from crawler to embeddings to summarizer. Lives in `src/researcher/crawler.rs`.

**SourceSummary** — LLM-generated summary of a `ScrapedSource`: `{ url, title, summary }`. Output of `summarize_all`, input to `write_report`. Lives in `src/researcher/summarizer.rs`.

**ResearchResult** — final output of `pipeline::run()`: `{ topic, queries, source_count, report }`. The `report` field is the full Markdown text with sources appended by `format_report`.

**ProgressEvent** — enum of pipeline lifecycle events (Planning, Crawling, Summarizing, etc.) emitted via callback. Has a `Display` impl with emoji messages. Separate from the token stream.

**token_tx** — `Option<mpsc::Sender<String>>` passed into `run()` and `write_report()`. Present in CLI/SSE modes (streaming LLM output token by token); absent in MCP/JSON modes (blocking complete).

**on_progress** — `impl Fn(ProgressEvent)` callback. In CLI: prints to stdout. In SSE: sends SSE events. In MCP: noop/logging.

**TEI** — Text Embeddings Inference server (Hugging Face). Optional component for source dedup+rerank. Enabled by setting `EMBED_BASE_URL`. Uses `BAAI/bge-large-en-v1.5`.
