/// researcher-mcp — MCP server exposing the research pipeline as a tool.
///
/// Transport: stdio (JSON-RPC over stdin/stdout).
/// Logging:   stderr only (stdout is reserved for the MCP protocol).
///
/// Usage (Claude Desktop config):
///   {
///     "mcpServers": {
///       "researcher": {
///         "command": "/path/to/researcher-mcp",
///         "env": {
///           "LLM_BASE_URL": "http://localhost:8080/v1",
///           "SEARXNG_URL": "http://localhost:4000"
///         }
///       }
///     }
///   }
mod config;
mod embeddings;
mod jobs;
mod llm;
mod researcher;
mod scraper;
mod search;

use anyhow::Result;
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ServiceExt,
};
use serde::Deserialize;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use config::Config;


// ── Tool input schemas ────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResearchInput {
    #[schemars(description = "Research topic or question")]
    pub query: String,

    #[schemars(description = "Output mode: quick (links+snippets only), summary (bullet facts), report (full markdown, default), deep (thorough long-form)")]
    pub mode: Option<String>,

    #[schemars(description = "Named domain profile: shopping-ro, tech-news, llm-news, academic, news, travel")]
    pub domain_profile: Option<String>,

    #[schemars(description = "Raw domain list override, e.g. [\"olx.ro\", \"reddit.com/r/LocalLLaMA\"]")]
    pub domains: Option<Vec<String>>,

    #[schemars(description = "Override max search sub-queries (uses config default if omitted)")]
    pub max_queries: Option<usize>,

    #[schemars(description = "Override max sources per query (uses config default if omitted)")]
    pub max_sources: Option<usize>,

}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PersonResearchInput {
    #[schemars(description = "Full name of the person to research, e.g. 'Maria Ionescu'")]
    pub name: String,

    #[schemars(description = "Research focus: 'company' (professional background), 'personal' (interests/lifestyle), or 'both' (default)")]
    pub method: Option<String>,

}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompanyResearchInput {
    #[schemars(description = "Company name to research, e.g. 'Acme Corp'")]
    pub name: String,

    #[schemars(description = "Optional country to narrow results, e.g. 'Romania'")]
    pub country: Option<String>,

}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct JobSearchInput {
    #[schemars(description = "Job search query, e.g. 'LLM inference engineer' or 'AI research Rust'")]
    pub query: String,

    #[schemars(description = "Output mode: 'list' (ranked shortlist, default) or 'deep' (shortlist + company briefs for top 5)")]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CodeResearchInput {
    #[schemars(description = "Framework or library to research, e.g. \"axum\", \"tokio\", \"claude code\"")]
    pub framework: String,
    #[schemars(description = "Version to target, e.g. \"0.8\". Defaults to \"latest\" if omitted.")]
    pub version: Option<String>,
    #[schemars(description = "Aspects to research: bugs, changelog, community, releases. Defaults to [\"bugs\", \"changelog\", \"community\"] if omitted.")]
    pub aspects: Option<Vec<String>>,
    #[schemars(description = "GitHub repo slug, e.g. \"tokio-rs/tokio\". Anchors bug/release queries to GitHub.")]
    pub repo: Option<String>,
    #[schemars(description = "Optional keyword to narrow results, e.g. \"middleware timeout\" or \"CORS\". Appended to every search query.")]
    pub query: Option<String>,

}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MarketInsightInput {
    /// Ticker symbol (e.g. "BTC", "NVDA", "$AAPL") or free-form topic
    /// (e.g. "AI chip stocks", "Ethereum staking post-Shanghai")
    pub query: String,
    /// Asset class for domain and prompt selection.
    /// "stock" | "crypto" | "macro" (default: "macro")
    pub asset_class: Option<String>,
    /// Output depth: "quick" | "summary" | "report" (default) | "deep"
    pub mode: Option<String>,

}

