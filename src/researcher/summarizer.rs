use anyhow::Result;
use tracing::{debug, info, warn};

use crate::llm::client::{ChatMessage, LlmClient};
use super::crawler::ScrapedSource;

#[derive(Debug, Clone)]
pub struct SourceSummary {
    pub url: String,
    pub title: String,
    pub query: String,
    pub summary: String,
}


#[derive(serde::Deserialize)]
struct JudgedSummary {
    relevant: bool,
    confidence: Option<f32>,
    summary: String,
}

/// Summarize a single scraped source in context of the sub-question it answers.
async fn summarize_source(llm: &LlmClient, source: &ScrapedSource, topic: &str) -> Result<Option<SourceSummary>> {
    let messages = vec![
        ChatMessage::system(
            "/no_think\n\
             You are a research analyst. Summarize web page content that is useful for a research question.\n\n\
             Return JSON with exactly these fields:\n\
             {\"relevant\": true/false, \"confidence\": 0.0-1.0, \"summary\": \"...\"}\n\n\
             Set relevant=true if the content provides ANY useful information related to the research topic — \
             including background context, model descriptions, hardware requirements, benchmarks, or comparisons. \
             Set relevant=false ONLY if the content is completely off-topic (e.g. cooking, sports, login pages, \
             generic homepages with no content, or pages in a foreign language unrelated to the topic).\n\n\
             When relevant=true, the summary should be concise and factual, including key facts, \
             data, and claims. When relevant=false, summary should be empty string.\n\n\
             Return ONLY the JSON object, no markdown fences or extra text.",
        ),
        ChatMessage::user(format!(
            "Research topic: {topic}\n\
             Specific question this source addresses: {}\n\n\
             Source URL: {}\n\
             Source content:\n{}\n",
            source.query, source.url, source.content,
        )),
    ];

    let response = llm.complete(messages).await?;

    // Try to parse structured JSON response
    match serde_json::from_str::<JudgedSummary>(response.trim()) {
        Ok(judged) => {
            if !judged.relevant {
                info!(
                    url = %source.url,
                    confidence = judged.confidence.unwrap_or(0.0),
                    "LLM judge: not relevant"
                );
                return Ok(None);
            }
            if judged.summary.is_empty() {
                info!(url = %source.url, "LLM judge: relevant but empty summary");
                return Ok(None);
            }
            Ok(Some(SourceSummary {
                url: source.url.clone(),
                title: source.title.clone(),
                query: source.query.clone(),
                summary: judged.summary,
            }))
        }
        Err(parse_err) => {
            let truncated: String = response.trim().chars().take(300).collect();
            warn!(
                url = %source.url,
                raw = %truncated,
                err = %parse_err,
                "LLM summarizer: JSON parse failed"
            );
            if response.is_empty() || response.to_lowercase().contains("not relevant") {
                Ok(None)
            } else {
                Ok(Some(SourceSummary {
                    url: source.url.clone(),
                    title: source.title.clone(),
                    query: source.query.clone(),
                    summary: response,
                }))
            }
        }
    }
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
    debug!(count = sources.len(), concurrency = MAX_CONCURRENT, "summarizing sources");

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
