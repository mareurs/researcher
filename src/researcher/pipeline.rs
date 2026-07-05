use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

use super::crawler::{crawl_all, ScrapedSource};
use super::judge;
use super::planner::{broaden_queries, generate_queries};
use super::publisher::{format_report, write_report};
use super::summarizer::summarize_all;
use crate::config::Config;
use crate::embeddings::client::EmbedClient;
use crate::embeddings::dedup::deduplicate;
use crate::embeddings::reranker::RerankerClient;
use crate::llm::client::LlmClient;
use crate::researcher::quality::{assess_quality, filter_sources};
use crate::trace::{ResearchTrace, SpanHandle};
use serde_json::json;

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchMode {
    Quick,
    Summary,
    #[default]
    Report,
    Deep,
}

impl std::str::FromStr for ResearchMode {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "quick" => Ok(ResearchMode::Quick),
            "summary" => Ok(ResearchMode::Summary),
            "deep" => Ok(ResearchMode::Deep),
            _ => Ok(ResearchMode::Report),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PersonMethod {
    Company,
    Personal,
    Both,
}

impl std::str::FromStr for PersonMethod {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "company" => Ok(PersonMethod::Company),
            "personal" => Ok(PersonMethod::Personal),
            _ => Ok(PersonMethod::Both),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum AssetClass {
    Stock,
    Crypto,
    #[default]
    Macro,
}

impl std::str::FromStr for AssetClass {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "stock" => Ok(AssetClass::Stock),
            "crypto" => Ok(AssetClass::Crypto),
            _ => Ok(AssetClass::Macro),
        }
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum ResearchTarget {
    #[default]
    Topic,
    Person {
        method: PersonMethod,
    },
    Company,
    Market {
        asset_class: AssetClass,
    },
}

pub struct ResearchRequest {
    pub topic: String,
    pub mode: ResearchMode,
    /// Raw domain list (e.g. ["olx.ro", "reddit.com"])
    pub domains: Vec<String>,
    /// Named profile from profiles.toml (e.g. "shopping-ro")
    pub domain_profile: Option<String>,
    pub target: ResearchTarget,
    /// Planner query style hint: developer-docs | news | product-research | academic | general
    pub intent: Option<String>,
}

impl ResearchRequest {
    #[allow(dead_code)]
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            mode: ResearchMode::default(),
            domains: vec![],
            domain_profile: None,
            target: ResearchTarget::default(),
            intent: None,
        }
    }
}

pub fn domains_for_target(target: &ResearchTarget) -> Vec<String> {
    // Returns preferred (not mandatory) domains for query planning.
    // These are passed as soft hints — the planner uses them for some queries
    // but also generates open-web queries for better scrape coverage.
    match target {
        ResearchTarget::Topic => vec![],
        ResearchTarget::Person { method } => {
            let professional: &[&str] = &[
                "github.com",
                "wikipedia.org",
                "medium.com",
                "news.ycombinator.com",
                "youtube.com",
            ];
            let personal: &[&str] = &["reddit.com", "youtube.com", "wikipedia.org"];
            let domains: Vec<&str> = match method {
                PersonMethod::Company => professional.to_vec(),
                PersonMethod::Personal => personal.to_vec(),
                PersonMethod::Both => {
                    let mut v = professional.to_vec();
                    for d in personal {
                        if !v.contains(d) {
                            v.push(d);
                        }
                    }
                    v
                }
            };
            domains.into_iter().map(String::from).collect()
        }
        ResearchTarget::Company => vec![
            "wikipedia.org",
            "techcrunch.com",
            "crunchbase.com",
            "trustpilot.com",
            "reddit.com",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        ResearchTarget::Market { asset_class } => match asset_class {
            AssetClass::Stock => vec![
                "reuters.com",
                "ft.com",
                "seekingalpha.com",
                "marketwatch.com",
                "investopedia.com",
                "finance.yahoo.com",
                "fool.com",
                "cnbc.com",
            ],
            AssetClass::Crypto => vec![
                "coindesk.com",
                "cointelegraph.com",
                "decrypt.co",
                "theblock.co",
                "bitcoinmagazine.com",
                "cryptoslate.com",
                "reddit.com",
            ],
            AssetClass::Macro => vec![
                "reuters.com",
                "ft.com",
                "bloomberg.com",
                "cnbc.com",
                "wsj.com",
                "economist.com",
                "marketwatch.com",
            ],
        }
        .into_iter()
        .map(String::from)
        .collect(),
    }
}

/// A single crawled source returned to the caller.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SourceEntry {
    pub url: String,
    pub title: String,
    pub snippet: String,
}

