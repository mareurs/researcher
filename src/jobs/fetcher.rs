use reqwest::Client;
use serde::Deserialize;
use tracing::info;

use crate::config::{Config, JobProfile};
use crate::search::search_with_fallback;

/// A single job listing from any source.
#[derive(Debug, Clone)]
pub struct JobListing {
    pub title: String,
    pub company: String,
    pub url: String,
    pub salary: Option<String>,
    pub description: String,
    pub source: String,
}

// ── Remotive ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RemotiveResponse {
    jobs: Vec<RemotiveJob>,
}

#[derive(Debug, Deserialize)]
struct RemotiveJob {
    title: String,
    company_name: String,
    url: String,
    #[serde(default)]
    salary: String,
    description: String,
}

async fn fetch_remotive(http: &Client, query: &str) -> Vec<JobListing> {
    let url = format!(
        "https://remotive.com/api/remote-jobs?search={}&limit=20",
        urlencoding::encode(query)
    );
    let Ok(resp) = http.get(&url).send().await else { return vec![] };
    let Ok(data) = resp.json::<RemotiveResponse>().await else { return vec![] };

    data.jobs.into_iter().map(|j| JobListing {
        title: j.title,
        company: j.company_name,
        url: j.url,
        salary: if j.salary.is_empty() { None } else { Some(j.salary) },
        description: truncate(&j.description, 400),
        source: "remotive".to_string(),
    }).collect()
}

// ── Adzuna ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct AdzunaResponse {
    results: Vec<AdzunaJob>,
}

#[derive(Debug, Deserialize)]
struct AdzunaJob {
    title: String,
    company: AdzunaCompany,
    redirect_url: String,
    #[serde(default)]
    salary_min: Option<f64>,
    #[serde(default)]
    salary_max: Option<f64>,
    description: String,
}

#[derive(Debug, Deserialize)]
struct AdzunaCompany {
    display_name: String,
}

async fn fetch_adzuna(http: &Client, query: &str, app_id: &str, app_key: &str) -> Vec<JobListing> {
    let country = std::env::var("ADZUNA_COUNTRY").unwrap_or_else(|_| "us".to_string());
    let url = format!(
        "https://api.adzuna.com/v1/api/jobs/{country}/search/1\
         ?app_id={app_id}&app_key={app_key}\
         &results_per_page=20\
         &what={}&where=remote\
         &content-type=application/json",
        urlencoding::encode(query)
    );
    let Ok(resp) = http.get(&url).send().await else { return vec![] };
    let Ok(data) = resp.json::<AdzunaResponse>().await else { return vec![] };

    data.results.into_iter().map(|j| JobListing {
        title: j.title,
        company: j.company.display_name,
        url: j.redirect_url,
        salary: match (j.salary_min, j.salary_max) {
            (Some(lo), Some(hi)) => Some(format!("${:.0}–${:.0}", lo, hi)),
            (Some(lo), None)     => Some(format!("${:.0}+", lo)),
            _                    => None,
        },
        description: truncate(&j.description, 400),
        source: "adzuna".to_string(),
    }).collect()
}

// ── SearXNG ───────────────────────────────────────────────────────────────────

async fn fetch_searxng(http: &Client, cfg: &Config, query: &str, profile: &JobProfile) -> Vec<JobListing> {
    let remote_clause = if profile.remote_only { " remote" } else { "" };
    let full_query = format!("{query}{remote_clause} job opening");

    let Ok(results) = search_with_fallback(
        http,
        &cfg.searxng_url,
        &cfg.brave_api_key,
        &cfg.tavily_api_key,
        &cfg.exa_api_key,
        None,   // job search is not profile-driven
        &full_query,
        cfg.search_results_per_query,
    ).await else { return vec![] };

    results.into_iter().map(|r| JobListing {
        title: r.title.clone(),
        company: extract_company_from_title(&r.title),
        url: r.url,
        salary: None,
        description: truncate(&r.snippet, 400),
        source: "searxng".to_string(),
    }).collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetch job listings from all configured sources, merge, and deduplicate by URL.
pub async fn fetch_jobs(http: &Client, cfg: &Config, query: &str, profile: &JobProfile) -> Vec<JobListing> {
    info!(%query, "fetching job listings");

    let (remotive, adzuna, searxng) = tokio::join!(
        fetch_remotive(http, query),
        async {
            match (
                std::env::var("ADZUNA_APP_ID").ok(),
                std::env::var("ADZUNA_APP_KEY").ok(),
            ) {
                (Some(id), Some(key)) => fetch_adzuna(http, query, &id, &key).await,
                _ => vec![],
            }
        },
        fetch_searxng(http, cfg, query, profile),
    );

    let mut seen = std::collections::HashSet::new();
    let mut listings = Vec::new();
    for job in remotive.into_iter().chain(adzuna).chain(searxng) {
        if seen.insert(job.url.clone()) {
            listings.push(job);
        }
    }

    info!(count = listings.len(), "job listings fetched");
    listings
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    // Strip HTML tags crudely (API descriptions often contain HTML)
    let mut clean = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => { in_tag = true; clean.push(' '); }
            '>' => { in_tag = false; }
            _ if !in_tag => clean.push(c),
            _ => {}
        }
    }
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.len() <= max { clean } else { format!("{}…", &clean[..max]) }
}

fn extract_company_from_title(title: &str) -> String {
    // "Senior AI Engineer at Acme Corp" → "Acme Corp"
    if let Some(pos) = title.to_lowercase().find(" at ") {
        title[pos + 4..].trim().to_string()
    } else {
        String::new()
    }
}
