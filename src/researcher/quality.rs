use tracing::debug;

use super::crawler::ScrapedSource;
use crate::config::Config;
use crate::researcher::pipeline::ResearchTarget;

/// Quality assessment for a scraped source.
#[derive(Debug, Clone)]
pub struct ContentQuality {
    pub text_density: f32,
    pub ad_link_ratio: f32,
    pub has_structure: bool,
    pub domain_authority: f32,
}

/// Domain authority tiers.
const TIER1_DOMAINS: &[&str] = &[
    "wikipedia.org", "github.com", "arxiv.org", "docs.rs",
    "doc.rust-lang.org", "developer.mozilla.org", "w3.org",
];
const TIER2_DOMAINS: &[&str] = &[
    "nytimes.com", "bbc.com", "reuters.com", "bloomberg.com",
    "techcrunch.com", "arstechnica.com", "nature.com", "stackoverflow.com",
];
const TIER3_DOMAINS: &[&str] = &[
    "reddit.com", "medium.com", "quora.com", "dev.to",
    "hackernoon.com", "substack.com",
];

/// Target-specific authority boosts.
fn target_authority_domains(target: &ResearchTarget) -> &'static [&'static str] {
    match target {
        ResearchTarget::Person { .. } => &[
            "linkedin.com", "twitter.com", "x.com", "crunchbase.com",
        ],
        ResearchTarget::Company => &[
            "linkedin.com", "crunchbase.com", "glassdoor.com",
            "trustpilot.com", "bloomberg.com",
        ],
        ResearchTarget::Topic => &[],
    }
}

fn domain_authority(domain: &str, target: &ResearchTarget) -> f32 {
    let target_domains = target_authority_domains(target);
    if target_domains.iter().any(|d| domain.contains(d)) {
        return 1.0;
    }
    if TIER1_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 1.0;
    }
    if TIER2_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 0.7;
    }
    if TIER3_DOMAINS.iter().any(|d| domain.contains(d)) {
        return 0.5;
    }
    0.3
}

/// Compute quality assessment for a source.
pub fn assess_quality(source: &ScrapedSource, target: &ResearchTarget) -> ContentQuality {
    let text_density = if source.raw_html_len > 0 {
        source.content.len() as f32 / source.raw_html_len as f32
    } else {
        0.5 // snippet fallback — no HTML available
    };

    let ad_link_ratio = if source.link_count > 0 {
        source.ad_link_count as f32 / source.link_count as f32
    } else {
        0.0
    };

    let has_structure = source.has_headings || source.has_lists || source.has_code_blocks;

    ContentQuality {
        text_density,
        ad_link_ratio,
        has_structure,
        domain_authority: domain_authority(&source.domain, target),
    }
}

/// Filter sources by content quality heuristics.
pub fn filter_sources(
    sources: Vec<ScrapedSource>,
    target: &ResearchTarget,
    cfg: &Config,
) -> Vec<(ScrapedSource, ContentQuality)> {
    let before = sources.len();

    let result: Vec<(ScrapedSource, ContentQuality)> = sources
        .into_iter()
        .filter_map(|source| {
            let quality = assess_quality(&source, target);

            if source.word_count < cfg.min_content_words {
                debug!(url = %source.url, words = source.word_count, "quality: dropping thin content");
                return None;
            }
            if source.paywall_detected {
                debug!(url = %source.url, "quality: dropping paywalled content");
                return None;
            }
            if quality.text_density < cfg.min_text_density {
                debug!(url = %source.url, density = quality.text_density, "quality: dropping low-density page");
                return None;
            }

            Some((source, quality))
        })
        .collect();

    debug!(before, after = result.len(), "quality filter complete");
    result
}

/// Compute a normalized quality score (0.0-1.0).
pub fn quality_score(q: &ContentQuality) -> f32 {
    let mut score = 0.5_f32;
    if q.has_structure {
        score += 0.2;
    }
    score -= q.ad_link_ratio * 0.3;
    score += (q.text_density.min(0.5) / 0.5) * 0.1;
    score.clamp(0.0, 1.0)
}
