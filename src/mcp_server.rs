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
    model::{JsonObject, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    ServiceExt,
};
use std::sync::Arc as StdArc;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use config::Config;
use crate::researcher::progressive::maybe_disclose;
use crate::llm::client::LlmClient;


// ── Tool input schemas ────────────────────────────────────────────────────────

/// Accepts JSON number or JSON string for `Option<usize>` fields.
/// LLM tool-callers often emit numeric params as strings ("1" instead of 1).
fn deser_opt_usize<'de, D>(d: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Unexpected, Visitor};
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<usize>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "a non-negative integer or string representing one")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> { Ok(Some(v as usize)) }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            if v < 0 { Err(de::Error::invalid_value(Unexpected::Signed(v), &self)) }
            else { Ok(Some(v as usize)) }
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            v.parse::<usize>().map(Some).map_err(|_| de::Error::invalid_value(Unexpected::Str(v), &self))
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }
        fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(V)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> { Ok(None) }
    }
    d.deserialize_option(V)
}

/// Accepts a proper JSON array, a JSON-stringified array (`"[\"a\",\"b\"]"`),
/// or a comma-separated string (`"a,b"`) for `Option<Vec<String>>` fields.
/// LLM tool-callers often emit array params as a stringified JSON array or
/// as a pipe/comma-separated string because they mistake the field for a string enum.
fn deser_opt_string_or_vec<'de, D>(d: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};

    struct V;

    impl<'de> Visitor<'de> for V {
        type Value = Option<Vec<String>>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "array of strings, JSON-encoded array string, or comma-separated string")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(V)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let v = v.trim();
            if v.is_empty() {
                return Ok(None);
            }
            // Try JSON array: "[\"releases\",\"changelog\"]"
            if let Ok(arr) = serde_json::from_str::<Vec<String>>(v) {
                return Ok(Some(arr));
            }
            // Fall back: comma-separated "releases,changelog"
            let items: Vec<String> = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Ok(if items.is_empty() { None } else { Some(items) })
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element::<String>()? {
                items.push(item);
            }
            Ok(if items.is_empty() { None } else { Some(items) })
        }
    }

    d.deserialize_any(V)
}


// ── Tool inputSchema sanitizer ────────────────────────────────────────────────
//
// rmcp 1.1 + schemars derive produces shapes Claude Code's Zod-based tool
// parser silently rejects (tool drops from function list while server stays
// connected — see docs/trackers/mcp-tool-drop.md). We rewrite the schema once
// at startup so CC accepts it:
//   - drop `$schema` (top-level + nested)
//   - drop `default: null`
//   - `format: "uint"` → omit format, ensure `minimum: 0`
//   - `type: ["X", "null"]` → `type: "X"` (Option<T> nullability is implicit)

fn sanitize_router_schemas(router: &mut ToolRouter<ResearcherServer>) {
    let old = std::mem::take(router);
    for mut route in old {
        let mut owned: JsonObject = (*route.attr.input_schema).clone();
        owned.remove("$schema");
        let mut as_val = serde_json::Value::Object(owned);
        clean_schema_value(&mut as_val);
        if let serde_json::Value::Object(obj) = as_val {
            route.attr.input_schema = StdArc::new(obj);
        }
        router.add_route(route);
    }
}

fn clean_schema_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.remove("$schema");
            if matches!(map.get("title"), Some(serde_json::Value::String(_))) {
                map.remove("title");
            }
            if matches!(map.get("default"), Some(serde_json::Value::Null)) {
                map.remove("default");
            }
            if matches!(map.get("format"), Some(serde_json::Value::String(s)) if s == "uint") {
                map.remove("format");
                map.entry("minimum")
                    .or_insert(serde_json::Value::Number(0.into()));
            }
            if let Some(serde_json::Value::Array(types)) = map.get("type").cloned() {
                let non_null: Vec<_> = types
                    .into_iter()
                    .filter(|t| t != &serde_json::Value::String("null".into()))
                    .collect();
                match non_null.len() {
                    1 => {
                        map.insert("type".into(), non_null.into_iter().next().unwrap());
                    }
                    n if n > 1 => {
                        map.insert("type".into(), serde_json::Value::Array(non_null));
                    }
                    _ => {}
                }
            }
            for child in map.values_mut() {
                clean_schema_value(child);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                clean_schema_value(child);
            }
        }
        _ => {}
    }
}


