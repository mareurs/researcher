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
    target: &crate::researcher::pipeline::ResearchTarget,
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

    use crate::researcher::pipeline::{ResearchTarget, PersonMethod};

    let prompt = match target {
        ResearchTarget::Person { method } => {
            let sections = match method {
                PersonMethod::Company => "\
## Identity\nCurrent role, company, location, tenure.\n\
## Career Path\nPrevious roles, trajectory, expertise areas.\n\
## Public Voice\nArticles, posts, talks, opinions they've shared publicly.\n\
## Conversation Hooks\nRecent wins, projects, interesting things worth referencing in a meeting.\n\
## How to Position Your Work\nWhat they likely care about given their role and background.",
                PersonMethod::Personal => "\
## Interests & Hobbies\nSports, travel, food, culture — from public posts and profiles.\n\
## Online Presence\nWhich platforms they're active on, posting style and tone.\n\
## Personal Conversation Starters\nSpecific topics to build rapport and make a personal connection.",
                PersonMethod::Both => "\
## Identity\nCurrent role, company, location, tenure.\n\
## Career Path\nPrevious roles, trajectory, expertise areas.\n\
## Public Voice\nArticles, posts, talks, opinions.\n\
## Conversation Hooks\nRecent wins, projects, things worth referencing.\n\
## How to Position Your Work\nWhat they care about given their role.\n\
## Interests & Hobbies\nSports, travel, food, culture — from public profiles.\n\
## Personal Conversation Starters\nTopics to build personal rapport.",
            };
            format!(
                "You are preparing a meeting-prep brief on a person named {topic}.\n\
                 Using the research below, write a concise markdown report with these exact sections:\n\n\
                 {sections}\n\n\
                 Be specific — include names, dates, companies, post topics. Avoid vague generalities.\n\
                 Cite sources inline with [N] notation.\n\n\
                 {sources_text}"
            )
        }
        ResearchTarget::Company => {
            format!(
                "You are preparing a meeting-prep brief on a company named {topic}.\n\
                 Using the research below, write a concise markdown report with these exact sections:\n\n\
                 ## What They Do\nProduct, market, business model in 2-3 sentences.\n\
                 ## Size & Stage\nHeadcount, funding rounds, revenue signals.\n\
                 ## Recent News\nLaunches, press mentions, funding, leadership changes.\n\
                 ## Culture & Values\nGlassdoor signals, about-page tone, leadership style.\n\
                 ## Strategic Context\nWhat they're optimizing for right now, key challenges, opportunities.\n\n\
                 Be specific — include numbers, dates, and named people. Cite sources with [N].\n\n\
                 {sources_text}"
            )
        }
        ResearchTarget::Topic => {
            match mode {
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
            }
        }
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

#[allow(dead_code)]
pub async fn write_code_report(
    llm: &LlmClient,
    summaries: &[SourceSummary],
    framework: &str,
    version: &str,
    aspects: &[String],
) -> Result<String> {
    info!(sources = summaries.len(), "writing code research report");

    let sources_text = summaries
        .iter()
        .enumerate()
        .map(|(i, s)| format!(
            "--- Source {} ---\nURL: {}\nAspect searched: {}\nSummary:\n{}\n",
            i + 1, s.url, s.query, s.summary,
        ))
        .collect::<Vec<_>>()
        .join("\n");

    let sections: Vec<&str> = aspects.iter().filter_map(|a| match a.as_str() {
        "bugs"      => Some("## Known Bugs & Issues\nRecent or notable bugs, regressions, and open issues. Include issue numbers and links where available."),
        "changelog" => Some("## Changelog & Breaking Changes\nRecent releases, notable changes, and any breaking changes since the specified version."),
        "community" => Some("## Community Sentiment\nRecent Reddit/HN discussions, developer opinions, pain points, and common complaints or praise."),
        "releases"  => Some("## Releases\nRecent release history with dates, version numbers, and highlights."),
        _           => None,
    }).collect();

    let section_instructions = sections.join("\n\n");

    if sections.is_empty() {
        return Ok("No valid aspects provided. Use: bugs, changelog, community, releases.".to_string());
    }

    let prompt = format!(
        "You are a developer-focused research analyst. Write a concise technical report on **{framework} {version}**.\n\
         Cover only these sections (skip any section if no relevant information was found in the sources):\n\n\
         {section_instructions}\n\n\
         Rules:\n\
         - Be specific: include version numbers, dates, issue numbers, PR links\n\
         - Cite sources inline with [N] notation\n\
         - Skip sections with no relevant data rather than speculating\n\
         - No fluff, no introductions, no conclusions — just the sections\n\n\
         Research gathered:\n{sources_text}"
    );

    let messages = vec![
        ChatMessage::system(
            "You are a concise technical research analyst specialising in software frameworks \
             and libraries. Write only what the sources support. Cite inline with [N] notation.",
        ),
        ChatMessage::user(prompt),
    ];

    llm.complete(messages).await
}
