use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

use crate::config::Config;
use crate::embeddings::client::EmbedClient;
use crate::embeddings::dedup::deduplicate;
use crate::embeddings::reranker::RerankerClient;
use crate::researcher::quality::{assess_quality, filter_sources};
use crate::llm::client::LlmClient;
use super::crawler::{crawl_all, ScrapedSource};
use super::planner::{broaden_queries, generate_queries};
use super::publisher::{format_report, write_report};
use super::summarizer::summarize_all;

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
            "quick"   => Ok(ResearchMode::Quick),
            "summary" => Ok(ResearchMode::Summary),
            "deep"    => Ok(ResearchMode::Deep),
            _         => Ok(ResearchMode::Report),
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
            "company"  => Ok(PersonMethod::Company),
            "personal" => Ok(PersonMethod::Personal),
            _          => Ok(PersonMethod::Both),
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
            "stock"  => Ok(AssetClass::Stock),
            "crypto" => Ok(AssetClass::Crypto),
            _        => Ok(AssetClass::Macro),
        }
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum ResearchTarget {
    #[default]
    Topic,
    Person { method: PersonMethod },
    Company,
    Market { asset_class: AssetClass },
}

pub struct ResearchRequest {
    pub topic: String,
    pub mode: ResearchMode,
    /// Raw domain list (e.g. ["olx.ro", "reddit.com"])
    pub domains: Vec<String>,
    /// Named profile from profiles.toml (e.g. "shopping-ro")
    pub domain_profile: Option<String>,
    pub target: ResearchTarget,
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
                "github.com", "wikipedia.org", "medium.com",
                "news.ycombinator.com", "youtube.com",
            ];
            let personal: &[&str] = &[
                "reddit.com", "youtube.com", "wikipedia.org",
            ];
            let domains: Vec<&str> = match method {
                PersonMethod::Company  => professional.to_vec(),
                PersonMethod::Personal => personal.to_vec(),
                PersonMethod::Both => {
                    let mut v = professional.to_vec();
                    for d in personal {
                        if !v.contains(d) { v.push(d); }
                    }
                    v
                }
            };
            domains.into_iter().map(String::from).collect()
        }
        ResearchTarget::Company => vec![
            "wikipedia.org", "techcrunch.com", "crunchbase.com",
            "trustpilot.com", "reddit.com",
        ].into_iter().map(String::from).collect(),
        ResearchTarget::Market { asset_class } => match asset_class {
            AssetClass::Stock => vec![
                "reuters.com", "ft.com", "seekingalpha.com", "marketwatch.com",
                "investopedia.com", "finance.yahoo.com", "fool.com", "cnbc.com",
            ],
            AssetClass::Crypto => vec![
                "coindesk.com", "cointelegraph.com", "decrypt.co", "theblock.co",
                "bitcoinmagazine.com", "cryptoslate.com", "reddit.com",
            ],
            AssetClass::Macro => vec![
                "reuters.com", "ft.com", "bloomberg.com", "cnbc.com",
                "wsj.com", "economist.com", "marketwatch.com",
            ],
        }.into_iter().map(String::from).collect(),
    }
}

