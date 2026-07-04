use anyhow::{bail, Context, Result};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::{process::Command, sync::mpsc};

use super::client::ChatMessage;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VertexTextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VertexContent {
    pub role: String,
    pub parts: Vec<VertexTextPart>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VertexSystemInstruction {
    pub parts: Vec<VertexTextPart>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VertexGenerationConfig {
    #[serde(rename = "temperature")]
    pub temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VertexRequest {
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    pub system_instruction: Option<VertexSystemInstruction>,
    pub contents: Vec<VertexContent>,
    #[serde(rename = "generationConfig")]
    pub generation_config: VertexGenerationConfig,
}

#[derive(Debug, Deserialize)]
pub struct VertexResponse {
    #[serde(default)]
    pub candidates: Vec<VertexCandidate>,
}

#[derive(Debug, Deserialize)]
pub struct VertexCandidate {
    pub content: Option<VertexResponseContent>,
}

#[derive(Debug, Deserialize)]
pub struct VertexResponseContent {
    #[serde(default)]
    pub parts: Vec<VertexResponsePart>,
}

#[derive(Debug, Deserialize)]
pub struct VertexResponsePart {
    #[serde(default)]
    pub text: Option<String>,
}

pub fn build_request(messages: &[ChatMessage], max_tokens: u32, temperature: f32) -> VertexRequest {
    let mut iter = messages.iter();
    let system_instruction = match iter.next() {
        Some(msg) if msg.role == "system" => Some(VertexSystemInstruction {
            parts: vec![VertexTextPart {
                text: msg.content.clone(),
            }],
        }),
        _ => None,
    };

    let contents = messages
        .iter()
        .filter(|msg| msg.role != "system")
        .map(|msg| VertexContent {
            role: match msg.role.as_str() {
                "assistant" => "model".to_string(),
                _ => "user".to_string(),
            },
            parts: vec![VertexTextPart {
                text: msg.content.clone(),
            }],
        })
        .collect();

    VertexRequest {
        system_instruction,
        contents,
        generation_config: VertexGenerationConfig {
            temperature,
            max_output_tokens: max_tokens,
        },
    }
}

pub fn extract_text(resp: VertexResponse) -> Result<String> {
    let text = resp
        .candidates
        .into_iter()
        .next()
        .and_then(|c| c.content)
        .map(|content| {
            content
                .parts
                .into_iter()
                .filter_map(|part| part.text)
                .collect::<String>()
        })
        .unwrap_or_default()
        .trim()
        .to_string();

    if text.is_empty() {
        bail!("Vertex response contained no text candidate");
    }

    Ok(text)
}

pub fn stream_chunk_text(resp: VertexResponse) -> String {
    resp.candidates
        .into_iter()
        .next()
        .and_then(|c| c.content)
        .map(|content| {
            content
                .parts
                .into_iter()
                .filter_map(|part| part.text)
                .collect::<String>()
        })
        .unwrap_or_default()
}

fn endpoint(base_url: &str, project: &str, location: &str, model: &str, stream: bool) -> String {
    let base = base_url.trim_end_matches('/');
    let method = if stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    format!(
        "{base}/projects/{project}/locations/{location}/publishers/google/models/{model}:{method}"
    )
}

pub async fn access_token() -> Result<String> {
    let candidates: [&[&str]; 2] = [
        &["auth", "application-default", "print-access-token"],
        &["auth", "print-access-token"],
    ];

    for args in candidates {
        let output = Command::new("gcloud")
            .args(args)
            .output()
            .await
            .with_context(|| format!("spawn gcloud {}", args.join(" ")))?;
        if output.status.success() {
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
    }

    bail!("Vertex provider requires ADC credentials; run `gcloud auth application-default login`")
}

#[allow(clippy::too_many_arguments)]
pub async fn complete(
    http: &Client,
    base_url: &str,
    project: &str,
    location: &str,
    model: &str,
    max_tokens: u32,
    temperature: f32,
    messages: Vec<ChatMessage>,
) -> Result<String> {
    let token = access_token().await?;
    let req = build_request(&messages, max_tokens, temperature);
    let url = endpoint(base_url, project, location, model, false);

    let resp = http
        .post(url)
        .bearer_auth(token)
        .json(&req)
        .send()
        .await
        .context("Vertex HTTP request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Vertex error {status}: {body}");
    }

    extract_text(resp.json().await.context("Vertex response parse")?)
}

#[allow(clippy::too_many_arguments)]
pub async fn stream(
    http: &Client,
    base_url: &str,
    project: &str,
    location: &str,
    model: &str,
    max_tokens: u32,
    temperature: f32,
    messages: Vec<ChatMessage>,
    tx: mpsc::Sender<String>,
) -> Result<String> {
    let token = access_token().await?;
    let req = build_request(&messages, max_tokens, temperature);
    let url = endpoint(base_url, project, location, model, true);

    let response = http
        .post(url)
        .bearer_auth(token)
        .json(&req)
        .send()
        .await
        .context("Vertex stream request failed")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Vertex stream error {status}: {body}");
    }

    let mut full = String::new();
    let mut stream = response.bytes_stream().eventsource();

    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                if ev.data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<VertexResponse>(&ev.data) {
                    let token = stream_chunk_text(chunk);
                    if !token.is_empty() {
                        full.push_str(&token);
                        let _ = tx.send(token).await;
                    }
                }
            }
            Err(_) => break,
        }
    }

    let full = full.trim().to_string();
    if full.is_empty() {
        bail!("Vertex stream returned no text chunks");
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_maps_system_and_chat_history() {
        let messages = vec![
            ChatMessage::system("system prompt"),
            ChatMessage::user("hello"),
            ChatMessage { role: "assistant".into(), content: "hi".into() },
        ];

        let req = build_request(&messages, 123, 0.4);
        assert_eq!(req.system_instruction.unwrap().parts[0].text, "system prompt");
        assert_eq!(req.contents.len(), 2);
        assert_eq!(req.contents[0].role, "user");
        assert_eq!(req.contents[0].parts[0].text, "hello");
        assert_eq!(req.contents[1].role, "model");
        assert_eq!(req.contents[1].parts[0].text, "hi");
        assert_eq!(req.generation_config.max_output_tokens, 123);
    }

    #[test]
    fn extract_text_returns_first_candidate_text() {
        let resp = VertexResponse {
            candidates: vec![VertexCandidate {
                content: Some(VertexResponseContent {
                    parts: vec![VertexResponsePart { text: Some("Hello".into()) }],
                }),
            }],
        };

        assert_eq!(extract_text(resp).unwrap(), "Hello");
    }

    #[test]
    fn extract_text_errors_on_empty_response() {
        let resp = VertexResponse { candidates: vec![] };
        assert!(extract_text(resp).is_err());
    }
}
