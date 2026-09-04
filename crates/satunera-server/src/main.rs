//! `satunera-server`: index the content repo, serve the read API.
//!
//! Configuration is environment variables, deployment-friendly and nothing
//! more: `CONTENT_DIR` (default `./content`), `DATABASE_URL` (default
//! `sqlite:judge.db`), `BIND` (default `0.0.0.0:8080`).

use std::path::PathBuf;

use satunera_server::api;
use satunera_store::SqliteStore;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let content_dir = PathBuf::from(env_or("CONTENT_DIR", "content"));
    let database_url = env_or("DATABASE_URL", "sqlite:judge.db");
    let bind = env_or("BIND", "0.0.0.0:8080");

    let store = SqliteStore::open(&database_url).await?;

    // A malformed problem refuses the boot, with the full violation report on
    // stderr. Deliberate: a judge silently missing a problem is the failure
    // mode nobody notices until a learner asks where week-3 went.
    let index = match satunera_server::boot(&content_dir, &store).await {
        Ok(index) => index,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let app = api::router(index);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "serving");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    Ok(())
}
