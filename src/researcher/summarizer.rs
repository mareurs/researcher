use anyhow::Result;
use futures::future::join_all;
use tracing::debug;

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
    #[allow(dead_code)]
    confidence: Option<f32>,
    summary: String,
}

/// Summarize a single scraped source in context of the sub-question it answers.
async fn summarize_source(llm: &LlmClient, source: &ScrapedSource, topic: &str) -> Result<Option<SourceSummary>> {
    let messages = vec![
        ChatMessage::system(
            "/no_think\n\
             You are a research analyst. Evaluate the web page content for relevance to the \
             research question, then summarize if relevant.\n\n\
             Return JSON with exactly these fields:\n\
             {\"relevant\": true/false, \"confidence\": 0.0-1.0, \"summary\": \"...\"}\n\n\
             Set relevant=false if the content does not meaningfully address the research question.\n\
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
                debug!(url = %source.url, "LLM judge: not relevant");
                return Ok(None);
            }
            if judged.summary.is_empty() {
                return Ok(None);
            }
            Ok(Some(SourceSummary {
                url: source.url.clone(),
                title: source.title.clone(),
                query: source.query.clone(),
                summary: judged.summary,
            }))
        }
        Err(_) => {
            // Fallback: treat entire response as plain summary
            debug!(url = %source.url, "LLM judge: JSON parse failed, using plain summary");
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
    debug!(count = sources.len(), "summarizing sources");

    let futs = sources.iter().map(|source| {
        let llm = llm.clone();
        let topic = topic.to_string();
        let source = source.clone();
        async move { summarize_source(&llm, &source, &topic).await }
    });

    join_all(futs)
        .await
        .into_iter()
        .filter_map(|result| match result {
            Ok(Some(summary)) => Some(summary),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(%e, "summarize failed");
                None
            }
        })
        .collect()
}
