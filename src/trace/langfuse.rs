//! Low-level Langfuse ingestion client and pure event builders.
//! Adapted from llm-proxy/src/langfuse.rs, extended for nested spans.

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Langfuse ingestion client. Cheap to clone (reqwest is Arc-backed).
#[derive(Clone)]
pub struct LangfuseClient {
    public_key: String,
    secret_key: String,
    base_url: String,
    http: Client,
}

impl LangfuseClient {
    pub fn new(public_key: String, secret_key: String, base_url: String, http: Client) -> Self {
        Self { public_key, secret_key, base_url, http }
    }

    /// POST a batch of ingestion events. Fire-and-forget: never returns an error,
    /// warn-logs on failure so a tracing outage can never break a research run.
    pub async fn ingest(&self, batch: Vec<Value>) {
        if batch.is_empty() {
            return;
        }
        let body = json!({ "batch": batch });
        match self
            .http
            .post(format!("{}/api/public/ingestion", self.base_url))
            .basic_auth(&self.public_key, Some(&self.secret_key))
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                tracing::warn!(%status, body = %text, "Langfuse ingestion rejected");
            }
            Err(e) => tracing::warn!(error = %e, "Langfuse ingestion failed"),
            Ok(_) => {}
        }
    }
}

/// Structured payload for one LLM generation observation.
#[derive(Clone, Default)]
pub struct GenerationLog {
    pub id: String,
    pub trace_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub model: String,
    pub start_ms: u128,
    pub end_ms: u128,
    pub ttft_ms: Option<f64>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub input: Value,
    pub output: Value,
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn ms_to_iso(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nanos)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// trace-create event. Langfuse upserts a trace by body.id, so this doubles as
/// the "open" event; `trace_finalize` sends a second one that merges output/level.
pub fn trace_create(
    trace_id: &str,
    name: &str,
    ts_ms: u128,
    input: Value,
    metadata: Value,
    tags: Vec<String>,
) -> Value {
    json!({
        "id": format!("{trace_id}-open"),
        "type": "trace-create",
        "timestamp": ms_to_iso(ts_ms),
        "body": {
            "id": trace_id,
            "name": name,
            "timestamp": ms_to_iso(ts_ms),
            "input": input,
            "metadata": metadata,
            "tags": tags,
        }
    })
}

/// Second trace-create that merges final output (and optional error level) by id.
pub fn trace_finalize(
    trace_id: &str,
    ts_ms: u128,
    output: Value,
    level: Option<&str>,
    status_message: Option<&str>,
) -> Value {
    let mut body = json!({ "id": trace_id, "output": output });
    if let Some(l) = level {
        body["level"] = json!(l);
    }
    if let Some(m) = status_message {
        body["statusMessage"] = json!(m);
    }
    json!({
        "id": format!("{trace_id}-final-{ts_ms}"),
        "type": "trace-create",
        "timestamp": ms_to_iso(ts_ms),
        "body": body
    })
}

// Signature fixed by the wire-layer interface spec (8 args); reducing it would
// mean bundling params into a struct, deferred to whichever later task touches this.
#[allow(clippy::too_many_arguments)]
pub fn span_create(
    id: &str,
    trace_id: &str,
    parent_id: Option<&str>,
    name: &str,
    start_ms: u128,
    end_ms: u128,
    output: Value,
    metadata: Value,
) -> Value {
    let mut body = json!({
        "id": id,
        "traceId": trace_id,
        "name": name,
        "startTime": ms_to_iso(start_ms),
        "endTime": ms_to_iso(end_ms),
        "output": output,
        "metadata": metadata,
    });
    if let Some(p) = parent_id {
        body["parentObservationId"] = json!(p);
    }
    json!({
        "id": format!("{id}-evt"),
        "type": "span-create",
        "timestamp": ms_to_iso(start_ms),
        "body": body
    })
}

pub fn generation_create(gen: &GenerationLog) -> Value {
    let total_ms = gen.end_ms.saturating_sub(gen.start_ms) as f64;
    let mut body = json!({
        "id": gen.id,
        "traceId": gen.trace_id,
        "name": gen.name,
        "model": gen.model,
        "startTime": ms_to_iso(gen.start_ms),
        "endTime": ms_to_iso(gen.end_ms),
        "input": gen.input,
        "output": gen.output,
        "usage": {
            "input": gen.input_tokens,
            "output": gen.output_tokens,
            "total": gen.input_tokens + gen.output_tokens,
            "unit": "TOKENS"
        },
        "metadata": { "ttft_ms": gen.ttft_ms, "total_ms": total_ms }
    });
    if let Some(p) = &gen.parent_id {
        body["parentObservationId"] = json!(p);
    }
    json!({
        "id": format!("{}-evt", gen.id),
        "type": "generation-create",
        "timestamp": ms_to_iso(gen.start_ms),
        "body": body
    })
}

/// score-create event. Unused in v1 (the hook for the Phase-2 LLM judge).
#[allow(dead_code)]
pub fn score_create(trace_id: &str, name: &str, value: f64, comment: Option<&str>, ts_ms: u128) -> Value {
    let mut body = json!({ "traceId": trace_id, "name": name, "value": value });
    if let Some(c) = comment {
        body["comment"] = json!(c);
    }
    json!({
        "id": format!("{trace_id}-score-{name}-{ts_ms}"),
        "type": "score-create",
        "timestamp": ms_to_iso(ts_ms),
        "body": body
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_to_iso_formats_utc_millis() {
        // 1_000 ms after epoch → 1970-01-01T00:00:01.000Z
        assert_eq!(ms_to_iso(1_000), "1970-01-01T00:00:01.000Z");
    }

    #[test]
    fn span_create_includes_parent_when_present() {
        let ev = span_create("s1", "t1", Some("root"), "plan", 0, 10, json!(null), json!({}));
        assert_eq!(ev["body"]["parentObservationId"], json!("root"));
        assert_eq!(ev["type"], json!("span-create"));
        assert_eq!(ev["body"]["traceId"], json!("t1"));
    }

    #[test]
    fn span_create_omits_parent_when_absent() {
        let ev = span_create("s1", "t1", None, "plan", 0, 10, json!(null), json!({}));
        assert!(ev["body"].get("parentObservationId").is_none());
    }

    #[test]
    fn generation_create_totals_usage() {
        let gen = GenerationLog {
            id: "g1".into(),
            trace_id: "t1".into(),
            parent_id: Some("s1".into()),
            name: "summarize".into(),
            model: "qwen".into(),
            start_ms: 0,
            end_ms: 5,
            ttft_ms: None,
            input_tokens: 10,
            output_tokens: 7,
            input: json!("in"),
            output: json!("out"),
        };
        let ev = generation_create(&gen);
        assert_eq!(ev["body"]["usage"]["total"], json!(17));
        assert_eq!(ev["body"]["parentObservationId"], json!("s1"));
        assert_eq!(ev["type"], json!("generation-create"));
    }

    #[test]
    fn trace_finalize_sets_error_level() {
        let ev = trace_finalize("t1", 1, json!(null), Some("ERROR"), Some("boom"));
        assert_eq!(ev["body"]["level"], json!("ERROR"));
        assert_eq!(ev["body"]["statusMessage"], json!("boom"));
    }

    #[tokio::test]
    #[ignore] // requires a live Langfuse at LANGFUSE_HOST with valid keys
    async fn live_ingestion_accepts_a_trace() {
        let host = std::env::var("LANGFUSE_HOST").unwrap();
        let pk = std::env::var("LANGFUSE_PUBLIC_KEY").unwrap();
        let sk = std::env::var("LANGFUSE_SECRET_KEY").unwrap();
        let client = LangfuseClient::new(pk, sk, host, Client::new());
        let tid = uuid::Uuid::new_v4().to_string();
        client
            .ingest(vec![trace_create(&tid, "test", now_ms(), json!("hi"), json!({}), vec![])])
            .await;
        // No panic == pass; check the Langfuse UI for the trace.
    }
}