/// A single crawled source returned to the caller.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SourceEntry {
    pub url: String,
    pub title: String,
    pub snippet: String,
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
    let max_queries = if depth { cfg.max_search_queries * 2 } else { cfg.max_search_queries };
    let max_sources = if depth { cfg.max_sources_per_query * 2 } else { cfg.max_sources_per_query };

    // 3. Build clients (heavy for report, fast for structured tasks)
    let llm = LlmClient::new(cfg);
    let llm_fast = LlmClient::new_fast(cfg);
    let http = Client::builder()
        .user_agent("Researcher/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

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
    let mut queries = generate_queries(planner_llm, topic, max_queries, &domains, &request.target).await?;
    on_progress(ProgressEvent::Queries(queries.clone()));

    // 5. Crawl (deep mode uses overridden max_sources_per_query)
    let mut eff_cfg;
    let cfg_ref = if depth {
        eff_cfg = cfg.clone();
        eff_cfg.max_sources_per_query = max_sources;
        &eff_cfg
    } else {
        cfg
    };
    on_progress(ProgressEvent::Crawling { total: queries.len() });
    let sources = crawl_all(&http, cfg_ref, &queries).await;
    info!(sources = sources.len(), "crawl complete");

    if sources.is_empty() {
        anyhow::bail!(
            "No sources found. All search backends (Vertex AI, SearXNG at {}, DuckDuckGo) returned no results or all pages failed to scrape.",
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
    on_progress(ProgressEvent::QualityFiltering { total: sources.len() });
    let quality_sources = filter_sources(sources, &request.target, cfg);
    info!(sources = quality_sources.len(), "quality filter complete");

    // 8b. Embedding dedup (if TEI configured)
    let sources = if !cfg.embed_base_url.is_empty() {
        on_progress(ProgressEvent::Deduplicating { total: quality_sources.len() });
        let embed = EmbedClient::new(&cfg.embed_base_url);
        let just_sources: Vec<ScrapedSource> = quality_sources.into_iter().map(|(s, _q)| s).collect();
        let deduped = deduplicate(&embed, just_sources, cfg.dedup_threshold, cfg.max_sources_per_query).await;

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
            match reranker.rerank(
                topic,
                quality_sources,
                cfg.rerank_relevance_weight,
                cfg.rerank_authority_weight,
                cfg.rerank_quality_weight,
                cfg.rerank_min_score,
            ).await {
                Ok(ranked) => {
                    on_progress(ProgressEvent::CrawlComplete { sources: ranked.len() });
                    ranked.into_iter().map(|r| r.source).collect()
                }
                Err(e) => {
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
    on_progress(ProgressEvent::Summarizing { total: sources.len() });
    let first_summaries = summarize_all(summarizer_llm, &sources, topic).await;
    info!(summaries = first_summaries.len(), "summarization complete");
    on_progress(ProgressEvent::SummarizingComplete { summaries: first_summaries.len() });

    // 9b. Recovery: if all summaries were irrelevant, retry with broader queries
    let summaries = if first_summaries.is_empty() {
        warn!("all summaries empty or irrelevant — retrying with broader queries");
        on_progress(ProgressEvent::RetryingWithBroaderQueries);

        let broader = broaden_queries(planner_llm, topic, &queries, max_queries, &domains, &request.target).await?;
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
        let retry_summaries = summarize_all(summarizer_llm, &retry_sources, topic).await;
        info!(summaries = retry_summaries.len(), "retry summarization complete");
        on_progress(ProgressEvent::SummarizingComplete { summaries: retry_summaries.len() });

        if retry_summaries.is_empty() {
            anyhow::bail!("All source summaries were empty or irrelevant.");
        }
        retry_summaries
    } else {
        first_summaries
    };

    // 10. Write report (streaming if token_tx provided)
    on_progress(ProgressEvent::WritingReport);
    let raw_report = write_report(publisher_llm, topic, &summaries, &request.mode, &request.target, token_tx).await?;
    let report = format_report(&raw_report, &summaries);
    on_progress(ProgressEvent::Done);

    Ok(ResearchResult {
        report: Some(report),
        sources: source_entries,
        queries,
    })
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
            Self::Summarizing { total } => write!(f, "🧠 Summarizing {} sources concurrently...", total),
            Self::SummarizingComplete { summaries } => write!(f, "✅ {} relevant summaries", summaries),
            Self::WritingReport => write!(f, "📝 Writing final report..."),
            Self::RetryingWithBroaderQueries => write!(f, "🔄 No results found — retrying with broader queries..."),
            Self::Done => write!(f, "✅ Research complete"),
        }
    }
}
