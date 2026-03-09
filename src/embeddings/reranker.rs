use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::researcher::crawler::ScrapedSource;
use crate::researcher::quality::ContentQuality;

#[derive(Serialize)]
struct RerankRequest {
    query: String,
    texts: Vec<String>,
}

#[derive(Deserialize)]
struct RerankResult {
    index: usize,
    score: f32,
}

pub struct RerankerClient {
    http: Client,
    base_url: String,
}

/// A source annotated with ranking scores.
#[allow(dead_code)]
pub struct RankedSource {
    pub source: ScrapedSource,
    pub quality: ContentQuality,
    pub relevance_score: f32,
    pub combined_score: f32,
}

impl RerankerClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Rerank sources by relevance to the query using a cross-encoder.
    /// Returns sources sorted by combined score (descending).
    pub async fn rerank(
        &self,
        query: &str,
        sources: Vec<(ScrapedSource, ContentQuality)>,
        relevance_weight: f32,
        authority_weight: f32,
        quality_weight: f32,
    ) -> Result<Vec<RankedSource>> {
        if sources.is_empty() {
            return Ok(vec![]);
        }

        let texts: Vec<String> = sources
            .iter()
            .map(|(s, _)| s.content.chars().take(2000).collect())
            .collect();

        debug!(count = texts.len(), "reranking sources");

        let url = format!("{}/rerank", self.base_url);
        let req = RerankRequest {
            query: query.to_string(),
            texts,
        };

        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("rerank request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("TEI rerank error {status}: {body}");
        }

        let results: Vec<RerankResult> = resp.json().await.context("rerank parse")?;

        let mut ranked: Vec<RankedSource> = sources
            .into_iter()
            .enumerate()
            .map(|(i, (source, quality))| {
                let relevance_score = results
                    .iter()
                    .find(|r| r.index == i)
                    .map(|r| r.score)
                    .unwrap_or(0.0);

                let q_score = crate::researcher::quality::quality_score(&quality);

                let combined_score = (relevance_score * relevance_weight)
                    + (quality.domain_authority * authority_weight)
                    + (q_score * quality_weight);

                RankedSource {
                    source,
                    quality,
                    relevance_score,
                    combined_score,
                }
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.combined_score
                .partial_cmp(&a.combined_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        debug!(
            count = ranked.len(),
            top_score = ranked.first().map(|r| r.combined_score).unwrap_or(0.0),
            "reranking complete"
        );

        Ok(ranked)
    }
}
