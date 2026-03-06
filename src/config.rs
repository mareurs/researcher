use clap::Parser;

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
    #[arg(long, env = "LLM_MAX_TOKENS", default_value = "2048")]
    pub llm_max_tokens: u32,

    /// Temperature for LLM responses
    #[arg(long, env = "LLM_TEMPERATURE", default_value = "0.3")]
    pub llm_temperature: f32,

    /// Strip <think>...</think> tokens from Qwen3/thinking model responses
    #[arg(long, env = "STRIP_THINKING_TOKENS", default_value = "true")]
    pub strip_thinking_tokens: bool,

    // ── Search ───────────────────────────────────────────────────────────────
    /// SearXNG base URL
    #[arg(long, env = "SEARXNG_URL", default_value = "http://localhost:4000")]
    pub searxng_url: String,

    /// Number of search results to fetch per sub-question
    #[arg(long, env = "SEARCH_RESULTS_PER_QUERY", default_value = "8")]
    pub search_results_per_query: usize,

    // ── Embeddings / dedup ───────────────────────────────────────────────────
    /// TEI embedding service base URL (empty = disable dedup)
    #[arg(long, env = "EMBED_BASE_URL", default_value = "")]
    pub embed_base_url: String,

    /// Cosine similarity threshold above which a source is considered duplicate
    #[arg(long, env = "DEDUP_THRESHOLD", default_value = "0.92")]
    pub dedup_threshold: f32,

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
}

/// Load domain profiles from `profiles.toml` in the current directory.
/// Returns empty map if the file is missing or malformed.
pub fn load_profiles() -> std::collections::HashMap<String, Vec<String>> {
    #[derive(serde::Deserialize)]
    struct ProfileEntry {
        domains: Vec<String>,
    }

    let Ok(content) = std::fs::read_to_string("profiles.toml") else {
        return Default::default();
    };
    match toml::from_str::<std::collections::HashMap<String, ProfileEntry>>(&content) {
        Ok(raw) => raw.into_iter().map(|(k, v)| (k, v.domains)).collect(),
        Err(e) => {
            tracing::warn!("profiles.toml parse failed: {e} — using empty profiles");
            Default::default()
        }
    }
}
