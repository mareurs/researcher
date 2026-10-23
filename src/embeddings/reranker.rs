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
        min_score: f32,
    ) -> Result<Vec<RankedSource>> {
        if [relevance_weight, authority_weight, quality_weight]
            .iter()
            .any(|w| !w.is_finite() || *w < 0.0)
            || min_score.is_nan()
        {
            anyhow::bail!("Invalid rerank weights or threshold");
        }
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

        // Missing, duplicate, out-of-range, or non-finite scores invalidate the
        // response. Fabricating zero scores silently changes ranking and filtering.
        let mut scores = vec![None; sources.len()];
        for result in &results {
            if result.index >= scores.len()
                || !result.score.is_finite()
                || scores[result.index].is_some()
            {
                anyhow::bail!("Invalid rerank index or score");
            }
            scores[result.index] = Some(result.score);
        }
        if scores.iter().any(Option::is_none) {
            anyhow::bail!("Incomplete rerank response");
        }
        let scores: Vec<f32> = scores.into_iter().map(Option::unwrap).collect();
        // Percentile ranks are invariant to raw score scale. Ties get the same
        // midrank; a single result is neutral. Raw scores remain available for
        // explicitly configured model-specific filtering.
        let normalized: Vec<f32> = scores
            .iter()
            .map(|score| {
                if scores.len() < 2 {
                    return 0.5;
                }
                let lower = scores.iter().filter(|s| *s < score).count() as f32;
                let tied = scores.iter().filter(|s| *s == score).count() as f32;
                (lower + (tied - 1.0) / 2.0) / (scores.len() - 1) as f32
            })
            .collect();

        let mut ranked: Vec<RankedSource> = sources
            .into_iter()
            .enumerate()
            .filter_map(|(i, (source, quality))| {
                let relevance_score = scores[i];

            // Drop sources whose raw relevance is below the minimum threshold
                if relevance_score < min_score {
                    debug!(url = %source.url, score = relevance_score, "reranker: dropping off-topic source");
                    return None;
                }

                let q_score = crate::researcher::quality::quality_score(&quality);

                let combined_score = (normalized[i] * relevance_weight)
                    + (quality.domain_authority * authority_weight)
                    + (q_score * quality_weight);

                Some(RankedSource {
                    source,
                    quality,
                    relevance_score,
                    combined_score,
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn rank_with(
        response: serde_json::Value,
        authorities: &[f32],
        min_score: f32,
    ) -> Result<Vec<RankedSource>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().fallback(move || {
            let response = response.clone();
            async move { axum::Json(response) }
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let sources = authorities
            .iter()
            .enumerate()
            .map(|(i, authority)| {
                (
                    ScrapedSource {
                        url: format!("https://example.test/{i}"),
                        title: format!("Source {i}"),
                        query: "topic".into(),
                        content: "source evidence".into(),
                        domain: "example.test".into(),
                        word_count: 2,
                        raw_html_len: 20,
                        link_count: 0,
                        ad_link_count: 0,
                        has_headings: true,
                        has_lists: false,
                        has_code_blocks: false,
                        paywall_detected: false,
                        provenance: Default::default(),
                    },
                    ContentQuality {
                        text_density: 1.0,
                        ad_link_ratio: 0.0,
                        has_structure: true,
                        domain_authority: *authority,
                    },
                )
            })
            .collect();
        let result = RerankerClient::new(&format!("http://{address}"))
            .rerank("topic", sources, 0.7, 0.2, 0.1, min_score)
            .await;
        server.abort();
        result
    }

    async fn rank(response: serde_json::Value) -> Result<Vec<RankedSource>> {
        rank_with(response, &[1.0, 0.0], f32::NEG_INFINITY).await
    }

    #[test]
    fn absolute_score_filter_is_disabled_by_default() {
        use clap::Parser;
        let cfg = crate::config::Config::parse_from(["researcher"]);
        assert_eq!(cfg.rerank_min_score, f32::NEG_INFINITY);
    }

    #[tokio::test]
    async fn relevance_order_is_invariant_to_raw_model_score_scale() {
        let tiny =
            rank(serde_json::json!([{"index":1,"score":0.00002},{"index":0,"score":0.00001}]))
                .await
                .unwrap();
        let large = rank(serde_json::json!([{"index":0,"score":-10.0},{"index":1,"score":10.0}]))
            .await
            .unwrap();
        assert_eq!(tiny[0].source.title, "Source 1");
        assert_eq!(
            tiny.iter().map(|r| &r.source.url).collect::<Vec<_>>(),
            large.iter().map(|r| &r.source.url).collect::<Vec<_>>()
        );
        assert_eq!(tiny[0].relevance_score, 0.00002);
        assert_eq!(tiny[1].relevance_score, 0.00001);
    }

    #[tokio::test]
    async fn malformed_rerank_indexes_are_errors_instead_of_zero_scores() {
        for response in [
            serde_json::json!([{"index":0,"score":0.3}]),
            serde_json::json!([{"index":0,"score":0.3},{"index":0,"score":0.4}]),
            serde_json::json!([{"index":0,"score":0.3},{"index":9,"score":0.4}]),
            // Full index coverage must not mask a repeated index.
            serde_json::json!([{"index":0,"score":0.3},{"index":1,"score":0.4},{"index":0,"score":0.5}]),
        ] {
            assert!(rank(response).await.is_err());
        }
    }

    #[tokio::test]
    async fn ties_share_midrank_and_a_single_source_is_neutral() {
        let tied = rank_with(
            serde_json::json!([{"index":0,"score":2.0},{"index":1,"score":2.0},{"index":2,"score":3.0}]),
            &[0.0, 0.0, 0.0], f32::NEG_INFINITY,
        ).await.unwrap();
        assert_eq!(
            tied.iter()
                .map(|r| r.source.title.as_str())
                .collect::<Vec<_>>(),
            ["Source 2", "Source 0", "Source 1"]
        );
        for (source, expected_relevance) in tied.iter().zip([0.7, 0.175, 0.175]) {
            let quality_component =
                crate::researcher::quality::quality_score(&source.quality) * 0.1;
            assert!((source.combined_score - quality_component - expected_relevance).abs() < 1e-6);
        }
        let single = rank_with(
            serde_json::json!([{"index":0,"score":-100.0}]),
            &[0.0],
            f32::NEG_INFINITY,
        )
        .await
        .unwrap();
        let quality_component = crate::researcher::quality::quality_score(&single[0].quality) * 0.1;
        assert!((single[0].combined_score - quality_component - 0.35).abs() < 1e-6);
    }

    #[tokio::test]
    async fn explicit_threshold_uses_raw_scores_and_keeps_the_boundary() {
        let ranked = rank_with(
            serde_json::json!([{"index":0,"score":-2.0},{"index":1,"score":-1.0},{"index":2,"score":0.0}]),
            &[0.0, 0.0, 0.0], -1.0,
        ).await.unwrap();
        assert_eq!(
            ranked
                .iter()
                .map(|r| r.source.title.as_str())
                .collect::<Vec<_>>(),
            ["Source 2", "Source 1"]
        );
        assert_eq!(
            ranked.iter().map(|r| r.relevance_score).collect::<Vec<_>>(),
            [0.0, -1.0]
        );
    }
}