/// Close a span if tracing is active. Absorbs the `Option` dance from `run()`.
fn end_span(
    trace: &Option<ResearchTrace>,
    span: Option<SpanHandle>,
    output: serde_json::Value,
    meta: serde_json::Value,
) {
    if let (Some(t), Some(s)) = (trace, span) {
        t.end_span(s, output, meta);
    }
}

/// On `Err`, closes `*span` (recording the error) before the caller propagates
/// via `?`. On `Ok`, leaves `*span` untouched so the caller closes it themselves
/// with stage-specific output/metadata.
fn close_span_on_err<T>(
    trace: &Option<ResearchTrace>,
    span: &mut Option<SpanHandle>,
    result: Result<T>,
) -> Result<T> {
    if let Err(e) = &result {
        end_span(trace, span.take(), json!(null), json!({ "error": e.to_string() }));
    }
    result
}

/// Scope an LlmClient to a span (owned clone). Falls back to a plain clone when untraced.
fn scoped(llm: &LlmClient, span: &Option<SpanHandle>, name: &str) -> LlmClient {
    match span {
        Some(s) => llm.under(&s.id, name),
        None => llm.clone(),
    }
}

/// Short label for a research target, used in the trace name/tags.
fn target_label(target: &ResearchTarget) -> &'static str {
    match target {
        ResearchTarget::Topic => "topic",
        ResearchTarget::Person { .. } => "person",
        ResearchTarget::Company => "company",
        ResearchTarget::Market { .. } => "market",
    }
}

