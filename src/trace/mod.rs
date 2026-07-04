//! Langfuse tracing for the research pipeline.
//! `langfuse` = wire layer (client + event builders); `ResearchTrace` = run-scoped façade.
//!
//! `ResearchTrace` is not yet wired into `run()` (that lands in Task 5), so nothing
//! outside `#[cfg(test)]` calls these items yet — silence dead_code until then.
// TODO(task-5): remove once run() wires ResearchTrace
#![allow(dead_code)]

pub mod langfuse;

use crate::config::Config;
use langfuse::{GenerationLog, LangfuseClient};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

pub use langfuse::now_ms;

/// One trace per research run. Clone-cheap (Arc-backed) so it can be injected
/// into LlmClients and survive the concurrent summarizer.
#[derive(Clone)]
pub struct ResearchTrace {
    client: Arc<LangfuseClient>,
    trace_id: String,
    max_payload: usize,
}

/// An open span. `end_span` consumes it and emits the span-create event.
pub struct SpanHandle {
    pub id: String,
    trace_id: String,
    name: String,
    start_ms: u128,
    parent_id: Option<String>,
}

impl ResearchTrace {
    /// Returns `None` (tracing disabled) unless host + both keys are set.
    pub fn from_config(cfg: &Config) -> Option<ResearchTrace> {
        if cfg.langfuse_host.is_empty()
            || cfg.langfuse_public_key.is_empty()
            || cfg.langfuse_secret_key.is_empty()
        {
            return None;
        }
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .ok()?;
        let client = LangfuseClient::new(
            cfg.langfuse_public_key.clone(),
            cfg.langfuse_secret_key.clone(),
            cfg.langfuse_host.trim_end_matches('/').to_string(),
            http,
        );
        Some(ResearchTrace {
            client: Arc::new(client),
            trace_id: Uuid::new_v4().to_string(),
            max_payload: cfg.langfuse_max_payload_chars,
        })
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn start(&self, name: &str, input: Value, metadata: Value, tags: Vec<String>) {
        let ev = langfuse::trace_create(&self.trace_id, name, now_ms(), self.truncate(input), metadata, tags);
        self.spawn(vec![ev]);
    }

    pub fn span(&self, name: &str, parent_id: Option<&str>) -> SpanHandle {
        SpanHandle {
            id: Uuid::new_v4().to_string(),
            trace_id: self.trace_id.clone(),
            name: name.to_string(),
            start_ms: now_ms(),
            parent_id: parent_id.map(str::to_string),
        }
    }

    pub fn end_span(&self, span: SpanHandle, output: Value, metadata: Value) {
        let ev = langfuse::span_create(
            &span.id,
            &span.trace_id,
            span.parent_id.as_deref(),
            &span.name,
            span.start_ms,
            now_ms(),
            self.truncate(output),
            metadata,
        );
        self.spawn(vec![ev]);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn log_generation(
        &self,
        parent_id: &str,
        name: &str,
        model: &str,
        start_ms: u128,
        end_ms: u128,
        ttft_ms: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        input: Value,
        output: Value,
    ) {
        let gen = GenerationLog {
            id: Uuid::new_v4().to_string(),
            trace_id: self.trace_id.clone(),
            parent_id: Some(parent_id.to_string()),
            name: name.to_string(),
            model: model.to_string(),
            start_ms,
            end_ms,
            ttft_ms,
            input_tokens,
            output_tokens,
            input: self.truncate(input),
            output: self.truncate(output),
        };
        self.spawn(vec![langfuse::generation_create(&gen)]);
    }

    pub fn finalize<T>(&self, result: &anyhow::Result<T>, output: Value) {
        let ev = match result {
            Ok(_) => langfuse::trace_finalize(&self.trace_id, now_ms(), self.truncate(output), None, None),
            Err(e) => langfuse::trace_finalize(
                &self.trace_id,
                now_ms(),
                Value::Null,
                Some("ERROR"),
                Some(&format!("{e:#}")),
            ),
        };
        self.spawn(vec![ev]);
    }

    fn spawn(&self, batch: Vec<Value>) {
        let client = self.client.clone();
        tokio::spawn(async move { client.ingest(batch).await });
    }

    /// Recursively truncate every string leaf to `max_payload` chars (char-safe).
    fn truncate(&self, v: Value) -> Value {
        fn trunc_str(s: &str, max: usize) -> String {
            if s.chars().count() <= max {
                return s.to_string();
            }
            let head: String = s.chars().take(max).collect();
            format!("{head}…[+{} chars]", s.chars().count() - max)
        }
        fn walk(v: Value, max: usize) -> Value {
            match v {
                Value::String(s) => Value::String(trunc_str(&s, max)),
                Value::Array(a) => Value::Array(a.into_iter().map(|x| walk(x, max)).collect()),
                Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, walk(x, max))).collect()),
                other => other,
            }
        }
        walk(v, self.max_payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use serde_json::json;

    fn cfg_with(host: &str, pk: &str, sk: &str) -> Config {
        let mut c = crate::config::Config::parse_from(["researcher"]);
        c.langfuse_host = host.to_string();
        c.langfuse_public_key = pk.to_string();
        c.langfuse_secret_key = sk.to_string();
        c.langfuse_max_payload_chars = 10;
        c
    }

    #[test]
    fn from_config_disabled_when_incomplete() {
        assert!(ResearchTrace::from_config(&cfg_with("", "", "")).is_none());
        assert!(ResearchTrace::from_config(&cfg_with("http://x", "", "sk")).is_none());
        assert!(ResearchTrace::from_config(&cfg_with("http://x", "pk", "")).is_none());
    }

    #[test]
    fn from_config_enabled_when_complete() {
        let t = ResearchTrace::from_config(&cfg_with("http://localhost:3000", "pk", "sk"));
        assert!(t.is_some());
        assert!(!t.unwrap().trace_id().is_empty());
    }

    #[test]
    fn truncate_shortens_long_strings_char_safe() {
        let t = ResearchTrace::from_config(&cfg_with("http://x", "pk", "sk")).unwrap();
        let out = t.truncate(json!({"k": "abcdefghijklmnop"})); // 16 chars, max 10
        let s = out["k"].as_str().unwrap();
        assert!(s.starts_with("abcdefghij"));
        assert!(s.contains("[+6 chars]"));
        // short strings untouched
        assert_eq!(t.truncate(json!("short")), json!("short"));
    }
}
