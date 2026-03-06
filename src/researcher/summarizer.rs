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

/// Summarize a single scraped source in context of the sub-question it answers.
async fn summarize_source(llm: &LlmClient, source: &ScrapedSource, topic: &str) -> Result<String> {
    let messages = vec![
        ChatMessage::system(
            "You are a research analyst. Summarize the provided web page content, \
             focusing only on information relevant to the research question. \
             Be concise and factual. Include key facts, data, and claims. \
             If the content is not relevant, say so briefly.",
        ),
        ChatMessage::user(format!(
            "Overall research topic: {topic}\n\
             Specific question this source addresses: {}\n\n\
             Source URL: {}\n\
             Source content:\n{}\n\n\
             Provide a focused summary relevant to the question above.",
            source.query, source.url, source.content,
        )),
    ];

    llm.complete(messages).await
}

/// Summarize all sources concurrently, returning only non-empty summaries.
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
        async move {
            let result = summarize_source(&llm, &source, &topic).await;
            (source, result)
        }
    });

    join_all(futs)
        .await
        .into_iter()
        .filter_map(|(source, result)| match result {
            Ok(summary) if !summary.is_empty() && !summary.to_lowercase().contains("not relevant") => {
                Some(SourceSummary {
                    url: source.url,
                    title: source.title,
                    query: source.query,
                    summary,
                })
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(%e, url = %source.url, "summarize failed");
                None
            }
        })
        .collect()
}