/// The full research pipeline:
///   query → planner → [search + scrape]×N → [summarize]×M → report
///
/// Progress events are emitted via the `on_progress` callback so callers
/// (CLI, WebSocket, SSE) can stream updates to the user.
pub async fn run(
    cfg: &Config,
    request: &ResearchRequest,
    on_progress: impl Fn(ProgressEvent),
    token_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<ResearchResult> {
    let topic = &request.topic;

    // 1. Resolve effective domains (union of profile + raw)
    let mut domains: Vec<String> = request
        .domain_profile
        .as_deref()
        .and_then(|p| cfg.profiles.get(p))
        .cloned()
        .unwrap_or_default();
    for d in &request.domains {
        if !domains.contains(d) {
            domains.push(d.clone());
        }
    }

    // Fall back to target-specific domain set when no explicit domains given
    if domains.is_empty() {
        domains = domains_for_target(&request.target);
    }

    // 2. Deep mode multipliers
    let depth = matches!(request.mode, ResearchMode::Deep);
    let max_queries = if depth {
        cfg.max_search_queries * 2
    } else {
        cfg.max_search_queries
    };
    let max_sources = if depth {
        cfg.max_sources_per_query * 2
    } else {
        cfg.max_sources_per_query
    };

    // Tracing: one trace per run (None when LANGFUSE_* unset ⇒ no-op). `trace_flush`
    // is the background flusher's handle (see `ResearchTrace::from_config`) — awaited
    // at the end of this function so queued Langfuse events are delivered before
    // `run()` returns, even for short-lived CLI invocations.
    let (trace, trace_flush) = match ResearchTrace::from_config(cfg) {
        Some((t, handle)) => (Some(t), Some(handle)),
        None => (None, None),
    };
    if let Some(t) = &trace {
        t.start(
            &format!("research:{}", target_label(&request.target)),
            json!({ "topic": topic, "domains": domains }),
            json!({
                "mode": format!("{:?}", request.mode),
                "target": target_label(&request.target),
                "profile": request.domain_profile,
                "max_queries": max_queries,
                "max_sources": max_sources,
            }),
            vec![
                format!("{:?}", request.mode).to_lowercase(),
                target_label(&request.target).to_string(),
            ],
        );
        tracing::info!(trace_id = %t.trace_id(), "langfuse trace started");
    }

    // 3. Build clients (heavy for report, fast for structured tasks)
    let llm = LlmClient::new(cfg).with_trace(trace.clone());
    let llm_fast = LlmClient::new_fast(cfg).with_trace(trace.clone());
    let http = Client::builder()
        .user_agent("Researcher/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let result: Result<ResearchResult> = async {
        // 3b. Resolve stage routing from config
        let effective_fast: Vec<String> = cfg.llm_fast_stages.clone();

        for s in &effective_fast {
            if !["planner", "summarizer", "publisher"].contains(&s.as_str()) {
                warn!(stage = %s, "unknown stage in fast_stages, ignored");
            }
        }

        let planner_llm = if effective_fast.iter().any(|s| s == "planner") { &llm_fast } else { &llm };
        let summarizer_llm = if effective_fast.iter().any(|s| s == "summarizer") { &llm_fast } else { &llm };
        let publisher_llm = if effective_fast.iter().any(|s| s == "publisher") { &llm_fast } else { &llm };

        info!(
            planner = if std::ptr::eq(planner_llm, &llm_fast) { "fast" } else { "heavy" },
            summarizer = if std::ptr::eq(summarizer_llm, &llm_fast) { "fast" } else { "heavy" },
            publisher = if std::ptr::eq(publisher_llm, &llm_fast) { "fast" } else { "heavy" },
            "stage routing"
        );

        // 4. Plan
        on_progress(ProgressEvent::Planning);
        let mut plan_span = trace.as_ref().map(|t| t.span("plan", None));
        let plan_result = generate_queries(
            &scoped(planner_llm, &plan_span, "plan"),
            topic, max_queries, &domains, &request.target,
            request.intent.as_deref(),
        ).await;
        let mut queries = close_span_on_err(&trace, &mut plan_span, plan_result)?;
        end_span(&trace, plan_span, json!(queries.clone()), json!({ "count": queries.len() }));
        on_progress(ProgressEvent::Queries(queries.clone()));

        // 5. Crawl — always build eff_cfg so we can propagate domain_profile from the request.
        //    domain_profile lives on ResearchRequest (not on the base cfg), so crawl_query
        //    needs it on the config it receives to drive backend selection.
        let mut eff_cfg = cfg.clone();
        eff_cfg.domain_profile = request.domain_profile.clone();
        if depth {
            eff_cfg.max_sources_per_query = max_sources;
        }
        let cfg_ref = &eff_cfg;
        let crawl_span = trace.as_ref().map(|t| t.span("crawl", None));
        on_progress(ProgressEvent::Crawling { total: queries.len() });
        let sources = crawl_all(&http, cfg_ref, &queries).await;
        end_span(
            &trace,
            crawl_span,
            json!(sources.iter().map(|s| &s.url).collect::<Vec<_>>()),
            json!({ "queries": queries.len(), "sources_found": sources.len() }),
        );
        info!(sources = sources.len(), "crawl complete");

        if sources.is_empty() {
            anyhow::bail!(
                "No sources found. All search backends (Brave/Tavily/Exa, SearXNG at {}, DuckDuckGo) returned no results or all pages failed to scrape.",
                cfg.searxng_url
            );
        }

        // 6. Build SourceEntry vec (used for quick-mode return and final result)
        let source_entries: Vec<SourceEntry> = sources
            .iter()
            .map(|s| SourceEntry {
                url: s.url.clone(),
                title: s.title.clone(),
                snippet: s.content.chars().take(200).collect(),
            })
            .collect();

        // 7. Quick mode short-circuit — return sources without summarizing/reporting
        if matches!(request.mode, ResearchMode::Quick) {
            on_progress(ProgressEvent::Done);
            return Ok(ResearchResult {
                report: None,
                sources: source_entries,
                queries,
            });
        }

        // 8a. Quality filter (always active)
        let qf_span = trace.as_ref().map(|t| t.span("quality-filter", None));
        on_progress(ProgressEvent::QualityFiltering { total: sources.len() });
        let n_in = sources.len();
        let quality_sources = filter_sources(sources, &request.target, cfg);
        end_span(&trace, qf_span, json!(null), json!({ "in": n_in, "out": quality_sources.len() }));
        info!(sources = quality_sources.len(), "quality filter complete");

        // 8b. Embedding dedup (if TEI configured)
        let sources = if !cfg.embed_base_url.is_empty() {
            on_progress(ProgressEvent::Deduplicating { total: quality_sources.len() });
            let embed = EmbedClient::new(&cfg.embed_base_url);
            let just_sources: Vec<ScrapedSource> = quality_sources.into_iter().map(|(s, _q)| s).collect();
            let dd_span = trace.as_ref().map(|t| t.span("dedup", None));
            let n_in = just_sources.len();
            let deduped = deduplicate(&embed, just_sources, cfg.dedup_threshold, cfg.max_sources_per_query).await;
            end_span(
                &trace,
                dd_span,
                json!(null),
                json!({ "in": n_in, "out": deduped.len(), "threshold": cfg.dedup_threshold }),
            );

            // Re-assess quality after dedup (lost annotations during dedup)
            let quality_sources: Vec<_> = deduped
                .into_iter()
                .map(|s| {
                    let q = assess_quality(&s, &request.target);
                    (s, q)
                })
                .collect();

            // 8c. Cross-encoder rerank (if reranker configured)
            if !cfg.rerank_base_url.is_empty() {
                on_progress(ProgressEvent::Reranking { total: quality_sources.len() });
                let reranker = RerankerClient::new(&cfg.rerank_base_url);
                // Clone before passing to rerank since it consumes the vec
                let fallback: Vec<ScrapedSource> = quality_sources.iter().map(|(s, _)| s.clone()).collect();
                let rr_span = trace.as_ref().map(|t| t.span("rerank", None));
                let n_in = quality_sources.len();
                match reranker.rerank(
                    topic,
                    quality_sources,
                    cfg.rerank_relevance_weight,
                    cfg.rerank_authority_weight,
                    cfg.rerank_quality_weight,
                    cfg.rerank_min_score,
                ).await {
                    Ok(ranked) => {
                        end_span(&trace, rr_span, json!(null), json!({ "in": n_in, "out": ranked.len() }));
                        on_progress(ProgressEvent::CrawlComplete { sources: ranked.len() });
                        ranked.into_iter().map(|r| r.source).collect()
                    }
                    Err(e) => {
                        end_span(&trace, rr_span, json!(null), json!({ "in": n_in, "error": e.to_string() }));
                        tracing::warn!(%e, "cross-encoder rerank failed, using dedup order");
                        on_progress(ProgressEvent::CrawlComplete { sources: fallback.len() });
                        fallback
                    }
                }
            } else {
                on_progress(ProgressEvent::CrawlComplete { sources: quality_sources.len() });
                quality_sources.into_iter().map(|(s, _)| s).collect()
            }
        } else {
            on_progress(ProgressEvent::CrawlComplete { sources: quality_sources.len() });
            quality_sources.into_iter().map(|(s, _)| s).collect()
        };

        // 9. Summarize concurrently
        info!(count = sources.len(), "sources entering summarizer (post-filter/dedup/rerank)");
        let sum_span = trace.as_ref().map(|t| t.span("summarize", None));
        on_progress(ProgressEvent::Summarizing { total: sources.len() });
        let first_summaries = summarize_all(&scoped(summarizer_llm, &sum_span, "summarize"), &sources, topic).await;
        end_span(
            &trace,
            sum_span,
            json!(null),
            json!({ "sources_in": sources.len(), "summaries_kept": first_summaries.len() }),
        );
        info!(summaries = first_summaries.len(), "summarization complete");
        on_progress(ProgressEvent::SummarizingComplete { summaries: first_summaries.len() });

        // 9b. Recovery: if all summaries were irrelevant, retry with broader queries
        let summaries = if first_summaries.is_empty() {
            warn!("all summaries empty or irrelevant — retrying with broader queries");
            on_progress(ProgressEvent::RetryingWithBroaderQueries);

            let mut retry_span = trace.as_ref().map(|t| t.span("retry", None));
            let retry_parent = retry_span.as_ref().map(|s| s.id.clone());
            let broaden_result = broaden_queries(
                &scoped(planner_llm, &retry_span, "retry.broaden"),
                topic, &queries, max_queries, &domains, &request.target,
                request.intent.as_deref(),
            ).await;
            let broader = close_span_on_err(&trace, &mut retry_span, broaden_result)?;
            queries = broader.clone();
            on_progress(ProgressEvent::Queries(broader.clone()));

            on_progress(ProgressEvent::Crawling { total: broader.len() });
            let retry_raw = crawl_all(&http, cfg_ref, &broader).await;
            info!(sources = retry_raw.len(), "retry crawl complete");

            on_progress(ProgressEvent::QualityFiltering { total: retry_raw.len() });
            let retry_quality = filter_sources(retry_raw, &request.target, cfg);

            // Skip embed dedup/rerank on retry — fewer sources, no benefit from aggressive filtering
            let retry_sources: Vec<ScrapedSource> = retry_quality.into_iter().map(|(s, _)| s).collect();
            on_progress(ProgressEvent::CrawlComplete { sources: retry_sources.len() });

            on_progress(ProgressEvent::Summarizing { total: retry_sources.len() });
            let retry_summaries = summarize_all(
                &match &retry_parent {
                    Some(id) => summarizer_llm.under(id, "retry.summarize"),
                    None => summarizer_llm.clone(),
                },
                &retry_sources,
                topic,
            ).await;
            info!(summaries = retry_summaries.len(), "retry summarization complete");
            on_progress(ProgressEvent::SummarizingComplete { summaries: retry_summaries.len() });

            if retry_summaries.is_empty() {
                end_span(
                    &trace,
                    retry_span,
                    json!(null),
                    json!({ "summaries_kept": 0, "error": "all summaries empty or irrelevant" }),
                );
                anyhow::bail!("All source summaries were empty or irrelevant.");
            }
            end_span(&trace, retry_span, json!(null), json!({ "summaries_kept": retry_summaries.len() }));
            retry_summaries
        } else {
            first_summaries
        };

        // 10. Write report (streaming if token_tx provided)
        let mut write_span = trace.as_ref().map(|t| t.span("write-report", None));
        on_progress(ProgressEvent::WritingReport);
        let write_result = write_report(
            &scoped(publisher_llm, &write_span, "write_report"),
            topic, &summaries, &request.mode, &request.target, token_tx,
        ).await;
        let raw_report = close_span_on_err(&trace, &mut write_span, write_result)?;
        end_span(&trace, write_span, json!({ "report_len": raw_report.len() }), json!({}));

        // 10b. LLM-judge scoring (background, opt-in via LLM_JUDGE_ENABLED).
        // Deliberately does NOT clone `trace` (a ResearchTrace) into the spawned
        // task — every clone shares the same per-run event channel that `run()`
        // awaits via `trace_flush` right before returning (see the flush-before-
        // return fix in this file). Holding a clone here would block that await
        // until the judge finished, reintroducing the latency this design
        // explicitly avoids. Instead we pass a plain `trace_id` string and an
        // untraced `LlmClient` (no `.with_trace(...)`), so the task is fully
        // independent of `run()`'s trace lifecycle and a judge failure can't
        // affect the returned ResearchResult.
        if cfg.judge_enabled {
            if let Some(t) = &trace {
                let trace_id = t.trace_id().to_string();
                let cfg_owned = cfg.clone();
                let judge_llm = LlmClient::new_fast(&cfg_owned);
                let topic_owned = topic.to_string();
                let report_owned = raw_report.clone();
                let summaries_owned = summaries.clone();
                tokio::spawn(async move {
                    judge::score_report(&cfg_owned, &judge_llm, &trace_id, &topic_owned, &report_owned, &summaries_owned).await;
                });
            }
        }

        let report = format_report(&raw_report, &summaries);
        on_progress(ProgressEvent::Done);

        Ok(ResearchResult {
            report: Some(report),
            sources: source_entries,
            queries,
        })
    }
    .await;

    if let Some(t) = &trace {
        let output = match &result {
            Ok(r) => json!({
                "report_len": r.report.as_ref().map(|s| s.len()),
                "queries": r.queries.len(),
                "sources": r.sources.len(),
            }),
            Err(_) => serde_json::Value::Null,
        };
        t.finalize(&result, output);
    }

    // Drop every clone of the trace's channel sender so the background flusher
    // sees the channel close, then await its handle — guarantees all queued
    // Langfuse events are sent before this function returns. Without this, a
    // short-lived CLI run can exit before the fire-and-forget events (often
    // `finalize` itself) are actually delivered.
    drop(llm);
    drop(llm_fast);
    drop(trace);
    if let Some(handle) = trace_flush {
        let _ = handle.await;
    }

    result
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ResearchResult {
    /// None in quick mode (no LLM report generated)
    pub report: Option<String>,
    pub sources: Vec<SourceEntry>,
    pub queries: Vec<String>,
}

/// Progress events emitted during research — used by CLI printer and WebSocket.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Planning,
    Queries(Vec<String>),
    Crawling { total: usize },
    QualityFiltering { total: usize },
    Deduplicating { total: usize },
    Reranking { total: usize },
    CrawlComplete { sources: usize },
    Summarizing { total: usize },
    SummarizingComplete { summaries: usize },
    WritingReport,
    RetryingWithBroaderQueries,
    Done,
}

impl std::fmt::Display for ProgressEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Planning => write!(f, "🔍 Planning research queries..."),
            Self::Queries(q) => write!(f, "📋 Generated {} search queries", q.len()),
            Self::Crawling { total } => write!(f, "🌐 Crawling {} queries in parallel...", total),
            Self::QualityFiltering { total } => write!(f, "Filtering {total} sources by quality"),
            Self::Deduplicating { total } => write!(f, "🔗 Deduplicating {} sources...", total),
            Self::Reranking { total } => write!(f, "Reranking {total} sources"),
            Self::CrawlComplete { sources } => write!(f, "✅ Scraped {} unique sources", sources),
            Self::Summarizing { total } => {
                write!(f, "🧠 Summarizing {} sources concurrently...", total)
            }
            Self::SummarizingComplete { summaries } => {
                write!(f, "✅ {} relevant summaries", summaries)
            }
            Self::WritingReport => write!(f, "📝 Writing final report..."),
            Self::RetryingWithBroaderQueries => {
                write!(f, "🔄 No results found — retrying with broader queries...")
            }
            Self::Done => write!(f, "✅ Research complete"),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::ResearchTrace;

    #[test]
    fn close_span_on_err_closes_span_with_error_before_propagating() {
        let (trace, mut rx) = ResearchTrace::for_test();
        let mut span = Some(trace.span("plan", None));
        let result: Result<u32> = close_span_on_err(
            &Some(trace),
            &mut span,
            Err(anyhow::anyhow!("boom")),
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "boom");
        assert!(span.is_none(), "span should be taken/consumed on the error path");

        let batch = rx.try_recv().expect("end_span should have queued a span-create event");
        assert_eq!(batch[0]["type"], serde_json::json!("span-create"));
        assert_eq!(batch[0]["body"]["metadata"]["error"], serde_json::json!("boom"));
    }

    #[test]
    fn close_span_on_err_leaves_span_open_on_success() {
        let (trace, mut rx) = ResearchTrace::for_test();
        let mut span = Some(trace.span("plan", None));
        let result = close_span_on_err(&Some(trace), &mut span, Ok::<_, anyhow::Error>(42));

        assert_eq!(result.unwrap(), 42);
        assert!(span.is_some(), "success path must not close the span — caller closes it with stage-specific output");
        assert!(rx.try_recv().is_err(), "no event should be queued until the caller's own end_span call");
    }
}
