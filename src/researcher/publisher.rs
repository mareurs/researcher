use anyhow::Result;
use tracing::info;

use crate::llm::client::{ChatMessage, LlmClient};
use super::summarizer::SourceSummary;
use crate::researcher::pipeline::ResearchMode;

/// Build the final research report.
/// If `token_tx` is provided, tokens are streamed to it in real-time.
pub async fn write_report(
    llm: &LlmClient,
    topic: &str,
    summaries: &[SourceSummary],
    mode: &ResearchMode,
    token_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<String> {
    info!(sources = summaries.len(), "writing final report");

    let sources_text = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "--- Source {} ---\nURL: {}\nQuestion addressed: {}\nSummary:\n{}\n",
                i + 1, s.url, s.query, s.summary,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = match mode {
        ResearchMode::Summary => format!(
            "Write a concise bullet-point summary:\n\
             - 5-8 key findings as bullet points\n\
             - Each bullet: one concrete fact, number, or conclusion\n\
             - Prioritize actionable facts, numbers, and dates\n\
             - No introduction, no conclusion, no section headers\n\n\
             Topic: {topic}\n\n{sources_text}"
        ),
        ResearchMode::Deep => format!(
            "Write a thorough, detailed research report on: {topic}\n\
             - Begin with an executive summary (2-3 paragraphs)\n\
             - Cover all major angles with dedicated ## sections\n\
             - Include specific facts, numbers, dates, and source references\n\
             - Conclude with key takeaways and open questions\n\
             - Use markdown formatting throughout\n\n\
             {sources_text}"
        ),
        _ => format!(
            "Research topic: {topic}\n\n\
             You have gathered the following research from multiple sources:\n\n\
             {sources_text}\n\n\
             Write a comprehensive markdown research report on '{topic}' that:\n\
             1. Starts with an executive summary\n\
             2. Has clearly organized sections\n\
             3. Synthesizes findings across sources\n\
             4. Cites sources inline with [N] notation\n\
             5. Ends with a 'Sources' section listing all URLs\n\
             6. Includes a 'Key Takeaways' section\n\n\
             Be thorough and analytical."
        ),
    };

    let messages = vec![
        ChatMessage::system(
            "You are an expert research analyst. Write comprehensive, well-structured \
             research reports based on gathered sources. Use markdown formatting. \
             Always cite sources inline using [N] notation matching the source numbers. \
             Be objective, thorough, and analytical. Synthesize information across \
             sources rather than just listing them.",
        ),
        ChatMessage::user(prompt),
    ];

    match token_tx {
        Some(tx) => llm.stream(messages, tx).await,
        None => llm.complete(messages).await,
    }
}

/// Format the final report with a source list appended.
pub fn format_report(report: &str, summaries: &[SourceSummary]) -> String {
    // Check if the LLM already included sources
    if report.to_lowercase().contains("## sources") || report.to_lowercase().contains("## references") {
        return report.to_string();
    }

    let sources = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!("[{}] {} — {}", i + 1, s.title, s.url))
        .collect::<Vec<_>>()
        .join("\n");

    format!("{}\n\n## Sources\n\n{}", report, sources)
}