// ── Server struct ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ResearcherServer {
    cfg: std::sync::Arc<Config>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ResearcherServer {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg: std::sync::Arc::new(cfg),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Web research agent.\nModes: quick=links+snippets, summary=bullet facts, report=full markdown (default), deep=thorough.\nProfiles: shopping-ro, tech-news, llm-news, academic, news, travel. Or pass domains:[\"site\"] directly.")]
    async fn research(
        &self,
        Parameters(input): Parameters<ResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};

        let mode = match input.mode.as_deref().unwrap_or("report") {
            "quick"   => ResearchMode::Quick,
            "summary" => ResearchMode::Summary,
            "deep"    => ResearchMode::Deep,
            _         => ResearchMode::Report,
        };

        let mut cfg = (*self.cfg).clone();
        if let Some(mq) = input.max_queries   { cfg.max_search_queries    = mq; }
        if let Some(ms) = input.max_sources   { cfg.max_sources_per_query = ms; }

        let request = ResearchRequest {
            topic: input.query,
            mode,
            domains: input.domains.unwrap_or_default(),
            domain_profile: input.domain_profile,
            target: ResearchTarget::default(),

        };

        match run(
            &cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await
        {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(description = "Research a person for meeting prep. Returns a markdown brief covering professional background, career, public voice, conversation hooks, and/or personal interests depending on 'method'. Sources: LinkedIn, Twitter/X, GitHub, Facebook, Instagram, Reddit, and more.")]
    async fn research_person(
        &self,
        Parameters(input): Parameters<PersonResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{
            run, ResearchMode, ResearchRequest, ResearchTarget, PersonMethod,
        };

        let method: PersonMethod = input.method
            .as_deref()
            .unwrap_or("both")
            .parse()
            .unwrap_or(PersonMethod::Both);

        let request = ResearchRequest {
            topic: input.name,
            mode: ResearchMode::Report,
            domains: vec![],
            domain_profile: None,
            target: ResearchTarget::Person { method },

        };

        match run(
            &self.cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(description = "Research a company for meeting prep. Returns a markdown brief covering what they do, size & stage, recent news, culture, and strategic context. Sources: LinkedIn, Crunchbase, Bloomberg, Glassdoor, Trustpilot, Wikipedia, and news.")]
    async fn research_company(
        &self,
        Parameters(input): Parameters<CompanyResearchInput>,
    ) -> String {
        use crate::researcher::pipeline::{
            run, ResearchMode, ResearchRequest, ResearchTarget,
        };

        let topic = match &input.country {
            Some(c) => format!("{} {}", input.name, c),
            None    => input.name.clone(),
        };

        let request = ResearchRequest {
            topic,
            mode: ResearchMode::Report,
            domains: vec![],
            domain_profile: None,
            target: ResearchTarget::Company,

        };

        match run(
            &self.cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
    }

    #[tool(description = "Search for remote AI engineering jobs matching your profile in profiles.toml [job-profile]. Returns a ranked markdown report. mode='list' for a quick shortlist (default), mode='deep' for full inline company briefs on top 5 matches. Sources: Remotive, Adzuna (if ADZUNA_APP_ID/KEY set), SearXNG.")]
    async fn search_jobs(
        &self,
        Parameters(input): Parameters<JobSearchInput>,
    ) -> String {
        use crate::jobs::fetcher::fetch_jobs;
        use crate::jobs::scorer::score_listings;
        use crate::jobs::publisher::write_job_report;
        use crate::llm::client::LlmClient;

        let profile = match &self.cfg.job_profile {
            Some(p) => p.clone(),
            None => return "Error: no [job-profile] section found in profiles.toml. \
                            Add one to enable job search.".to_string(),
        };

        let deep = input.mode.as_deref() == Some("deep");

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let llm = LlmClient::new(&self.cfg);

        let listings = fetch_jobs(&http, &self.cfg, &input.query, &profile).await;

        let scored = match score_listings(&llm, &listings, &profile, 6).await {
            Ok(s) => s,
            Err(e) => return format!("Error scoring listings: {e:#}"),
        };

        match write_job_report(&self.cfg, &scored, &input.query, deep).await {
            Ok(report) => report,
            Err(e) => format!("Error writing report: {e:#}"),
        }
    }


    #[tool(description = "Research a framework or library: known bugs, changelogs, breaking changes, \
releases, and community sentiment. \
aspects: bugs, changelog, community, releases — pass one or more (default: bugs+changelog+community). \
query: optional keyword to narrow results, e.g. \"middleware timeout\" or \"CORS\". \
repo: GitHub slug e.g. 'tokio-rs/tokio' — anchors bug/release queries to GitHub issues/releases. \
version: e.g. '0.8' — defaults to 'latest'. \
Sources: GitHub Issues, Reddit, Hacker News, official changelogs.")]
    async fn research_code(
        &self,
        Parameters(input): Parameters<CodeResearchInput>,
    ) -> String {
        use crate::researcher::crawler::crawl_all;
        use crate::researcher::summarizer::summarize_all;
        use crate::researcher::publisher::write_code_report;
        use crate::llm::client::LlmClient;

        let version = input.version.as_deref().unwrap_or("latest");
        let framework = &input.framework;
        let aspects = input.aspects.unwrap_or_else(|| {
            vec!["bugs".to_string(), "changelog".to_string(), "community".to_string()]
        });

        // Optional keyword narrows every query (e.g. "middleware timeout")
        let q = input.query.as_deref().unwrap_or("").trim().to_string();
        let q_suffix = if q.is_empty() { String::new() } else { format!(" {q}") };

        // Build query list from templates — no LLM planner call
        let mut queries: Vec<String> = Vec::new();
        for aspect in &aspects {
            match aspect.as_str() {
                "bugs" => {
                    queries.push(format!("{framework} {version}{q_suffix} bug issue"));
                    if let Some(repo) = &input.repo {
                        queries.push(format!("{framework} {version}{q_suffix} issue site:github.com/{repo}/issues"));
                    }
                }
                "changelog" => {
                    queries.push(format!("{framework} {version}{q_suffix} changelog release notes"));
                    queries.push(format!("{framework} {version}{q_suffix} breaking changes"));
                }
                "community" => {
                    queries.push(format!("{framework} {version}{q_suffix} site:reddit.com"));
                    queries.push(format!("{framework} {version}{q_suffix} site:news.ycombinator.com"));
                }
                "releases" => {
                    queries.push(format!("{framework} {version}{q_suffix} release"));
                    if let Some(repo) = &input.repo {
                        queries.push(format!("{framework} {version}{q_suffix} site:github.com/{repo}/releases"));
                    }
                }
                _ => {} // silently skip unknown aspects
            }
        }

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let llm = LlmClient::new(&self.cfg);
        let topic = format!("{framework} {version}{q_suffix}");

        let sources = crawl_all(&http, &self.cfg, &queries).await;
        if sources.is_empty() {
            return format!("Error: no sources found for {framework} {version}{q_suffix}");
        }

        let summaries = summarize_all(&llm, &sources, &topic).await;
        if summaries.is_empty() {
            return format!("Error: could not summarize sources for {framework} {version}{q_suffix}");
        }

        match write_code_report(&llm, &summaries, framework, version, &aspects).await {
            Ok(report) => report,
            Err(e)     => format!("Error writing report: {e:#}"),
        }
    }

    #[tool(description = "Market insight research: stocks, crypto, and macro. \
        query: ticker (BTC, NVDA) or topic (AI chip stocks). \
        asset_class: stock | crypto | macro (default: macro). \
        mode: quick | summary | report (default) | deep.")]
    async fn market_insight(
        &self,
        Parameters(input): Parameters<MarketInsightInput>,
    ) -> String {
        use crate::researcher::pipeline::{
            run, AssetClass, ResearchMode, ResearchRequest, ResearchTarget,
        };

        let asset_class: AssetClass = input
            .asset_class
            .as_deref()
            .unwrap_or("macro")
            .parse()
            .unwrap_or_default();

        let mode: ResearchMode = input
            .mode
            .as_deref()
            .unwrap_or("report")
            .parse()
            .unwrap_or_default();

        let target = ResearchTarget::Market { asset_class };
        let domains = crate::researcher::pipeline::domains_for_target(&target);

        let request = ResearchRequest {
            topic: input.query,
            mode,
            domains,
            domain_profile: None,
            target,

        };

        match run(
            &self.cfg,
            &request,
            |ev| eprintln!("[researcher] {ev}"),
            None,
        ).await {
            Ok(r)  => serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string()),
            Err(e) => format!("Error: {e:#}"),
        }
    }

}

