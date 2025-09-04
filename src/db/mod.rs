use crate::models::{Message, Queue};
use sqlx::SqlitePool;

pub async fn get_queue_by_name(pool: &SqlitePool, name: &str) -> sqlx::Result<Option<Queue>> {
    sqlx::query_as::<_, Queue>(
        "SELECT id, name, dlq_id, max_attempts, visibility_ms FROM queue WHERE name = ?"
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn create_queue(
    pool: &SqlitePool,
    name: &str,
    dlq_id: Option<i64>,
    max_attempts: i32,
    visibility_ms: i32,
) -> sqlx::Result<i64> {
    let rec = sqlx::query(
        "INSERT INTO queue (name, dlq_id, max_attempts, visibility_ms) VALUES (?, ?, ?, ?)",
    )
    .bind(name)
    .bind(dlq_id)
    .bind(max_attempts)
    .bind(visibility_ms)
    .execute(pool)
    .await?;
    Ok(rec.last_insert_rowid())
}

pub async fn enqueue_message(pool: &SqlitePool, msg: &Message) -> sqlx::Result<i64> {
    let rec = sqlx::query(
        "INSERT INTO message (queue_id, payload_json, priority, idempotency_key, attempts, available_at, lease_expires_at, leased_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(msg.queue_id)
    .bind(&msg.payload_json)
    .bind(msg.priority)
    .bind(&msg.idempotency_key)
    .bind(msg.attempts)
    .bind(msg.available_at)
    .bind(msg.lease_expires_at)
    .bind(&msg.leased_by)
    .bind(msg.created_at)
    .bind(msg.expires_at)
    .execute(pool)
    .await?;
    Ok(rec.last_insert_rowid())
}

pub async fn get_message_by_id(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Message>> {
    sqlx::query_as::<_, Message>(
        "SELECT id, queue_id, payload_json, priority, idempotency_key, attempts, available_at, lease_expires_at, leased_by, created_at, expires_at FROM message WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}
/// List all queues
pub async fn list_queues(pool: &SqlitePool) -> sqlx::Result<Vec<Queue>> {
    sqlx::query_as::<_, Queue>(
        "SELECT id, name, dlq_id, max_attempts, visibility_ms FROM queue ORDER BY id"
    )
    .fetch_all(pool)
    .await
}

/// Delete a queue by name, returning how many rows were affected
pub async fn delete_queue_by_name(pool: &SqlitePool, name: &str) -> sqlx::Result<u64> {
    let res = sqlx::query(
        "DELETE FROM queue WHERE name = ?"
    )
    .bind(name)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

// Add more helper functions for poll, ack, nack, batch operations, DLQ, etc.
