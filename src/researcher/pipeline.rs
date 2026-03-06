use anyhow::Result;
use reqwest::Client;
use tracing::info;

use crate::config::Config;
use crate::embeddings::client::EmbedClient;
use crate::embeddings::dedup::{deduplicate, rank_by_relevance};
use crate::llm::client::LlmClient;
use super::crawler::crawl_all;
use super::planner::generate_queries;
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

#[derive(Debug, Clone)]
pub enum ResearchTarget {
    Topic,
    Person { method: PersonMethod },
    Company,
}

impl Default for ResearchTarget {
    fn default() -> Self { ResearchTarget::Topic }
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
    match target {
        ResearchTarget::Topic => vec![],
        ResearchTarget::Person { method } => {
            let professional: &[&str] = &[
                "linkedin.com", "twitter.com", "x.com", "github.com",
                "medium.com", "scholar.google.com",
            ];
            let personal: &[&str] = &[
                "facebook.com", "instagram.com", "twitter.com", "x.com",
                "reddit.com", "tiktok.com",
            ];
            let domains: Vec<&str> = match method {
                PersonMethod::Company  => professional.to_vec(),
                PersonMethod::Personal => personal.to_vec(),
                PersonMethod::Both => {
                    let mut v = professional.to_vec();
                    v.extend_from_slice(personal);
                    v
                }
            };
            domains.into_iter().map(String::from).collect()
        }
        ResearchTarget::Company => vec![
            "linkedin.com", "crunchbase.com", "bloomberg.com",
            "glassdoor.com", "trustpilot.com", "wikipedia.org",
        ].into_iter().map(String::from).collect(),
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

    // 3. Build clients
    let llm = LlmClient::new(cfg);
    let http = Client::builder()
        .user_agent("Researcher/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // 4. Plan
    on_progress(ProgressEvent::Planning);
    let queries = generate_queries(&llm, topic, max_queries, &domains, &request.target).await?;
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
            "No sources scraped. Check SearXNG is reachable at {}",
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

    // 8. Embedding dedup (if TEI configured)
    let sources = if !cfg.embed_base_url.is_empty() {
        on_progress(ProgressEvent::Deduplicating { total: sources.len() });
        let embed = EmbedClient::new(&cfg.embed_base_url);
        let deduped = deduplicate(&embed, sources, cfg.dedup_threshold).await;
        let ranked = rank_by_relevance(&embed, topic, deduped).await;
        on_progress(ProgressEvent::CrawlComplete { sources: ranked.len() });
        ranked
    } else {
        on_progress(ProgressEvent::CrawlComplete { sources: sources.len() });
        sources
    };

    // 9. Summarize concurrently
    on_progress(ProgressEvent::Summarizing { total: sources.len() });
    let summaries = summarize_all(&llm, &sources, topic).await;
    info!(summaries = summaries.len(), "summarization complete");
    on_progress(ProgressEvent::SummarizingComplete { summaries: summaries.len() });

    if summaries.is_empty() {
        anyhow::bail!("All source summaries were empty or irrelevant.");
    }

    // 10. Write report (streaming if token_tx provided)
    on_progress(ProgressEvent::WritingReport);
    let raw_report = write_report(&llm, topic, &summaries, &request.mode, &request.target, token_tx).await?;
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
    Deduplicating { total: usize },
    CrawlComplete { sources: usize },
    Summarizing { total: usize },
    SummarizingComplete { summaries: usize },
    WritingReport,
    Done,
}

impl std::fmt::Display for ProgressEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Planning => write!(f, "🔍 Planning research queries..."),
            Self::Queries(q) => write!(f, "📋 Generated {} search queries", q.len()),
            Self::Crawling { total } => write!(f, "🌐 Crawling {} queries in parallel...", total),
            Self::Deduplicating { total } => write!(f, "🔗 Deduplicating {} sources...", total),
            Self::CrawlComplete { sources } => write!(f, "✅ Scraped {} unique sources", sources),
            Self::Summarizing { total } => write!(f, "🧠 Summarizing {} sources concurrently...", total),
            Self::SummarizingComplete { summaries } => write!(f, "✅ {} relevant summaries", summaries),
            Self::WritingReport => write!(f, "📝 Writing final report..."),
            Self::Done => write!(f, "✅ Research complete"),
        }
    }
}
