use crate::config::Config;
use crate::llm::client::{ChatMessage, LlmClient};
use crate::researcher::summarizer::SourceSummary;
use crate::trace::langfuse::{score_create, LangfuseClient};
use crate::trace::now_ms;
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Deserialize)]
struct JudgeScores {
    relevance: f64,
    coverage: f64,
    faithfulness: f64,
    citation_validity: f64,
    rationale: String,
}

fn parse_judge_response(text: &str) -> Option<JudgeScores> {
    let start = text.find('{')?;
    let end = text.rfind('}').map(|i| i + 1)?;
    if start >= end {
        return None;
    }
    serde_json::from_str(&text[start..end]).ok()
}

/// Score the final report on relevance/coverage/faithfulness/citation-validity
/// via an LLM judge, and attach each as a named score directly on `trace_id`.
///
/// Deliberately independent of `ResearchTrace`: `llm` must be an UNTRACED
/// client and this function only takes a plain `trace_id`, never a live
/// `ResearchTrace` clone. This function is designed to run inside a detached
/// background task (see `pipeline::run()`) — holding any `ResearchTrace`
/// clone here would keep its per-run event channel open and block `run()`'s
/// trace-flush-before-return guarantee, adding latency to every research call.
///
/// Never panics and never propagates an error — a judge failure (LLM error,
/// malformed JSON, Langfuse ingest failure) is logged and silently skipped.
pub async fn score_report(
    cfg: &Config,
    llm: &LlmClient,
    trace_id: &str,
    topic: &str,
    report: &str,
    summaries: &[SourceSummary],
) {
    let sources_text = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("--- Source {} ---\nURL: {}\nSummary:\n{}\n", i + 1, s.url, s.summary))
        .collect::<Vec<_>>()
        .join("\n");

    let messages = vec![
        ChatMessage::system(
            "You are a strict evaluator of research reports. Score the given report \
             on exactly four dimensions, using ONLY the provided sources as ground truth. \
             Output ONLY a JSON object — no markdown fences, no prose before or after.\n\n\
             - relevance: does the report stay on-topic, with no unrelated padding? (0.0-1.0)\n\
             - coverage: does it capture the significant angles present across the given sources? (0.0-1.0)\n\
             - faithfulness: is every claim traceable to a source, with nothing invented? (0.0-1.0)\n\
             - citation_validity: do [N] citations point at real, in-range source numbers, placed near the claim they support? (0.0-1.0)\n\n\
             Respond with exactly this shape: \
             {\"relevance\": 0.0, \"coverage\": 0.0, \"faithfulness\": 0.0, \"citation_validity\": 0.0, \"rationale\": \"one or two sentences\"}"
        ),
        ChatMessage::user(format!(
            "Topic: {topic}\n\nSources:\n{sources_text}\n\nReport to score:\n{report}"
        )),
    ];

    let response = match llm.complete(messages).await {
        Ok(text) => text,
        Err(e) => {
            warn!(error = %e, "judge LLM call failed, skipping scores");
            return;
        }
    };

    let scores = match parse_judge_response(&response) {
        Some(s) => s,
        None => {
            warn!(response = %response, "failed to parse judge JSON response, skipping scores");
            return;
        }
    };

    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "failed to build Langfuse client for judge scores, skipping");
            return;
        }
    };
    let client = LangfuseClient::new(
        cfg.langfuse_public_key.clone(),
        cfg.langfuse_secret_key.clone(),
        cfg.langfuse_host.trim_end_matches('/').to_string(),
        http,
    );

    let clamp = |v: f64| v.clamp(0.0, 1.0);
    let comment = Some(scores.rationale.as_str());
    let ts = now_ms();
    let batch = vec![
        score_create(trace_id, "relevance", clamp(scores.relevance), comment, ts),
        score_create(trace_id, "coverage", clamp(scores.coverage), comment, ts),
        score_create(trace_id, "faithfulness", clamp(scores.faithfulness), comment, ts),
        score_create(trace_id, "citation-validity", clamp(scores.citation_validity), comment, ts),
    ];
    client.ingest(batch).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let text = r#"{"relevance": 0.9, "coverage": 0.8, "faithfulness": 0.7, "citation_validity": 0.6, "rationale": "ok"}"#;
        let scores = parse_judge_response(text).expect("should parse");
        assert_eq!(scores.relevance, 0.9);
        assert_eq!(scores.citation_validity, 0.6);
        assert_eq!(scores.rationale, "ok");
    }

    #[test]
    fn parses_json_wrapped_in_code_fence_and_prose() {
        let text = "Sure, here you go:\n```json\n{\"relevance\": 0.5, \"coverage\": 0.5, \"faithfulness\": 0.5, \"citation_validity\": 0.5, \"rationale\": \"meh\"}\n```\nHope that helps!";
        let scores = parse_judge_response(text).expect("should parse despite wrapping");
        assert_eq!(scores.coverage, 0.5);
    }

    #[test]
    fn returns_none_on_malformed_json() {
        let text = "{not valid json at all";
        assert!(parse_judge_response(text).is_none());
    }

    #[test]
    fn returns_none_when_no_braces_present() {
        let text = "I refuse to answer in JSON.";
        assert!(parse_judge_response(text).is_none());
    }
}