// ── stdio traffic logger ──────────────────────────────────────────────────────
//
// Wraps tokio::io::{Stdin,Stdout} so every byte CC sends to us (and we send
// back) gets appended to a file with millisecond timestamps and direction
// markers. Enabled by setting `MCP_LOG_TRAFFIC=/path/to/log` in the server env.
// Disabled (zero overhead) when the env var is unset or empty.

#[derive(Clone)]
struct TrafficLog {
    file: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
}

impl TrafficLog {
    fn from_env() -> Option<Self> {
        let path = std::env::var("MCP_LOG_TRAFFIC").ok()?;
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| eprintln!("MCP_LOG_TRAFFIC open {path} failed: {e}"))
            .ok()?;
        let log = TrafficLog {
            file: std::sync::Arc::new(std::sync::Mutex::new(file)),
        };
        log.write_line("--- session start ---");
        Some(log)
    }

    fn write_line(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = self.file.lock() {
            let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
            let _ = writeln!(f, "[{ts}] {line}");
        }
    }

    fn log_bytes(&self, dir: &str, bytes: &[u8]) {
        let s = String::from_utf8_lossy(bytes);
        for line in s.split_inclusive('\n') {
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if !trimmed.is_empty() {
                self.write_line(&format!("{dir} {trimmed}"));
            }
        }
    }
}

struct LoggedStdin {
    inner: tokio::io::Stdin,
    log: Option<TrafficLog>,
}

struct LoggedStdout {
    inner: tokio::io::Stdout,
    log: Option<TrafficLog>,
}

impl tokio::io::AsyncRead for LoggedStdin {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let new = &buf.filled()[before..];
            if !new.is_empty() {
                if let Some(log) = &self.log {
                    log.log_bytes("→", new);
                }
            }
        }
        result
    }
}

impl tokio::io::AsyncWrite for LoggedStdout {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let result = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &result {
            if let Some(log) = &self.log {
                log.log_bytes("←", &buf[..*n]);
            }
        }
        result
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn logged_stdio() -> (LoggedStdin, LoggedStdout) {
    let log = TrafficLog::from_env();
    let (stdin, stdout) = rmcp::transport::stdio();
    (
        LoggedStdin {
            inner: stdin,
            log: log.clone(),
        },
        LoggedStdout {
            inner: stdout,
            log,
        },
    )
}


#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResearchInput {
    #[schemars(description = "Research subject or question. For target=code, this is the framework/library name.")]
    pub query: String,

    #[schemars(description = "topic(default)|person|company|market|code|jobs")]
    pub target: Option<String>,

    #[schemars(description = "quick|summary|report(default)|deep")]
    pub mode: Option<String>,

    #[schemars(description = "toc|abstract(default)|takeaways")]
    pub summary_style: Option<String>,

    #[schemars(description = "named preset: shopping-ro,tech-news,llm-news,academic,news,travel")]
    pub domain_profile: Option<String>,

    #[schemars(description = "ad-hoc domain filter list; unions with domain_profile. Pass as JSON array: [\"example.com\",\"docs.rs\"]")]
    #[serde(default, deserialize_with = "deser_opt_string_or_vec")]
    pub domains: Option<Vec<String>>,

    #[schemars(description = "max sub-questions for planner (topic/market only)")]
    #[serde(default, deserialize_with = "deser_opt_usize")]
    pub max_queries: Option<usize>,

    #[schemars(description = "max sources scraped per query (topic/market only)")]
    #[serde(default, deserialize_with = "deser_opt_usize")]
    pub max_sources: Option<usize>,

    #[schemars(description = "developer-docs|news|product-research|academic|general(default)")]
    pub intent: Option<String>,

    #[schemars(description = "[person] company|personal|both(default)")]
    pub person_method: Option<String>,

    #[schemars(description = "[company] country to disambiguate company name")]
    pub country: Option<String>,

    #[schemars(description = "[market] stock|crypto|macro(default)")]
    pub asset_class: Option<String>,

    #[schemars(description = "[code] version or 'latest'")]
    pub version: Option<String>,

    #[schemars(description = "[code] ARRAY of aspects: bugs, changelog, community, releases (default: [\"bugs\",\"changelog\",\"community\"]). Pass as JSON array, not a string.")]
    #[serde(default, deserialize_with = "deser_opt_string_or_vec")]
    pub aspects: Option<Vec<String>>,

