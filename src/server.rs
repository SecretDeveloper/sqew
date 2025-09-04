use axum::{Router, routing::get};
use sqlx::SqlitePool;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber;
use anyhow::anyhow;

/// Run the HTTP server on the given port
pub async fn run_server(port: u16) -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Initialize database pool
    let current_dir = env::current_dir().map_err(|e| {
        tracing::error!("Failed to get current directory: {e}");
        anyhow!("Current directory error: {e}")
    })?;
    let db_file = current_dir.join("sqew.db");
    let db_file_str = db_file.to_string_lossy();
    // Create a blank SQLite file if it does not exist
    if !db_file.exists() {
        std::fs::File::create(&db_file).map_err(|e| {
            tracing::error!(
                "Failed to create blank DB file: {} (tried path: {})",
                e,
                db_file_str
            );
            anyhow!(
                "DB file creation error: {} (tried path: {})",
                e,
                db_file_str
            )
        })?;
    }
    let db_url = format!("sqlite://{}", db_file_str);
    let pool = SqlitePool::connect(&db_url).await.map_err(|e| {
        tracing::error!(
            "Failed to connect to DB: {} (tried path: {})",
            e,
            db_file_str
        );
        anyhow!("DB connection error: {} (tried path: {})", e, db_file_str)
    })?;

    // Build router with placeholder routes and shared state
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .with_state(pool.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Listening on {}", addr);
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        tracing::error!("Failed to bind address: {e}");
        anyhow!("Bind error: {e}")
    })?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            tracing::info!("Received Ctrl+C, shutting down gracefully...");
        })
        .await
        .map_err(|e| {
            tracing::error!("Server error: {e}");
            anyhow!("Server error: {e}")
        })?;
    Ok(())
}