#[tool_handler]
impl ServerHandler for ResearcherServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(
            "AI research agent — 6 tools:\n\
             \n\
             • research(query, mode?, domain_profile?, domains?, max_queries?, max_sources?)\n\
               General web research. Modes: quick=snippets, summary=bullets, report=full markdown (default), deep=thorough.\n\
               Profiles: shopping-ro, tech-news, llm-news, academic, news, travel. Or pass domains:[\"example.com\"] to pin sites.\n\
             \n\
             • research_person(name, method?)\n\
               Meeting prep brief on any person. Covers career, public voice, interests, conversation hooks.\n\
               method: professional | personal | both (default). Sources: LinkedIn, X, GitHub, Facebook, Instagram, Reddit.\n\
             \n\
             • research_company(name, country?)\n\
               Meeting prep brief on a company. Covers what they do, size/stage, news, culture, strategy.\n\
               Sources: LinkedIn, Crunchbase, Bloomberg, Glassdoor, Trustpilot, Wikipedia.\n\
             \n\
             • search_jobs(query, mode?)\n\
               Find remote AI engineering jobs matching your profiles.toml [job-profile].\n\
               mode: list=shortlist (default), deep=full company briefs on top 5. Sources: Remotive, Adzuna, SearXNG.\n\
             \n\
             • research_code(framework, version?, aspects?, repo?, query?)\n\
               Research a library/framework: bugs, breaking changes, releases, community sentiment.\n\
               aspects: bugs | changelog | community | releases (default: bugs+changelog+community).\n\
               query: keyword to narrow, e.g. \"middleware timeout\". repo: GitHub slug e.g. \"tokio-rs/axum\".\n\
             \n\
             • market_insight(query, asset_class?, mode?)\n\
               Stock, crypto, and macro market research. Web research only — no price APIs.\n\
               query: ticker (BTC, NVDA, $AAPL) or topic (AI chip stocks, Ethereum staking).\n\
               asset_class: stock | crypto | macro (default: macro).\n\
               mode: quick | summary | report (default) | deep.\n\
             ".to_string(),
        )
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // All logging to stderr — stdout is the MCP protocol channel
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .init();

    // Config from env vars (no --server or --query flags needed in MCP mode)
    let cfg = Config {
        server: false,
        query: None,
        output: None,
        ..config_from_env()
    };

    tracing::info!(
        llm_base_url = %cfg.llm_base_url,
        llm_fast_base_url = %cfg.llm_fast_base_url,
        llm_fast_model = %cfg.llm_fast_model,
        llm_fast_stages = ?cfg.llm_fast_stages,
        searxng_url = %cfg.searxng_url,
        model = %cfg.llm_model,
        "researcher-mcp starting"
    );

    let service = ResearcherServer::new(cfg)
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP serve error: {e:?}"))?;

    service.waiting().await?;
    Ok(())
}

