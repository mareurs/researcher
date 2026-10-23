use anyhow::Result;
use tracing::{debug, info, warn};

use super::crawler::ScrapedSource;
use crate::llm::client::{ChatMessage, LlmClient};

#[derive(Debug, Clone)]
pub struct SourceSummary {
    pub url: String,
    pub title: String,
    pub query: String,
    pub summary: String,
    pub provenance: super::crawler::SourceProvenance,
}

#[derive(serde::Deserialize)]
struct JudgedSummary {
    relevant: bool,
    confidence: Option<f32>,
    #[serde(default)]
    quotes: Vec<String>,
}

/// Select relevant verbatim evidence rather than passing model-written
/// generalizations to the publisher as if they were source text.
async fn summarize_source(
    llm: &LlmClient,
    source: &ScrapedSource,
    topic: &str,
) -> Result<Option<SourceSummary>> {
    let messages = vec![
        ChatMessage::system(
            "/no_think\nYou select evidence from a web page for a research question.\n             Return JSON: {\"relevant\": true/false, \"confidence\": 0.0-1.0, \"quotes\": [\"exact source excerpt\"]}.\n             Include only excerpts copied verbatim from Source content. Do not paraphrase, correct,              join separated fragments, add Markdown, or complete remembered facts. A URL/title alone is not evidence.              Keep each excerpt's conditions, exceptions and negation. Each excerpt must be self-contained: if it says this method, it, or these, include the actual preceding method heading or antecedent within the same contiguous copied passage. For technical guarantees include exact method              names and both safe and unsafe examples when requested. For shopping retain exact price/currency,              stock and condition wording only when present; a listing or search snippet does not prove current stock.              Select passages that answer the actual question, including conflicting or limiting evidence.              Do not select a generic page merely because it shares a keyword.              If no relevant factual passage exists, set relevant=false and quotes=[].              Output only the JSON object."
        ),
        ChatMessage::user(format!(
            "Research topic: {topic}\nSpecific question: {}\nSource URL: {}\n{}\nSource content:\n{}",
            source.query, source.url, source.provenance.description(), source.content
        )),
    ];
    let response = llm.complete(messages).await?;
    let trimmed = response
        .trim()
        .strip_prefix("```json")
        .or_else(|| response.trim().strip_prefix("```"))
        .unwrap_or(response.trim())
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();
    let judged: JudgedSummary = serde_json::from_str(trimmed)?;
    if !judged.relevant {
        info!(url = %source.url, confidence = judged.confidence.unwrap_or(0.0), "source not relevant");
        return Ok(None);
    }
    if judged.quotes.is_empty() {
        anyhow::bail!(
            "Relevant source {} returned no verbatim evidence",
            source.url
        );
    }
    let mut quotes = Vec::new();
    for quote in judged.quotes {
        let quote = quote.trim();
        if quote.is_empty() || !source.content.contains(quote) {
            anyhow::bail!("Unverifiable source excerpt for {}", source.url);
        }
        if !quotes.iter().any(|existing: &String| existing == quote) {
            quotes.push(quote.to_string());
        }
    }
    Ok(Some(SourceSummary {
        url: source.url.clone(),
        title: source.title.clone(),
        query: source.query.clone(),
        summary: quotes.join("\n"),
        provenance: source.provenance.clone(),
    }))
}

