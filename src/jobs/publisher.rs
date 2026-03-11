use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};
use super::scorer::ScoredJob;

/// Render a job search report in list or deep mode.
/// Deep mode calls research_company for each top-N company in parallel.
pub async fn write_job_report(
    cfg: &Config,
    jobs: &[ScoredJob],
    query: &str,
    deep: bool,
) -> Result<String> {
    if jobs.is_empty() {
        return Ok(format!("# Job Search: \"{query}\"\n\nNo matching jobs found."));
    }

    info!(count = jobs.len(), deep, "writing job report");

    // Company briefs for deep mode — top 5 in parallel
    let company_briefs: Vec<Option<String>> = if deep {
        let top_n = jobs.len().min(5);
        let futs: Vec<_> = jobs[..top_n].iter().map(|j| {
            let company = j.listing.company.clone();
            let cfg = cfg.clone();
            async move {
                if company.is_empty() { return None; }
                let request = ResearchRequest {
                    topic: company.clone(),
                    mode: ResearchMode::Report,
                    domains: vec![],
                    domain_profile: None,
                    target: ResearchTarget::Company,

                };
                match run(&cfg, &request, |_| {}, None).await {
                    Ok(r) => r.report.filter(|s| !s.is_empty()),
                    Err(e) => {
                        tracing::warn!(company = %company, error = %e, "company brief failed");
                        None
                    }
                }
            }
        }).collect();

        let mut briefs = futures::future::join_all(futs).await;
        // Pad with None for jobs beyond top_n
        while briefs.len() < jobs.len() {
            briefs.push(None);
        }
        briefs
    } else {
        vec![None; jobs.len()]
    };

    // Markdown table header
    let mut out = format!("# Job Search: \"{query}\" — {} matches\n\n", jobs.len());
    out.push_str("| # | Title | Company | Salary | Match | Apply |\n");
    out.push_str("|---|-------|---------|--------|-------|-------|\n");
    for (i, j) in jobs.iter().enumerate() {
        let salary = j.listing.salary.as_deref().unwrap_or("—");
        out.push_str(&format!(
            "| {} | {} | {} | {} | ⭐ {}/10 | [link]({}) |\n",
            i + 1, j.listing.title, j.listing.company, salary, j.score, j.listing.url
        ));
    }
    out.push('\n');

    // Per-job cards
    for (i, (job, brief)) in jobs.iter().zip(company_briefs.iter()).enumerate() {
        let salary = job.listing.salary.as_deref().unwrap_or("not listed");
        out.push_str(&format!(
            "## {}. {} — {} ⭐ {}/10\n\n\
             **Why it fits:** {}\n\
             **Salary:** {}\n\
             **Role:** {}\n\
             **Apply:** {}\n",
            i + 1,
            job.listing.title,
            job.listing.company,
            job.score,
            job.reason,
            salary,
            job.listing.description,
            job.listing.url,
        ));

        if let Some(b) = brief {
            out.push_str("\n### Company Brief\n\n");
            out.push_str(b);
        }

        out.push('\n');
    }

    Ok(out)
}
