use crate::models::{Message, Queue};
use anyhow::Context;
use sqlx::{Executor, Sqlite, SqlitePool, Transaction};
use std::path::Path;
use std::{env, fs};
// Embedded initial SQL schema for bootstrapping a new database
const INIT_SQL: &str = r#"
-- Initial schema for Sqew message queue
CREATE TABLE queue (
  id            INTEGER PRIMARY KEY,
  name          TEXT UNIQUE NOT NULL,
  max_attempts  INTEGER NOT NULL DEFAULT 5
);

CREATE TABLE message (
  id               INTEGER PRIMARY KEY,
  queue_id         INTEGER NOT NULL REFERENCES queue(id) ON DELETE CASCADE,
  payload          TEXT NOT NULL,
  attempts         INTEGER NOT NULL DEFAULT 0,
  available_at     INTEGER NOT NULL,
  created_at       INTEGER NOT NULL
);

CREATE INDEX ix_msg_visible ON message(queue_id, available_at);
"#;

pub async fn get_queue_by_name(
    pool: &SqlitePool,
    name: &str,
) -> sqlx::Result<Option<Queue>> {
    sqlx::query_as::<_, Queue>(
        "SELECT id, name, max_attempts FROM queue WHERE name = ?",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn create_queue(
    pool: &SqlitePool,
    name: &str,
    max_attempts: i32,
) -> sqlx::Result<i64> {
    let rec =
        sqlx::query("INSERT INTO queue (name, max_attempts) VALUES (?, ?)")
            .bind(name)
            .bind(max_attempts)
            .execute(pool)
            .await?;
    Ok(rec.last_insert_rowid())
}

pub async fn enqueue_message(
    pool: &SqlitePool,
    msg: &Message,
) -> sqlx::Result<i64> {
    let rec = sqlx::query(
        "INSERT INTO message (queue_id, payload, attempts, available_at, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(msg.queue_id)
    .bind(&msg.payload)
    .bind(msg.attempts)
    .bind(msg.available_at)
    .bind(msg.created_at)
    .execute(pool)
    .await?;
    Ok(rec.last_insert_rowid())
}

pub async fn get_message_by_id(
    pool: &SqlitePool,
    id: i64,
) -> sqlx::Result<Option<Message>> {
    sqlx::query_as::<_, Message>(
        "SELECT id, queue_id, payload, attempts, available_at, created_at FROM message WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}
/// Delete messages by IDs (ack)
pub async fn ack_messages(
    pool: &SqlitePool,
    ids: &[i64],
) -> sqlx::Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders =
        std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
    let sql = format!("DELETE FROM message WHERE id IN ({})", placeholders);
    let mut q = sqlx::query(&sql);
    for id in ids {
        q = q.bind(id);
    }
    let res = q.execute(pool).await?;
    Ok(res.rows_affected())
}
/// List all queues
pub async fn list_queues(pool: &SqlitePool) -> sqlx::Result<Vec<Queue>> {
    sqlx::query_as::<_, Queue>(
        "SELECT id, name, max_attempts FROM queue ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

/// Delete a queue by name, returning how many rows were affected
pub async fn delete_queue_by_name(
    pool: &SqlitePool,
    name: &str,
) -> sqlx::Result<u64> {
    let res = sqlx::query("DELETE FROM queue WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// Add more helper functions for poll, ack, nack, batch operations, DLQ, etc.
/// Purge all messages in the given queue
pub async fn purge_messages_by_queue(
    pool: &SqlitePool,
    queue_name: &str,
) -> sqlx::Result<u64> {
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
        "SELECT id, queue_id, payload, attempts, available_at, created_at
         FROM message
         WHERE queue_id = (SELECT id FROM queue WHERE name = ?)
         ORDER BY available_at, id
         LIMIT ?",
    )
    .bind(queue_name)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(msgs)
}

/// Poll (lease) up to `limit` messages: select ready, set available_at forward, return messages.
pub async fn poll_messages(
    pool: &SqlitePool,
    queue_name: &str,
    limit: i64,
    visibility_ms: i64,
) -> sqlx::Result<Vec<Message>> {
    let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT m.id
         FROM message m
         WHERE m.queue_id = (SELECT id FROM queue WHERE name = ?)
           AND m.available_at <= ?
         ORDER BY m.available_at, m.id
         LIMIT ?",
    )
    .bind(queue_name)
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;

    if ids.is_empty() {
        tx.commit().await?;
        return Ok(Vec::new());
    }

    let new_available = now + visibility_ms.max(0);
    let placeholders =
        std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
    let update_sql = format!(
        "UPDATE message SET available_at = ? WHERE id IN ({})",
        placeholders
    );
    let mut uq = sqlx::query(&update_sql).bind(new_available);
    for id in &ids {
        uq = uq.bind(id);
    }
    uq.execute(&mut *tx).await?;

    let select_sql = format!(
        "SELECT id, queue_id, payload, attempts, available_at, created_at
         FROM message WHERE id IN ({}) ORDER BY available_at, id",
        placeholders
    );
    let mut sq = sqlx::query_as::<_, Message>(&select_sql);
    for id in &ids {
        sq = sq.bind(id);
    }
    let messages = sq.fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(messages)
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
           AND available_at <= ?",
    )
    .bind(queue_id)
    .bind(now_ms)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Count queued messages in a queue
pub async fn count_queued_messages_by_queue(
    pool: &SqlitePool,
    queue_id: i64,
) -> sqlx::Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM message WHERE queue_id = ?")
            .bind(queue_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Run VACUUM to compact the database
pub async fn compact_db(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("VACUUM").execute(pool).await?;
    Ok(())
}
// The initial schema is embedded via the migrations directory SQL

/// Initialize the SQLite connection pool.
pub async fn init_pool() -> anyhow::Result<SqlitePool> {
    let current_dir =
        env::current_dir().context("Failed to get current directory")?;
    let db_file = current_dir.join("sqew.db");
    init_pool_at(&db_file).await
}

/// Initialize the SQLite connection pool at a specific path.
pub async fn init_pool_at(path: &Path) -> anyhow::Result<SqlitePool> {
    let db_url = format!("sqlite://{}", path.to_string_lossy());
    let pool = SqlitePool::connect(&db_url)
        .await
        .context("Failed to connect to the database")?;
    Ok(pool)
}

/// Create the database file (if missing) and run initial migrations.
pub async fn create_db_if_needed() -> anyhow::Result<()> {
    let current_dir =
        env::current_dir().context("Failed to get current directory")?;
    let db_file = current_dir.join("sqew.db");
    create_db_if_needed_at(&db_file, false).await
}

/// Create the database file at the given path (if missing) and run initial schema.
/// If `force_recreate` is true, delete any existing file first.
pub async fn create_db_if_needed_at(
    path: &Path,
    force_recreate: bool,
) -> anyhow::Result<()> {
    let mut is_new = false;
    if force_recreate {
        if path.exists() {
            fs::remove_file(path).with_context(|| {
                format!("Failed to delete DB at {}", path.display())
            })?;
        }
        is_new = true;
    }
    if !path.exists() {
        fs::File::create(path).with_context(|| {
            format!("Failed to create DB file at {}", path.display())
        })?;
        is_new = true;
    }
    if is_new {
        let db_url = format!("sqlite://{}", path.to_string_lossy());
        let pool = SqlitePool::connect(&db_url)
            .await
            .context("Failed to connect to the database for initialization")?;
        pool.execute(INIT_SQL)
            .await
            .context("Failed to execute initial database schema")?;
    }
    Ok(())
}

/// Nack: increment attempts, set available_at forward; drop if attempts >= max_attempts.
pub async fn nack_messages(
    pool: &SqlitePool,
    ids: &[i64],
    delay_ms: i64,
) -> sqlx::Result<(u64, u64)> {
    if ids.is_empty() {
        return Ok((0, 0));
    }
    let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let new_available = now + delay_ms.max(0);
    let placeholders =
        std::iter::repeat("?").take(ids.len()).collect::<Vec<_>>().join(",");

    // Update attempts and visibility
    let update_sql = format!(
        "UPDATE message SET attempts = attempts + 1, available_at = ? WHERE id IN ({})",
        placeholders
    );
    let mut uq = sqlx::query(&update_sql).bind(new_available);
    for id in ids {
        uq = uq.bind(id);
    }
    let updated = uq.execute(&mut *tx).await?.rows_affected();

    // Drop messages exceeding max_attempts
    let delete_sql = format!(
        "DELETE FROM message
         WHERE id IN (
            SELECT m.id FROM message m
            JOIN queue q ON q.id = m.queue_id
            WHERE m.id IN ({}) AND m.attempts >= q.max_attempts
         )",
        placeholders
    );
    let mut dq = sqlx::query(&delete_sql);
    for id in ids {
        dq = dq.bind(id);
    }
    let dropped = dq.execute(&mut *tx).await?.rows_affected();

    tx.commit().await?;
    let requeued = updated.saturating_sub(dropped);
    Ok((requeued, dropped))
}

/// Remove a message by ID
pub async fn remove_message_by_id(
    pool: &SqlitePool,
    id: i64,
) -> sqlx::Result<u64> {
    let res = sqlx::query("DELETE FROM message WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