    #[schemars(description = "[code] GitHub owner/repo for targeted issue/release search")]
    pub repo: Option<String>,

    #[schemars(description = "[code] keyword to narrow search within the framework")]
    pub keywords: Option<String>,

    #[schemars(description = "If true, bypass progressive disclosure and return raw JSON ResearchResult {report, sources, queries}. Default false.")]
    #[serde(default)]
    pub raw_json: Option<bool>,
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
        let mut tool_router = Self::tool_router();
        sanitize_router_schemas(&mut tool_router);
        Self {
            cfg: std::sync::Arc::new(cfg),
            tool_router,
        }
    }

    #[tool(description = "Research any topic, person, company, market, codebase, or jobs.")]
    async fn research(
        &self,
        Parameters(input): Parameters<ResearchInput>,
    ) -> String {
        tracing::info!(
            target = ?input.target,
            mode = ?input.mode,
            query = %input.query,
            "research tool invoked"
        );
        match input.target.as_deref().unwrap_or("topic") {
            "person"  => dispatch_person(self.cfg.clone(), input).await,
            "company" => dispatch_company(self.cfg.clone(), input).await,
            "market"  => dispatch_market(self.cfg.clone(), input).await,
            "code"    => dispatch_code(self.cfg.clone(), input).await,
            "jobs"    => dispatch_jobs(self.cfg.clone(), input).await,
            _         => dispatch_topic(self.cfg.clone(), input).await,
        }
    }







}

async fn dispatch_topic(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};

    let mode = match input.mode.as_deref().unwrap_or("report") {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };

    let cfg_owned: Option<Config> = if input.max_queries.is_some() || input.max_sources.is_some() {
        let mut owned = (*cfg).clone();
        if let Some(mq) = input.max_queries   { owned.max_search_queries    = mq; }
        if let Some(ms) = input.max_sources   { owned.max_sources_per_query = ms; }
        Some(owned)
    } else {
        None
    };
    let cfg_use: &Config = cfg_owned.as_ref().unwrap_or(&cfg);

    let request = ResearchRequest {
        topic: input.query,
        mode,
        domains: input.domains.unwrap_or_default(),
        domain_profile: input.domain_profile,
        target: ResearchTarget::default(),
        intent: input.intent,
    };

    match run(cfg_use, &request, |ev| eprintln!("[researcher] {ev}"), None).await {
        Ok(r) => {
            if input.raw_json.unwrap_or(false) {
                return serde_json::to_string(&r).unwrap_or_else(|e| format!("Error serializing result: {e}"));
            }
            let llm = LlmClient::new(cfg_use);
            let style = input.summary_style.as_deref().unwrap_or("abstract");
            match maybe_disclose(&llm, &request.topic, &r, style, cfg_use.progressive_disclosure_chars).await {
                Ok(d)  => d.into_string(),
                Err(e) => {
                    eprintln!("[researcher] progressive disclosure failed: {e:#}");
                    serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                }
            }
        }
        Err(e) => format!("Error: {e:#}"),
    }
}

async fn dispatch_person(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget, PersonMethod};

    let method: PersonMethod = input.person_method
        .as_deref()
        .unwrap_or("both")
        .parse()
        .unwrap_or(PersonMethod::Both);

    let request = ResearchRequest {
        topic: input.query,
        mode: ResearchMode::Report,
        domains: vec![],
        domain_profile: None,
        target: ResearchTarget::Person { method },
        intent: None,
    };

    match run(&cfg, &request, |ev| eprintln!("[researcher] {ev}"), None).await {
        Ok(r) => {
            if input.raw_json.unwrap_or(false) {
                return serde_json::to_string(&r).unwrap_or_else(|e| format!("Error serializing result: {e}"));
            }
            let llm = LlmClient::new(&cfg);
            let style = input.summary_style.as_deref().unwrap_or("abstract");
            match maybe_disclose(&llm, &request.topic, &r, style, cfg.progressive_disclosure_chars).await {
                Ok(d)  => d.into_string(),
                Err(e) => {
                    eprintln!("[researcher] progressive disclosure failed: {e:#}");
                    serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                }
            }
        }
        Err(e) => format!("Error: {e:#}"),
    }
}

