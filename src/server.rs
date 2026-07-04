use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response, Sse},
    routing::{get, post},
    Router,
};
use futures::stream::Stream;
use serde::Serialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::config::Config;
use crate::researcher::pipeline::{run, ProgressEvent};

pub type AppState = Arc<Config>;

#[derive(serde::Deserialize)]
pub struct ResearchBody {
    pub query: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub domain_profile: Option<String>,
    #[serde(default)]
    pub domains: Option<Vec<String>>,
    #[serde(default)]
    pub intent: Option<String>,
}

fn into_pipeline_request(body: ResearchBody) -> crate::researcher::pipeline::ResearchRequest {
    use crate::researcher::pipeline::{ResearchMode, ResearchRequest, ResearchTarget};
    let mode = match body.mode.as_deref().unwrap_or("report") {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };
    ResearchRequest {
        topic: body.query,
        mode,
        domains: body.domains.unwrap_or_default(),
        domain_profile: body.domain_profile,
        target: ResearchTarget::default(),
        intent: body.intent,
    }
}

#[derive(Serialize)]
struct ProgressMessage {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

pub fn router(cfg: Arc<Config>) -> Router {
    Router::new()
        .route("/", get(index_html))
        .route("/health", get(health))
        .route("/research", post(research_json))
        .route("/research/stream", post(research_stream))
        .layer(CorsLayer::permissive())
        .with_state(cfg)
}

async fn index_html() -> impl IntoResponse {
    let html = include_str!("../static/index.html");
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// POST /research — blocking, returns complete report as JSON.
async fn research_json(
    State(cfg): State<AppState>,
    Json(req): Json<ResearchBody>,
) -> Response {
    let request = into_pipeline_request(req);
    let result = run(&cfg, &request, |_| {}, None).await;
    match result {
        Ok(r) => Json(serde_json::json!({
            "report": r.report,
            "sources": r.sources,
            "queries": r.queries,
        })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ).into_response(),
    }
}

/// POST /research/stream — returns SSE stream of progress events + final report.
async fn research_stream(
    State(cfg): State<AppState>,
    Json(req): Json<ResearchBody>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    info!(query = %req.query, "streaming research request");

    let (event_tx, event_rx) = mpsc::channel::<axum::response::sse::Event>(128);
    let (token_tx, mut token_rx) = mpsc::channel::<String>(256);

    // Forward LLM tokens as SSE `token` events
    let token_event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(tok) = token_rx.recv().await {
            let data = serde_json::json!({ "type": "token", "content": tok }).to_string();
            let _ = token_event_tx
                .send(axum::response::sse::Event::default().data(data).event("token"))
                .await;
        }
    });

    // Run the research pipeline
    tokio::spawn(async move {
        let request = into_pipeline_request(req);
        let tx_progress = event_tx.clone();
        let result = run(&cfg, &request, move |event| {
            let msg = ProgressMessage {
                kind: "progress".into(),
                message: event.to_string(),
                data: match &event {
                    ProgressEvent::Queries(q) => Some(serde_json::json!({ "queries": q })),
                    ProgressEvent::CrawlComplete { sources } => {
                        Some(serde_json::json!({ "sources": sources }))
                    }
                    _ => None,
                },
            };
            let json = serde_json::to_string(&msg).unwrap_or_default();
            let _ = tx_progress.try_send(
                axum::response::sse::Event::default().data(json),
            );
        }, Some(token_tx))
        .await;

        let final_event = match result {
            Ok(res) => {
                let msg = serde_json::json!({
                    "type": "complete",
                    "queries": res.queries,
                    "sources": res.sources,
                    "report": res.report.unwrap_or_default(),
                });
                axum::response::sse::Event::default()
                    .data(msg.to_string())
                    .event("complete")
            }
            Err(e) => {
                let msg = serde_json::json!({ "type": "error", "message": e.to_string() });
                axum::response::sse::Event::default()
                    .data(msg.to_string())
                    .event("error")
            }
        };

        let _ = event_tx.send(final_event).await;
    });

    let stream = futures::StreamExt::map(ReceiverStream::new(event_rx), Ok);
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}
