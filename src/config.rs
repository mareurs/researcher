use clap::Parser;

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub linkedin_cookie:  Option<String>,
    pub fb_cookie:        Option<String>,
    pub instagram_cookie: Option<String>,
    pub twitter_cookie:   Option<String>,
}

impl AuthConfig {
    /// Return the Cookie header value for the given hostname, if configured.
    pub fn cookie_for_host(&self, host: &str) -> Option<&str> {
        if host.contains("linkedin.com") {
            self.linkedin_cookie.as_deref()
        } else if host.contains("facebook.com") {
            self.fb_cookie.as_deref()
        } else if host.contains("instagram.com") {
            self.instagram_cookie.as_deref()
        } else if host.contains("twitter.com") || host == "x.com" || host.ends_with(".x.com") {
            self.twitter_cookie.as_deref()
        } else {
            None
        }
    }
}


/// Researcher config — all values can come from env vars or CLI flags.
#[derive(Debug, Clone, Parser)]
#[command(name = "researcher", about = "Fast AI research agent")]
pub struct Config {
    /// Research query (CLI mode)
    #[arg(short, long, env = "RESEARCH_QUERY")]
    pub query: Option<String>,

    /// Run as HTTP API server instead of one-shot CLI
    #[arg(long, env = "SERVER_MODE", default_value = "false")]
    pub server: bool,

    /// Server bind address
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:3000")]
    pub bind_addr: String,

    // ── LLM ──────────────────────────────────────────────────────────────────
    /// OpenAI-compatible base URL (llama.cpp, vLLM, Ollama, or api.openai.com)
    #[arg(long, env = "LLM_BASE_URL", default_value = "http://localhost:8080/v1")]
    pub llm_base_url: String,

    /// Model name/id to pass in chat completion requests
    #[arg(long, env = "LLM_MODEL", default_value = "Qwen_Qwen3.5-9B-Q4_K_M")]
    pub llm_model: String,

    /// API key (use "no-key-needed" for local servers)
    #[arg(long, env = "LLM_API_KEY", default_value = "no-key-needed")]
    pub llm_api_key: String,

    /// Max tokens for summary/report responses
    #[arg(long, env = "LLM_MAX_TOKENS", default_value = "4096")]
    pub llm_max_tokens: u32,

    /// Temperature for LLM responses
    #[arg(long, env = "LLM_TEMPERATURE", default_value = "0.3")]
    pub llm_temperature: f32,

    /// Strip <think>...</think> tokens from Qwen3/thinking model responses
    #[arg(long, env = "STRIP_THINKING_TOKENS", default_value = "true")]
    pub strip_thinking_tokens: bool,

    // ── Fast LLM backend (structured tasks: planner, summarizer) ────────────

    /// Base URL for the fast/lightweight LLM backend (structured tasks).
    /// Empty string = fall back to LLM_BASE_URL.
    #[arg(long, env = "LLM_FAST_BASE_URL", default_value = "")]
    pub llm_fast_base_url: String,

    /// Model name for the fast LLM backend
    #[arg(long, env = "LLM_FAST_MODEL", default_value = "Qwen3.5-4B-Q4_K_M")]
    pub llm_fast_model: String,

    /// API key for the fast LLM backend. Empty = fall back to LLM_API_KEY.
    #[arg(long, env = "LLM_FAST_API_KEY", default_value = "")]
    pub llm_fast_api_key: String,

    /// Max tokens for fast LLM responses
    #[arg(long, env = "LLM_FAST_MAX_TOKENS", default_value = "4096")]
    pub llm_fast_max_tokens: u32,

    /// Pipeline stages that use the fast LLM backend (comma-separated).
    /// Valid: planner, summarizer, publisher. Default: planner,summarizer.
    #[arg(long, env = "LLM_FAST_STAGES", value_delimiter = ',', default_values_t = vec!["planner".to_string(), "summarizer".to_string(), "publisher".to_string()])]
    pub llm_fast_stages: Vec<String>,

    // ── Search ───────────────────────────────────────────────────────────────
    /// SearXNG base URL
    #[arg(long, env = "SEARXNG_URL", default_value = "http://localhost:4000")]
    pub searxng_url: String,

    /// Number of search results to fetch per sub-question
    #[arg(long, env = "SEARCH_RESULTS_PER_QUERY", default_value = "8")]
    pub search_results_per_query: usize,

    /// Google Custom Search API key (empty = disabled)
    #[arg(long, env = "GOOGLE_API_KEY", default_value = "")]
    pub google_api_key: String,

    /// Google Custom Search Engine ID / cx (empty = disabled)
    #[arg(long, env = "GOOGLE_CSE_ID", default_value = "")]
    pub google_cse_id: String,

    // ── Embeddings / dedup ───────────────────────────────────────────────────
    /// TEI embedding service base URL (empty = disable dedup)
    #[arg(long, env = "EMBED_BASE_URL", default_value = "")]
    pub embed_base_url: String,

    /// Cosine similarity threshold above which a source is considered duplicate
    #[arg(long, env = "DEDUP_THRESHOLD", default_value = "0.92")]
    pub dedup_threshold: f32,

