use tracing::debug;

use super::client::EmbedClient;
use crate::researcher::crawler::ScrapedSource;

/// Cosine similarity between two unit-norm vectors.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// Remove semantically duplicate sources using embedding similarity.
/// Keeps the first occurrence when two sources exceed `threshold` similarity.
/// Falls back to returning all sources if the embedding call fails.
pub async fn deduplicate(
    client: &EmbedClient,
    sources: Vec<ScrapedSource>,
    threshold: f32,
) -> Vec<ScrapedSource> {
    if sources.len() <= 1 {
        return sources;
    }

    // Use first ~2000 chars of content as embedding input.
    // BGE-large-en-v1.5 handles 512 tokens (~2000 chars); TEI --auto-truncate handles overshoot.
    let texts: Vec<String> = sources
        .iter()
        .map(|s| s.content.chars().take(2000).collect())
        .collect();

    let embeddings = match client.embed(&texts).await {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(%err, "embed failed, skipping dedup");
            return sources;
        }
    };

    let mut kept: Vec<usize> = Vec::new();

    'outer: for (i, emb_i) in embeddings.iter().enumerate() {
        for &j in &kept {
            let sim = cosine(emb_i, &embeddings[j]);
            if sim >= threshold {
                debug!(
                    i, j, sim,
                    url_i = %sources[i].url,
                    url_j = %sources[j].url,
                    "dedup: dropping near-duplicate"
                );
                continue 'outer;
            }
        }
        kept.push(i);
    }

    let before = sources.len();
    let result: Vec<ScrapedSource> = kept.into_iter().map(|i| sources[i].clone()).collect();
    debug!(before, after = result.len(), "dedup complete");
    result
}



