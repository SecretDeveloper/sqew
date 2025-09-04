use crate::db;
use crate::models::{Message, Queue};
use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::signal;

/// Run the HTTP server on the given port
pub async fn run_server(port: u16) -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Ensure database exists and migrations applied
    db::create_db_if_needed().await?;
    // Initialize database pool
    let pool = db::init_pool().await?;

    // Build router with queue routes and shared state
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        // Queue endpoints
        .route("/queues", get(list_queues).post(create_queue))
        .route("/queues/{name}", get(show_queue).delete(delete_queue))
        .route("/queues/{name}/stats", get(queue_stats))
        // Message endpoints
        .route(
            "/queues/{name}/messages",
            get(peek_messages).delete(purge_messages),
        )
        // DLQ and maintenance
        .route("/queues/{name}/dlq/requeue", post(requeue_dlq))
        .route("/queues/{name}/compact", post(compact_db))
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
// Request payload for creating a queue
#[derive(Deserialize)]
struct CreateQueueBody {
    name: String,
    max_attempts: Option<i32>,
    visibility_ms: Option<i32>,
}

// Query parameters for peeking messages
#[derive(Deserialize)]
struct PeekParams {
    limit: Option<i64>,
}

// List all queues
async fn list_queues(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Queue>>, (StatusCode, String)> {
    let queues = db::list_queues(&pool)
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
    // Check duplicate
    if (db::get_queue_by_name(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?)
    .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            format!("Queue '{}' already exists", name),
        ));
    }
    let max_attempts = body.max_attempts.unwrap_or(5);
    let visibility_ms = body.visibility_ms.unwrap_or(30000);
    let _id = db::create_queue(&pool, &name, None, max_attempts, visibility_ms)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let queue = db::get_queue_by_name(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap();
    Ok((StatusCode::CREATED, Json(queue)))
}

// Get queue details
async fn show_queue(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<Queue>, (StatusCode, String)> {
    let q = db::get_queue_by_name(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(queue) = q {
        Ok(Json(queue))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Queue '{}' not found", name)))
    }
}

// Delete a queue
async fn delete_queue(Path(name): Path<String>, State(pool): State<SqlitePool>) -> StatusCode {
    match db::delete_queue_by_name(&pool, &name).await {
        Ok(n) if n > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}

// Get queue stats
async fn queue_stats(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let q = db::get_queue_by_name(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let q = q.ok_or((StatusCode::NOT_FOUND, format!("Queue '{}' not found", name)))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .as_millis() as i64;
    let ready = db::count_ready_messages(&pool, q.id, now)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let leased = db::count_leased_messages(&pool, q.id, now)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let dlq_count = if let Some(dlq_id) = q.dlq_id {
        db::count_messages_by_queue(&pool, dlq_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        0
    };
    let stats = json!({"ready": ready, "leased": leased, "dlq": dlq_count});
    Ok(Json(stats))
}

// Peek messages in a queue
async fn peek_messages(
    Path(name): Path<String>,
    Query(params): Query<PeekParams>,
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<Message>>, (StatusCode, String)> {
    let limit = params.limit.unwrap_or(1);
    let msgs = db::peek_messages(&pool, &name, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(msgs))
}

// Purge all messages in a queue
async fn purge_messages(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let deleted = db::purge_messages_by_queue(&pool, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"deleted": deleted})))
}

// Requeue DLQ messages
async fn requeue_dlq(
    Path(name): Path<String>,
    State(pool): State<SqlitePool>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .as_millis() as i64;
    let moved = db::requeue_dlq(&pool, &name, now)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"requeued": moved})))
}

// Compact the database
async fn compact_db(State(pool): State<SqlitePool>) -> StatusCode {
    if db::compact_db(&pool).await.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
