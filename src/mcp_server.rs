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
use researcher::pipeline::run;

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
        use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest};

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
            "AI research agent. Given a topic, it generates search queries, \
             scrapes and summarizes sources, and produces a comprehensive \
             markdown report with citations. Backed by a local Qwen3.5-9B \
             model via llama.cpp or any OpenAI-compatible endpoint."
                .to_string(),
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
        llm_max_tokens: env_usize("LLM_MAX_TOKENS", 2048) as u32,
        llm_temperature: env_f32("LLM_TEMPERATURE", 0.3),
        strip_thinking_tokens: env_bool("STRIP_THINKING_TOKENS", true),
        searxng_url: env("SEARXNG_URL", "http://localhost:4000"),
        search_results_per_query: env_usize("SEARCH_RESULTS_PER_QUERY", 8),
        embed_base_url: env("EMBED_BASE_URL", ""),
        dedup_threshold: env_f32("DEDUP_THRESHOLD", 0.92),
        max_search_queries: env_usize("MAX_SEARCH_QUERIES", 4),
        max_sources_per_query: env_usize("MAX_SOURCES_PER_QUERY", 4),
        max_page_chars: env_usize("MAX_PAGE_CHARS", 8000),
        mode: "report".to_string(),
        domain_profile: None,
        cli_domains: vec![],
        profiles: crate::config::load_profiles(),
    }
}
