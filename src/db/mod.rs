use crate::models::{Message, Queue};
use anyhow::Context;
use sqlx::{SqlitePool, migrate::Migrator};
use std::{env, fs};

pub async fn get_queue_by_name(pool: &SqlitePool, name: &str) -> sqlx::Result<Option<Queue>> {
    sqlx::query_as::<_, Queue>(
        "SELECT id, name, dlq_id, max_attempts, visibility_ms FROM queue WHERE name = ?",
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
        "SELECT id, name, dlq_id, max_attempts, visibility_ms FROM queue ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

/// Delete a queue by name, returning how many rows were affected
pub async fn delete_queue_by_name(pool: &SqlitePool, name: &str) -> sqlx::Result<u64> {
    let res = sqlx::query("DELETE FROM queue WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// Add more helper functions for poll, ack, nack, batch operations, DLQ, etc.
/// Purge all messages in the given queue
pub async fn purge_messages_by_queue(pool: &SqlitePool, queue_name: &str) -> sqlx::Result<u64> {
    // Delete messages matching the queue name
    let res =
        sqlx::query("DELETE FROM message WHERE queue_id = (SELECT id FROM queue WHERE name = ?)")
            .bind(queue_name)
            .execute(pool)
            .await?;
    Ok(res.rows_affected())
}

/// Peek (list) messages in a queue without leasing
pub async fn peek_messages(
    pool: &SqlitePool,
    queue_name: &str,
    limit: i64,
) -> sqlx::Result<Vec<Message>> {
    let msgs = sqlx::query_as::<_, Message>(
        "SELECT id, queue_id, payload_json, priority, idempotency_key, attempts, available_at, lease_expires_at, leased_by, created_at, expires_at
         FROM message
         WHERE queue_id = (SELECT id FROM queue WHERE name = ?)
         ORDER BY priority DESC, available_at, id
         LIMIT ?"
    )
    .bind(queue_name)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(msgs)
}

/// Count ready messages (available and not leased or lease expired)
pub async fn count_ready_messages(
    pool: &SqlitePool,
    queue_id: i64,
    now_ms: i64,
) -> sqlx::Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message
         WHERE queue_id = ?
           AND available_at <= ?
           AND (lease_expires_at IS NULL OR lease_expires_at <= ?)",
    )
    .bind(queue_id)
    .bind(now_ms)
    .bind(now_ms)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Count leased messages (lease_expires_at > now)
pub async fn count_leased_messages(
    pool: &SqlitePool,
    queue_id: i64,
    now_ms: i64,
) -> sqlx::Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM message
         WHERE queue_id = ?
           AND lease_expires_at > ?",
    )
    .bind(queue_id)
    .bind(now_ms)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Count all messages in a queue (e.g., DLQ)
pub async fn count_messages_by_queue(pool: &SqlitePool, queue_id: i64) -> sqlx::Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message WHERE queue_id = ?")
        .bind(queue_id)
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// Requeue all messages from the DLQ back to the main queue
pub async fn requeue_dlq(pool: &SqlitePool, queue_name: &str, now_ms: i64) -> sqlx::Result<u64> {
    // Move messages from DLQ to primary queue
    let res = sqlx::query(
        "UPDATE message
         SET queue_id = (SELECT id FROM queue WHERE name = ?),
             attempts = 0,
             available_at = ?,
             lease_expires_at = NULL,
             leased_by = NULL
         WHERE queue_id = (
             SELECT dlq_id FROM queue WHERE name = ?
         )",
    )
    .bind(queue_name)
    .bind(now_ms)
    .bind(queue_name)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Run VACUUM to compact the database
pub async fn compact_db(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("VACUUM").execute(pool).await?;
    Ok(())
}
// Embed migrations from the `migrations` directory
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Initialize the SQLite connection pool.
pub async fn init_pool() -> anyhow::Result<SqlitePool> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let db_file = current_dir.join("sqew.db");
    let db_url = format!("sqlite://{}", db_file.to_string_lossy());
    let pool = SqlitePool::connect(&db_url)
        .await
        .context("Failed to connect to the database")?;
    Ok(pool)
}

/// Create the database file (if missing) and run initial migrations.
pub async fn create_db_if_needed() -> anyhow::Result<()> {
    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let db_file = current_dir.join("sqew.db");
    // Only run migrations on a new database
    if !db_file.exists() {
        fs::File::create(&db_file)
            .with_context(|| format!("Failed to create DB file at {}", db_file.display()))?;
        let db_url = format!("sqlite://{}", db_file.to_string_lossy());
        let pool = SqlitePool::connect(&db_url)
            .await
            .context("Failed to connect to the database for migrations")?;

        MIGRATOR
            .run(&pool)
            .await
            .context("Failed to run database migrations")?;
    }
    Ok(())
}