    // ── Reranker ─────────────────────────────────────────────────────────────
    /// TEI cross-encoder reranker base URL (empty = disable reranking)
    #[arg(long, env = "RERANK_BASE_URL", default_value = "")]
    pub rerank_base_url: String,

    /// Weight for cross-encoder relevance score in combined ranking
    #[arg(long, env = "RERANK_RELEVANCE_WEIGHT", default_value = "0.7")]
    pub rerank_relevance_weight: f32,

    /// Weight for domain authority in combined ranking
    #[arg(long, env = "RERANK_AUTHORITY_WEIGHT", default_value = "0.2")]
    pub rerank_authority_weight: f32,

    /// Weight for content quality heuristics in combined ranking
    #[arg(long, env = "RERANK_QUALITY_WEIGHT", default_value = "0.1")]
    pub rerank_quality_weight: f32,

    /// Minimum raw cross-encoder relevance score to keep a source (logit scale).
    /// Sources below this are dropped after reranking. -5.0 drops clearly off-topic results.
    #[arg(long, env = "RERANK_MIN_SCORE", default_value = "-5.0")]
    pub rerank_min_score: f32,

    // ── Quality filter ───────────────────────────────────────────────────────
    /// Minimum word count for a source to pass quality filter
    #[arg(long, env = "MIN_CONTENT_WORDS", default_value = "100")]
    pub min_content_words: usize,

    /// Minimum text/HTML density ratio for quality filter
    #[arg(long, env = "MIN_TEXT_DENSITY", default_value = "0.05")]
    pub min_text_density: f32,

    // ── Research pipeline ────────────────────────────────────────────────────
    /// Max sub-questions the planner generates
    #[arg(long, env = "MAX_SEARCH_QUERIES", default_value = "4")]
    pub max_search_queries: usize,

    /// Max sources to scrape per sub-question
    #[arg(long, env = "MAX_SOURCES_PER_QUERY", default_value = "4")]
    pub max_sources_per_query: usize,

    /// Max characters to keep from a scraped page before summarizing
    #[arg(long, env = "MAX_PAGE_CHARS", default_value = "8000")]
    pub max_page_chars: usize,

    /// Save the final report to a file (markdown)
    #[arg(short, long, env = "OUTPUT_FILE")]
    pub output: Option<std::path::PathBuf>,

    // ── Mode / domain filtering ───────────────────────────────────────────────
    /// Research mode for CLI: quick, summary, report (default), deep
    #[clap(long, env = "RESEARCH_MODE", default_value = "report")]
    pub mode: String,

    /// Named domain profile from profiles.toml
    #[clap(long, env = "DOMAIN_PROFILE")]
    pub domain_profile: Option<String>,

    /// Comma-separated domains to restrict search to
    #[clap(long, value_delimiter = ',', env = "DOMAINS")]
    pub cli_domains: Vec<String>,

    /// Domain profiles loaded from profiles.toml at startup. Not a CLI flag.
    #[clap(skip)]
    pub profiles: std::collections::HashMap<String, Vec<String>>,

    /// Per-platform authentication cookies. Not a CLI flag.
    #[clap(skip)]
    pub auth: AuthConfig,

    /// Job search profile loaded from profiles.toml [job-profile]. Not a CLI flag.
    #[clap(skip)]
    pub job_profile: Option<JobProfile>,
}

/// User profile for job search, loaded from the `[job-profile]` section of `profiles.toml`.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct JobProfile {
    pub title: String,
    pub seniority: String,
    pub salary_floor: String,
    #[serde(default)]
    pub remote_only: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub preferred_company_size: String,
    #[serde(default)]
    pub avoid_industries: Vec<String>,
    #[serde(default)]
    pub about_me: String,
}

/// Load the `[job-profile]` section from `profiles.toml`.
/// Returns `None` if the file is missing, the section is absent, or it fails to parse.
pub fn load_job_profile() -> Option<JobProfile> {
    let content = match std::fs::read_to_string("profiles.toml") {
        Ok(c) => c,
        Err(_) => {
            tracing::debug!("profiles.toml not found — job profile unavailable");
            return None;
        }
    };
    let table: toml::Table = toml::from_str(&content).ok()?;
    if !table.contains_key("job-profile") {
        tracing::debug!("no [job-profile] section in profiles.toml — job search disabled");
        return None;
    }
    let section = table.get("job-profile")?;
    toml::Value::try_into(section.clone())
        .inspect_err(|e| tracing::warn!(error = %e, "failed to parse [job-profile] section"))
        .ok()
}


/// Load domain profiles from `profiles.toml` in the current directory.
/// Returns empty map if the file is missing or malformed.
pub fn load_profiles() -> std::collections::HashMap<String, Vec<String>> {
    let Ok(content) = std::fs::read_to_string("profiles.toml") else {
        return Default::default();
    };
    let table: toml::Table = match toml::from_str(&content) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("profiles.toml parse failed: {e} — using empty profiles");
            return Default::default();
        }
    };
    let mut profiles = std::collections::HashMap::new();
    for (key, value) in table {
        // Skip non-domain-profile sections (e.g. [job-profile])
        if let Some(domains_val) = value.get("domains") {
            if let Ok(domains) = domains_val.clone().try_into::<Vec<String>>() {
                profiles.insert(key, domains);
            }
        }
    }
    profiles
}
