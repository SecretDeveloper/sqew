use axum::{Router, routing::get};
use sqlx::SqlitePool;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Initialize database pool
    let current_dir = env::current_dir().map_err(|e| {
        tracing::error!("Failed to get current directory: {e}");
        anyhow::anyhow!("Current directory error: {e}")
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
            anyhow::anyhow!(
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
        anyhow::anyhow!("DB connection error: {} (tried path: {})", e, db_file_str)
    })?;

    // Build router with placeholder routes
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        // Add more routes for queues, messages, etc.
        ;

    let addr = SocketAddr::from(([127, 0, 0, 1], 8888));
    tracing::info!("Listening on {}", addr);
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        tracing::error!("Failed to bind address: {e}");
        anyhow::anyhow!("Bind error: {e}")
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
            anyhow::anyhow!("Server error: {e}")
        })?;
    Ok(())
}