/// Build Config from environment variables with sensible MCP defaults.
/// Avoids clap parsing (no argv in MCP context).
fn config_from_env() -> Config {
    fn env(key: &str, default: &str) -> String {
        std::env::var(key).unwrap_or_else(|_| default.to_string())
    }
    fn env_usize(key: &str, default: usize) -> usize {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    fn env_f32(key: &str, default: f32) -> f32 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    fn env_bool(key: &str, default: bool) -> bool {
        std::env::var(key)
            .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(default)
    }

    Config {
        query: None,
        server: false,
        bind_addr: String::new(),
        output: None,
        llm_base_url: env("LLM_BASE_URL", "http://localhost:8080/v1"),
        llm_model: env("LLM_MODEL", "Qwen_Qwen3.5-9B-Q4_K_M"),
        llm_api_key: env("LLM_API_KEY", "no-key-needed"),
        llm_max_tokens: env_usize("LLM_MAX_TOKENS", 4096) as u32,
        llm_temperature: env_f32("LLM_TEMPERATURE", 0.3),
        strip_thinking_tokens: env_bool("STRIP_THINKING_TOKENS", true),
        llm_fast_base_url: env("LLM_FAST_BASE_URL", ""),
        llm_fast_model: env("LLM_FAST_MODEL", "Qwen3.5-4B-Q4_K_M"),
        llm_fast_api_key: env("LLM_FAST_API_KEY", ""),
        llm_fast_max_tokens: env_usize("LLM_FAST_MAX_TOKENS", 4096) as u32,
        llm_fast_stages: env("LLM_FAST_STAGES", "planner,summarizer,publisher")
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        searxng_url: env("SEARXNG_URL", "http://localhost:4000"),
        search_results_per_query: env_usize("SEARCH_RESULTS_PER_QUERY", 8),
        embed_base_url: env("EMBED_BASE_URL", ""),
        dedup_threshold: env_f32("DEDUP_THRESHOLD", 0.92),
        rerank_base_url: env("RERANK_BASE_URL", ""),
        rerank_relevance_weight: env_f32("RERANK_RELEVANCE_WEIGHT", 0.7),
        rerank_authority_weight: env_f32("RERANK_AUTHORITY_WEIGHT", 0.2),
        rerank_quality_weight: env_f32("RERANK_QUALITY_WEIGHT", 0.1),
        rerank_min_score: env_f32("RERANK_MIN_SCORE", -5.0),
        min_content_words: env_usize("MIN_CONTENT_WORDS", 100),
        min_text_density: env_f32("MIN_TEXT_DENSITY", 0.05),
        max_search_queries: env_usize("MAX_SEARCH_QUERIES", 4),
        max_sources_per_query: env_usize("MAX_SOURCES_PER_QUERY", 4),
        max_page_chars: env_usize("MAX_PAGE_CHARS", 8000),
        mode: "report".to_string(),
        domain_profile: None,
        cli_domains: vec![],
        profiles: crate::config::load_profiles(),
        auth: config::AuthConfig {
            linkedin_cookie:  std::env::var("LINKEDIN_COOKIE").ok(),
            fb_cookie:        std::env::var("FB_COOKIE").ok(),
            instagram_cookie: std::env::var("INSTAGRAM_COOKIE").ok(),
            twitter_cookie:   std::env::var("TWITTER_COOKIE").ok(),
        },
        job_profile: config::load_job_profile(),
    }
}