async fn dispatch_company(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};

    let topic = if let Some(c) = input.country {
        format!("{} {}", input.query, c)
    } else {
        input.query
    };

    let request = ResearchRequest {
        topic,
        mode: ResearchMode::Report,
        domains: vec![],
        domain_profile: None,
        target: ResearchTarget::Company,
        intent: None,
    };

    match run(&cfg, &request, |ev| eprintln!("[researcher] {ev}"), None).await {
        Ok(r) => {
            if input.raw_json.unwrap_or(false) {
                return serde_json::to_string(&r).unwrap_or_else(|e| format!("Error serializing result: {e}"));
            }
            let llm = LlmClient::new(&cfg);
            let style = input.summary_style.as_deref().unwrap_or("abstract");
            match maybe_disclose(&llm, &request.topic, &r, style, cfg.progressive_disclosure_chars).await {
                Ok(d)  => d.into_string(),
                Err(e) => {
                    eprintln!("[researcher] progressive disclosure failed: {e:#}");
                    serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                }
            }
        }
        Err(e) => format!("Error: {e:#}"),
    }
}

async fn dispatch_market(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::researcher::pipeline::{run, AssetClass, ResearchMode, ResearchRequest, ResearchTarget};

    let asset_class: AssetClass = input.asset_class
        .as_deref()
        .unwrap_or("macro")
        .parse()
        .unwrap_or_default();

    let mode = match input.mode.as_deref().unwrap_or("report") {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };

    let target = ResearchTarget::Market { asset_class };
    let domains = crate::researcher::pipeline::domains_for_target(&target);

    let request = ResearchRequest {
        topic: input.query,
        mode,
        domains,
        domain_profile: None,
        target,
        intent: None,
    };

    match run(&cfg, &request, |ev| eprintln!("[researcher] {ev}"), None).await {
        Ok(r) => {
            if input.raw_json.unwrap_or(false) {
                return serde_json::to_string(&r).unwrap_or_else(|e| format!("Error serializing result: {e}"));
            }
            let llm = LlmClient::new(&cfg);
            let style = input.summary_style.as_deref().unwrap_or("abstract");
            match maybe_disclose(&llm, &request.topic, &r, style, cfg.progressive_disclosure_chars).await {
                Ok(d)  => d.into_string(),
                Err(e) => {
                    eprintln!("[researcher] progressive disclosure failed: {e:#}");
                    serde_json::to_string_pretty(&r).unwrap_or_else(|e| e.to_string())
                }
            }
        }
        Err(e) => format!("Error: {e:#}"),
    }
}

async fn dispatch_code(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::researcher::crawler::crawl_all;
    use crate::researcher::summarizer::summarize_all;
    use crate::researcher::publisher::write_code_report;
    use crate::researcher::pipeline::ResearchResult;

    let version = input.version.as_deref().unwrap_or("latest").to_string();
    let framework = input.query.clone();
    let aspects = input.aspects.unwrap_or_else(|| {
        vec!["bugs".to_string(), "changelog".to_string(), "community".to_string()]
    });

    let q = input.keywords.as_deref().unwrap_or("").trim().to_string();
    let q_suffix = if q.is_empty() { String::new() } else { format!(" {q}") };

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
            _ => {}
        }
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap(); // ClientBuilder::build() only fails if TLS cannot initialize — unrecoverable
    let llm = LlmClient::new(&cfg);
    let topic = format!("{framework} {version}{q_suffix}");

    let sources = crawl_all(&http, &cfg, &queries).await;
    if sources.is_empty() {
        return format!("Error: no sources found for {topic}");
    }

    let summaries = summarize_all(&llm, &sources, &topic).await;
    if summaries.is_empty() {
        return format!("Error: could not summarize sources for {topic}");
    }

    match write_code_report(&llm, &summaries, &framework, &version, &aspects).await {
        Ok(report) => {
            let style = input.summary_style.as_deref().unwrap_or("abstract");
            let result = ResearchResult {
                report: Some(report),
                sources: vec![],
                queries,
            };
            if input.raw_json.unwrap_or(false) {
                return serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing result: {e}"));
            }
            match maybe_disclose(&llm, &framework, &result, style, cfg.progressive_disclosure_chars).await {
                Ok(d)  => d.into_string(),
                Err(e) => {
                    eprintln!("[researcher] progressive disclosure failed: {e:#}");
                    result.report.unwrap_or_default()
                }
            }
        }
        Err(e) => format!("Error writing report: {e:#}"),
    }
}

