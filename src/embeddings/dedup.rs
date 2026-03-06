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

    // Use first 512 chars of content as the embedding input (TEI truncates anyway)
    let texts: Vec<String> = sources
        .iter()
        .map(|s| s.content.chars().take(512).collect())
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

/// Score each source by cosine similarity to the research topic query.
/// Returns sources reranked by relevance (highest first).
/// Falls back to original order if embedding fails.
pub async fn rank_by_relevance(
    client: &EmbedClient,
    query: &str,
    sources: Vec<ScrapedSource>,
) -> Vec<ScrapedSource> {
    if sources.is_empty() {
        return sources;
    }

    // Embed query + all source snippets in one batch
    let mut texts = vec![query.chars().take(512).collect::<String>()];
    texts.extend(sources.iter().map(|s| s.content.chars().take(512).collect::<String>()));

    let embeddings = match client.embed(&texts).await {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(%err, "relevance scoring failed, keeping original order");
            return sources;
        }
    };

    let query_emb = &embeddings[0];
    let mut scored: Vec<(f32, ScrapedSource)> = embeddings[1..]
        .iter()
        .zip(sources)
        .map(|(emb, src)| (cosine(query_emb, emb), src))
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let result: Vec<ScrapedSource> = scored.into_iter().map(|(_, s)| s).collect();
    debug!(count = result.len(), "sources reranked by relevance");
    result
}