/// Summarize all sources concurrently, returning only non-empty summaries.
/// Summarize all sources concurrently, returning only relevant non-empty summaries.
pub async fn summarize_all(
    llm: &LlmClient,
    sources: &[ScrapedSource],
    topic: &str,
) -> Vec<SourceSummary> {
    // Limit concurrency to avoid overwhelming llama.cpp (PARALLEL=2 on heavy, 4 on fast).
    // join_all sends everything at once and causes HTTP failures when the queue fills.
    const MAX_CONCURRENT: usize = 3;
    debug!(
        count = sources.len(),
        concurrency = MAX_CONCURRENT,
        "summarizing sources"
    );

    // Clone into owned vec so futures are 'static and buffer_unordered works.
    let owned: Vec<ScrapedSource> = sources.to_vec();
    let stream = futures::stream::iter(owned.into_iter().map(|source| {
        let llm = llm.clone();
        let topic = topic.to_string();
        async move { summarize_source(&llm, &source, &topic).await }
    }));

    use futures::StreamExt;
    let results: Vec<_> = stream.buffer_unordered(MAX_CONCURRENT).collect().await;

    let mut kept = 0usize;
    let mut rejected = 0usize;
    let mut errors = 0usize;

    let summaries = results
        .into_iter()
        .filter_map(|result| match result {
            Ok(Some(summary)) => {
                kept += 1;
                Some(summary)
            }
            Ok(None) => {
                rejected += 1;
                None
            }
            Err(e) => {
                errors += 1;
                warn!(%e, "summarize failed");
                None
            }
        })
        .collect();

    info!(kept, rejected, errors, "summarize_all breakdown");
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn source(content: &str) -> ScrapedSource {
        ScrapedSource {
            url: "https://example.test/docs".into(),
            title: "Documentation".into(),
            query: "safe and unsafe".into(),
            provenance: super::super::crawler::SourceProvenance::observed(
                super::super::crawler::SourceOrigin::FetchedPage,
                false,
            ),
            content: content.into(),
            domain: "example.test".into(),
            word_count: content.split_whitespace().count(),
            raw_html_len: content.len(),
            link_count: 0,
            ad_link_count: 0,
            has_headings: true,
            has_lists: true,
            has_code_blocks: true,
            paywall_detected: false,
        }
    }

    async fn summarize_with_response(
        content: &str,
        output: serde_json::Value,
    ) -> Result<Option<SourceSummary>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let app = axum::Router::new().fallback(move || {
            let output = output.clone();
            async move {
                axum::Json(
                    serde_json::json!({"choices":[{"message":{"content": output.to_string()}}]}),
                )
            }
        });
        let server =
            tokio::spawn(async move { axum::serve(listener, app).await.expect("mock LLM") });
        let mut cfg = crate::config::Config::parse_from(["researcher"]);
        cfg.llm_provider = "openai".into();
        cfg.llm_base_url = format!("http://{address}");
        let result =
            summarize_source(&LlmClient::new(&cfg), &source(content), "safe and unsafe").await;
        server.abort();
        result
    }

    #[tokio::test]
    async fn publisher_evidence_uses_quotes_instead_of_model_generalization() {
        let quote = "read is safe; read_exact is NOT safe.";
        let result = summarize_with_response(
            quote,
            serde_json::json!({
                "relevant": true, "summary": "All AsyncRead methods are safe.", "quotes": [quote]
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.summary, quote);
    }

    #[tokio::test]
    async fn rejects_fabricated_source_quotes() {
        let result = summarize_with_response(
            "Only read is safe.",
            serde_json::json!({
                "relevant": true, "summary": "write_all is safe.", "quotes": ["write_all is safe."]
            }),
        )
        .await;
        assert!(result.is_err(), "unsupported evidence accepted: {result:?}");
    }

    #[tokio::test]
    async fn relevant_summary_without_quotes_cannot_bypass_grounding() {
        let result = summarize_with_response(
            "Only read is safe.",
            serde_json::json!({
                "relevant": true, "summary": "All operations are safe."
            }),
        )
        .await;
        assert!(result.is_err(), "unquoted model prose accepted: {result:?}");
    }

    #[tokio::test]
    async fn source_prompt_distinguishes_search_snippet_from_fetched_page() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let app =
            axum::Router::new().fallback(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let tx = tx.clone();
                async move {
                    tx.send(body["messages"][1]["content"].as_str().unwrap().to_string())
                        .await
                        .unwrap();
                    axum::Json(serde_json::json!({"choices":[{"message":{"content":
                    "{\"relevant\":true,\"quotes\":[\"Price 374.99 RON.\"]}"}}]}))
                }
            });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut cfg = crate::config::Config::parse_from(["researcher"]);
        cfg.llm_provider = "openai".into();
        cfg.llm_base_url = format!("http://{address}");
        let mut snippet = source("Price 374.99 RON.");
        snippet.raw_html_len = 0;
        snippet.provenance.origin = super::super::crawler::SourceOrigin::SearchSnippet;
        let _ = summarize_source(&LlmClient::new(&cfg), &snippet, "current price and stock").await;
        let prompt = rx.recv().await.unwrap();
        server.abort();
        assert!(prompt.contains("search_snippet"), "{prompt}");
        assert!(
            prompt.contains("does not verify current price or stock"),
            "{prompt}"
        );
    }
}