async fn dispatch_jobs(cfg: std::sync::Arc<Config>, input: ResearchInput) -> String {
    use crate::jobs::fetcher::fetch_jobs;
    use crate::jobs::scorer::score_listings;
    use crate::jobs::publisher::write_job_report;

    let profile = match &cfg.job_profile {
        Some(p) => p.clone(),
        None => return "Error: no [job-profile] section found in profiles.toml. \
                        Add one to enable job search.".to_string(),
    };

    let deep = input.mode.as_deref() == Some("deep");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap(); // ClientBuilder::build() only fails if TLS cannot initialize — unrecoverable
    let llm = LlmClient::new(&cfg);

    let listings = fetch_jobs(&http, &cfg, &input.query, &profile).await;

    let scored = match score_listings(&llm, &listings, &profile, 6).await {
        Ok(s) => s,
        Err(e) => return format!("Error scoring listings: {e:#}"),
    };

    match write_job_report(&cfg, &scored, &input.query, deep).await {
        Ok(report) => report,
        Err(e) => format!("Error writing report: {e:#}"),
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ResearcherServer {
    fn get_info(&self) -> ServerInfo {
        tracing::info!("get_info called by client");
        ServerInfo::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "researcher",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Single tool: research(query, target?, ...)\n\
             \n\
             target:        topic(default)|person|company|market|code|jobs\n\
             mode:          quick=snippets | summary=bullets | report=full(default) | deep=2×sources+exhaustive\n\
             intent:        developer-docs | news | product-research | academic | general(default)\n\
             domain_profile: named preset (news/academic/tech-news/llm-news/shopping-ro/travel)\n\
             domains:       ad-hoc site list — unions with domain_profile\n\
             summary_style: toc | abstract(default) | takeaways\n\
             \n\
             Per-target params\n\
             \n\
             [person]  person_method: company|personal|both(default)\n\
             [company] country: disambiguates name\n\
             [market]  asset_class: stock|crypto|macro(default)\n\
             [code]    version, aspects=[\"changelog\",\"releases\"] (JSON ARRAY: bugs|changelog|community|releases), repo (owner/repo), keywords\n\
             [jobs]    uses profiles.toml [job-profile]; mode=deep for full descriptions\n\
             \n\
             Examples\n\
             \n\
             research(\"Axum 0.8 breaking changes\", target=\"code\", version=\"0.8\", aspects=[\"changelog\",\"releases\"])\n\
             research(\"OpenAI o3 release\", intent=\"news\", domain_profile=\"news\", mode=\"summary\")\n\
             research(\"Elon Musk\", target=\"person\", person_method=\"company\")\n\
             research(\"BTC halving\", target=\"market\", asset_class=\"crypto\")\n\
             research(\"senior Rust engineer\", target=\"jobs\")\n\
             \n\
             Progressive Disclosure\n\
             \n\
             When report > PROGRESSIVE_DISCLOSURE_CHARS (default 4000 chars), full report saved to\n\
             ~/.local/share/researcher/<timestamp>-<slug>.md and compact envelope returned:\n\
               { summary, toc, path, word_count, hint }\n\
             Read full report at 'path' with a file-reading tool.\n\
             Set PROGRESSIVE_DISCLOSURE_CHARS=0 to always return inline.\
             "
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
        llm_fast_base_url = %cfg.llm_fast_base_url,
        llm_fast_model = %cfg.llm_fast_model,
        llm_fast_stages = ?cfg.llm_fast_stages,
        searxng_url = %cfg.searxng_url,
        model = %cfg.llm_model,
        traffic_log = ?std::env::var("MCP_LOG_TRAFFIC").ok(),
        "researcher-mcp starting"
    );

    let service = ResearcherServer::new(cfg)
        .serve(logged_stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP serve error: {e:?}"))?;

    tracing::info!("researcher-mcp service running — awaiting client");
    service.waiting().await?;
    tracing::info!("researcher-mcp service stopped");
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
        brave_api_key: env("BRAVE_API_KEY", ""),
        tavily_api_key: env("TAVILY_API_KEY", ""),
        exa_api_key: env("EXA_API_KEY", ""),
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
        progressive_disclosure_chars: env_usize("PROGRESSIVE_DISCLOSURE_CHARS", 4000),
    }
}
