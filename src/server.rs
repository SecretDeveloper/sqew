use crate::models::{Message, Queue};
use crate::queue;
use crate::queue::Config as QueueConfig;
use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;
use tokio::signal;

/// Run the HTTP server on the given port
pub async fn run_server(port: u16) -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Initialize database pool (ensures DB exists and schema is ready)
    let pool = queue::init_pool(&QueueConfig::default()).await?;

    // Build router with queue routes and shared state
    let app = app_router(pool.clone());

    // Allow overriding bind address via env (useful for Docker). Default 127.0.0.1
    let bind_ip = std::env::var("SQEW_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let ip: IpAddr = bind_ip
        .parse()
        .unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    let addr = SocketAddr::from((ip, port));
    tracing::info!("Listening on {} - Use Ctrl+C to quit.", addr);
    let listener = TcpListener::bind(addr).await.map_err(|e| {
        tracing::error!("Failed to bind address: {e}");
        anyhow!("Bind error: {e}")
    })?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
            tracing::info!("Received Ctrl+C, shutting down gracefully...");
        })
        .await
        .map_err(|e| {
            tracing::error!("Server error: {e}");
            anyhow!("Server error: {e}")
        })?;
    Ok(())
}

/// Construct the Axum `Router` for the service, injecting shared state.
pub fn app_router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        // Queue endpoints
        .route("/queues", get(list_queues).post(create_queue))
        .route("/queues/{name}", get(show_queue).delete(delete_queue))
        .route("/queues/{name}/stats", get(queue_stats))
        // Message endpoints
        .route(
            "/queues/{name}/messages",
            get(peek_messages)
                .post(enqueue_message_http)
                .delete(purge_messages),
        )
        .with_state(pool)
}
// Request payload for creating a queue
#[derive(Deserialize)]
struct CreateQueueBody {
    name: String,
    max_attempts: Option<i32>,
}

// Query parameters for peeking messages
#[derive(Deserialize)]
struct PeekParams {
    limit: Option<i64>,
}

// Request payload for enqueueing a message
#[derive(Deserialize)]
struct EnqueueBody {
    payload: serde_json::Value,
    #[serde(default)]
    delay_ms: Option<i64>,
}

// List all queues
async fn list_queues(
    State(pool): State<SqlitePool>
) -> Result<Json<Vec<Queue>>, (StatusCode, String)> {
    let queues = queue::list_queues(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(queues))
}

// Create a new queue
async fn create_queue(
    State(pool): State<SqlitePool>,
    Json(body): Json<CreateQueueBody>,
) -> Result<(StatusCode, Json<Queue>), (StatusCode, String)> {
    let name = body.name;
    let max_attempts = body.max_attempts.unwrap_or(5);
    // Create queue via service layer
    let new_q =
        queue::create_queue(&pool, &name, max_attempts).await.map_err(|e| {
            if e.to_string().contains("already exists") {
                (StatusCode::CONFLICT, e.to_string())
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;
    Ok((StatusCode::CREATED, Json(new_q)))
}

// Get queue details
async fn show_queue(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<Queue>, (StatusCode, String)> {
    let q = queue::show_queue(&pool, &name)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(q))
}

// Delete a queue
async fn delete_queue(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> StatusCode {
    match queue::delete_queue(&pool, &name).await {
        Ok(true) => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}

// Get queue stats
async fn queue_stats(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let stats = queue::stats(&pool, &name).await.map_err(|e| {
        if e.to_string().contains("not found") {
            (StatusCode::NOT_FOUND, e.to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    })?;
    Ok(Json(stats))
}

// Peek messages in a queue
async fn peek_messages(
    Path(name): Path<String>,
    Query(params): Query<PeekParams>,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Message>>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(1);
    let msgs = queue::peek_queue(&pool, &name, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(msgs))
}

// Purge all messages in a queue
async fn purge_messages(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let deleted = queue::purge_queue(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"deleted": deleted})))
}

// Enqueue a single message into a queue via HTTP
async fn enqueue_message_http(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
    Json(body): Json<EnqueueBody>,
) -> Result<(StatusCode, Json<Message>), (StatusCode, String)> {
    let delay = body.delay_ms.unwrap_or(0);
    let created = queue::enqueue_message(&pool, &name, &body.payload, delay)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(created)))
}
