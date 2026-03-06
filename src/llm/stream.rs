use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::client::ChatMessage;

#[derive(Debug, Serialize)]
struct StreamRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

/// Stream chat completion tokens over an mpsc channel.
/// Returns a receiver that yields token strings as they arrive.
/// Stream chat completion tokens via SSE, forwarding each token to `tx`.
/// Also accumulates the full response and returns it when complete.
pub async fn stream_completion(
    http: &Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    max_tokens: u32,
    temperature: f32,
    strip_thinking: bool,
    messages: Vec<ChatMessage>,
    tx: mpsc::Sender<String>,
) -> Result<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let req = StreamRequest {
        model: model.to_string(),
        messages,
        max_tokens,
        temperature,
        stream: true,
    };

    let response = http
        .post(&url)
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await
        .context("stream request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("LLM stream error {status}: {body}");
    }

    let mut stream = response.bytes_stream().eventsource();
    let mut full = String::new();

    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) if ev.data == "[DONE]" => break,
            Ok(ev) => {
                if let Ok(chunk) = serde_json::from_str::<StreamChunk>(&ev.data) {
                    for choice in chunk.choices {
                        if let Some(token) = choice.delta.content {
                            full.push_str(&token);
                            let _ = tx.send(token).await;
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }

    let full = full.trim().to_string();
    Ok(if strip_thinking { strip_think(&full) } else { full })
}

fn strip_think(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find("<think>") {
            None => { out.push_str(rest); break; }
            Some(start) => {
                out.push_str(&rest[..start]);
                match rest[start..].find("</think>") {
                    None => break,
                    Some(end) => rest = &rest[start + end + 8..],
                }
            }
        }
    }
    out.trim().to_string()
}
