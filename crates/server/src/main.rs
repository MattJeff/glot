use std::env;

use glot_server::{AppState, MIGRATOR, jobs, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "glot_server=info,tower_http=info".into());
    // GLOT_LOG_FORMAT=json in production: one JSON object per line, no ANSI colors.
    if env::var("GLOT_LOG_FORMAT").is_ok_and(|v| v == "json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let bind = env::var("GLOT_BIND").unwrap_or_else(|_| "127.0.0.1:8790".into());
    let web_origin = env::var("GLOT_WEB_ORIGIN").unwrap_or_else(|_| "http://localhost:3100".into());
    let secret = env::var("GLOT_SECRET_KEY").unwrap_or_else(|_| {
        tracing::warn!("GLOT_SECRET_KEY not set: using an insecure development key");
        "glot-insecure-dev-secret".into()
    });

    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;
    MIGRATOR.run(&db).await?;
    jobs::requeue_orphans(&db).await?;

    let state = AppState::new(db, &secret, web_origin.starts_with("https://"));
    jobs::spawn_worker(state.clone());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("glot-server listening on http://{bind}");
    axum::serve(listener, router(state, &web_origin))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Ctrl-C locally, SIGTERM from `docker stop` / systemd in production.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
