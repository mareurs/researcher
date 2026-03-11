mod config;
mod embeddings;
mod jobs;
mod llm;
mod researcher;
mod scraper;
mod search;
mod server;

use clap::Parser;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use config::Config;
use researcher::pipeline;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Tracing — RUST_LOG controls verbosity (e.g. RUST_LOG=info,researcher=debug)
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer())
        .init();

    let mut cfg = Config::parse();
    cfg.profiles = crate::config::load_profiles();
    cfg.auth = config::AuthConfig {
        linkedin_cookie:  std::env::var("LINKEDIN_COOKIE").ok(),
        fb_cookie:        std::env::var("FB_COOKIE").ok(),
        instagram_cookie: std::env::var("INSTAGRAM_COOKIE").ok(),
        twitter_cookie:   std::env::var("TWITTER_COOKIE").ok(),
    };
    cfg.job_profile = config::load_job_profile();

    if cfg.server {
        run_server(cfg).await
    } else {
        run_cli(cfg).await
    }
}

async fn run_server(cfg: Config) -> anyhow::Result<()> {
    let addr = cfg.bind_addr.clone();
    let state = Arc::new(cfg);
    let app = server::router(state);

    info!(%addr, "starting researcher API server");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_cli(cfg: Config) -> anyhow::Result<()> {
    use crate::researcher::pipeline::{run, ResearchMode, ResearchRequest, ResearchTarget};

    let topic = cfg
        .query
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Provide a query with --query / -q, or use --server"))?;

    eprintln!("\n🔬 Researcher\n   {}\n", topic);

    let mode = match cfg.mode.as_str() {
        "quick"   => ResearchMode::Quick,
        "summary" => ResearchMode::Summary,
        "deep"    => ResearchMode::Deep,
        _         => ResearchMode::Report,
    };

    let request = ResearchRequest {
        topic: topic.clone(),
        mode,
        domains: cfg.cli_domains.clone(),
        domain_profile: cfg.domain_profile.clone(),
        target: ResearchTarget::default(),

    };

    // Token channel — print each token to stdout as it arrives
    let (token_tx, mut token_rx) = tokio::sync::mpsc::channel::<String>(256);

    // Spawn a task that prints tokens inline (no newline between them)
    let print_task = tokio::spawn(async move {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        while let Some(tok) = token_rx.recv().await {
            let _ = stdout.write_all(tok.as_bytes());
            let _ = stdout.flush();
        }
    });

    let result = run(&cfg, &request, |event| {
        eprintln!("  {event}");
        // Print a blank line before report starts streaming
        if matches!(event, pipeline::ProgressEvent::WritingReport) {
            eprintln!();
        }
    }, Some(token_tx))
    .await?;

    // Wait for all tokens to be printed
    let _ = print_task.await;
    eprintln!("\n");

    // Save to file if --output specified
    if let Some(report) = &result.report {
        if let Some(path) = &cfg.output {
            std::fs::write(path, report)?;
            eprintln!("💾 Report saved to {}", path.display());
        } else {
            println!("{report}");
        }
    }

    eprintln!(
        "✅ Done — {} sources, {} sub-queries",
        result.sources.len(),
        result.queries.len(),
    );

    Ok(())
}
