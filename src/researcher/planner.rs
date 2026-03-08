use anyhow::Result;
use tracing::info;

use crate::llm::client::{ChatMessage, LlmClient};

/// Ask the LLM to decompose a research query into focused sub-questions.
/// Returns a list of search queries to run in parallel.
pub async fn generate_queries(
    llm: &LlmClient,
    topic: &str,
    max_queries: usize,
    domains: &[String],
    target: &crate::researcher::pipeline::ResearchTarget,
) -> Result<Vec<String>> {
    info!(%topic, "planning research queries");

    use crate::researcher::pipeline::{ResearchTarget, PersonMethod};

    let system_prompt = match target {
        ResearchTarget::Person { method } => {
            let focus = match method {
                PersonMethod::Company  => "professional background, career, expertise, public work, and thought leadership",
                PersonMethod::Personal => "personal interests, hobbies, lifestyle, and online presence",
                PersonMethod::Both     => "both professional background and personal interests/hobbies",
            };
            format!(
                "You are a research planning assistant specializing in people research. \
                 Generate focused search queries to build a profile of a person — covering their {focus}. \
                 Use their name in every query. Be specific and targeted."
            )
        }
        ResearchTarget::Company => {
            "You are a research planning assistant specializing in company research. \
             Generate focused search queries covering: what the company does, its size and funding stage, \
             recent news and launches, culture and values, and strategic priorities. \
             Use the company name in every query.".to_string()
        }
        ResearchTarget::Topic => {
            "You are a research planning assistant. Your job is to decompose a research \
             topic into specific, focused search queries that together will provide \
             comprehensive coverage of the topic. Each query should target a different \
             angle or subtopic. Be specific and use natural language search terms. \
             IMPORTANT: If the topic contains ambiguous terms that could refer to multiple \
             things (e.g. a programming language name that is also a common word or product), \
             always include disambiguating context in every query — for example, prefer \
             'Rust programming language async' over just 'Rust async'.".to_string()
        }
    };

    let domain_instruction = if !domains.is_empty() {
        let domain_list = domains
            .iter()
            .map(|d| format!("site:{d}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let allowed = domains.join(", ");
        format!(
            "\n\nIMPORTANT: Restrict ALL queries to these domains only. Each query MUST include a site filter.\n\
             Example format: your search terms {domain_list}\n\
             Allowed domains: {allowed}"
        )
    } else {
        String::new()
    };

    let messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(format!(
            "Research topic: {topic}\n\n\
             Generate exactly {max_queries} distinct search queries to research this topic \
             comprehensively. Each query should be on its own line, with no numbering, \
             bullets, or extra formatting — just the raw query text.{domain_instruction}",
        )),
    ];

    let response = llm.complete(messages).await?;

    let queries: Vec<String> = response
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•', '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '.', ')']))
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.len() > 5)
        .take(max_queries)
        .map(String::from)
        .collect();

    info!(count = queries.len(), ?queries, "generated search queries");
    Ok(queries)
}
