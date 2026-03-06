use anyhow::Result;
use serde::Deserialize;
use tracing::info;

use crate::config::JobProfile;
use crate::llm::client::{ChatMessage, LlmClient};
use super::fetcher::JobListing;

/// A job listing with an LLM-assigned match score and reason.
#[derive(Debug, Clone)]
pub struct ScoredJob {
    pub listing: JobListing,
    pub score: u8,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
struct ScoreEntry {
    id: usize,
    score: u8,
    reason: String,
}

/// Score all listings against the user profile in a single LLM call.
/// Returns listings with score >= threshold, sorted descending by score.
pub async fn score_listings(
    llm: &LlmClient,
    listings: &[JobListing],
    profile: &JobProfile,
    threshold: u8,
) -> Result<Vec<ScoredJob>> {
    if listings.is_empty() {
        return Ok(vec![]);
    }

    info!(count = listings.len(), "scoring job listings against profile");

    let listings_text = listings.iter().enumerate().map(|(i, j)| {
        let salary = j.salary.as_deref().unwrap_or("not listed");
        format!(
            "{id}. {title} @ {company} | salary: {salary}\n   {desc}",
            id = i + 1,
            title = j.title,
            company = j.company,
            desc = j.description,
        )
    }).collect::<Vec<_>>().join("\n\n");

    let profile_text = format!(
        "Title: {}\nSeniority: {}\nSalary floor: {}\nRemote only: {}\n\
         Skills: {}\nPreferred company size: {}\nAvoid industries: {}\n\n\
         About me:\n{}",
        profile.title,
        profile.seniority,
        profile.salary_floor,
        profile.remote_only,
        profile.skills.join(", "),
        profile.preferred_company_size,
        profile.avoid_industries.join(", "),
        profile.about_me,
    );

    let prompt = format!(
        "You are a job-match evaluator. Score each job listing against this candidate profile.\n\n\
         ## Candidate Profile\n{profile_text}\n\n\
         ## Job Listings\n{listings_text}\n\n\
         Return a JSON array (no markdown, no explanation) with one object per listing:\n\
         [{{\"id\": 1, \"score\": 8, \"reason\": \"one line\"}}, ...]\n\
         Score 1-10. 1-5 = poor fit. 6-7 = decent. 8-10 = strong fit.\n\
         Penalise missing salary if salary_floor is set. Penalise industries in avoid_industries.\n\
         Use reason to explain the score in one concrete sentence."
    );

    let messages = vec![
        ChatMessage::system(
            "You are a precise job-match evaluator. Return only valid JSON arrays. \
             No markdown fences, no explanation outside the JSON."
        ),
        ChatMessage::user(prompt),
    ];

    let response = llm.complete(messages).await?;

    // Find the JSON array even if model adds surrounding text
    let json_start = response.find('[').unwrap_or(0);
    let json_end   = response.rfind(']').map(|i| i + 1).unwrap_or(response.len());
    let json_slice = &response[json_start..json_end];

    let scores: Vec<ScoreEntry> = serde_json::from_str(json_slice)
        .inspect_err(|e| tracing::warn!(error = %e, "failed to parse scorer JSON response"))
        .unwrap_or_default();

    let mut result: Vec<ScoredJob> = scores.into_iter().filter_map(|s| {
        let idx = s.id.checked_sub(1)?;
        let listing = listings.get(idx)?.clone();
        if s.score >= threshold {
            Some(ScoredJob { listing, score: s.score, reason: s.reason })
        } else {
            None
        }
    }).collect();

    result.sort_by(|a, b| b.score.cmp(&a.score));

    info!(kept = result.len(), "scoring complete");
    Ok(result)
}
