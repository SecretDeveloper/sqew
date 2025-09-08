use clap::Subcommand;
// (moved imports closer to usage below)

/// Queue-related CLI subcommands
#[derive(Subcommand, Debug)]
pub enum QueueCommands {
    /// List available queues
    List,
    /// Add a new queue
    Add {
        /// Queue name
        name: String,
        /// Maximum attempts (default: 5)
        #[arg(long, default_value_t = 5)]
        max_attempts: i32,
    },
    /// Remove a queue
    Remove {
        /// Queue name
        name: String,
    },
    /// Show queue details and stats
    Show {
        /// Queue name
        name: String,
    },
    /// Purge (delete) all messages in the queue
    Purge {
        /// Queue name
        name: String,
    },
    /// Peek messages without leasing
    Peek {
        /// Queue name
        name: String,
        /// Number of messages to peek
        #[arg(long, default_value_t = 1)]
        limit: i64,
    },
    /// Compact the database (VACUUM)
    Compact {
        /// Queue name (unused, for CLI consistency)
        name: String,
    },
}

/// Message-related CLI subcommands
#[derive(Subcommand, Debug)]
pub enum MessageCommands {
    /// Enqueue a JSON message. Use --payload or --file (NDJSON or JSON array).
    Enqueue {
        /// Queue name
        queue: String,
        /// Inline JSON payload (e.g. '{"k":"v"}')
        #[arg(long)]
        payload: Option<String>,
        /// Read payload(s) from file (NDJSON or JSON array)
        #[arg(long)]
        file: Option<std::path::PathBuf>,
        /// Delay visibility in milliseconds (default: 0)
        #[arg(long, default_value_t = 0)]
        delay_ms: i64,
    },
    /// Poll (lease) up to N messages; updates visibility via available_at.
    Poll {
        /// Queue name
        queue: String,
        /// Batch size (default: 1)
        #[arg(long, default_value_t = 1)]
        batch: i64,
        /// Visibility timeout in ms (default: 30000)
        #[arg(long, default_value_t = 30_000)]
        visibility_ms: i64,
    },
    /// Acknowledge (delete) messages by IDs
    Ack {
        /// Comma-separated message IDs, e.g. 1,2,3
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
    },
    /// Negative-acknowledge: increment attempts and requeue after delay
    Nack {
        /// Comma-separated message IDs, e.g. 1,2,3
        #[arg(long, value_delimiter = ',')]
        ids: Vec<i64>,
        /// Delay before message becomes visible again
        #[arg(long, default_value_t = 1000)]
        delay_ms: i64,
    },
    /// Remove a message by ID (hard delete)
    Remove {
        /// Message ID
        id: i64,
    },
    /// Peek messages in a queue (no leasing)
    Peek {
        /// Queue name
        queue: String,
        /// Number of messages to peek (default: 1)
        #[arg(long, default_value_t = 1)]
        limit: u32,
    },
    /// Peek a single message by ID
    PeekId {
        /// Message ID
        id: i64,
    },
}

/// Execute a queue command
use crate::db;
use crate::models::Message;
use crate::models::Queue;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use sqlx::SqlitePool;
use std::path::PathBuf;

// Service-level queue operations, wrapping the DB layer
/// List all queues
pub async fn list_queues(pool: &SqlitePool) -> Result<Vec<Queue>> {
    db::list_queues(pool).await.context("Failed to list queues")
}

/// Create a new queue, return the created Queue
pub async fn create_queue(
    pool: &SqlitePool,
    name: &str,
    max_attempts: i32,
) -> Result<Queue> {
    if db::get_queue_by_name(pool, name).await?.is_some() {
        return Err(anyhow!("Queue '{}' already exists", name));
    }
    db::create_queue(pool, name, max_attempts)
        .await
        .context("Failed to create queue")?;
    let q = db::get_queue_by_name(pool, name)
        .await
        .context("Failed to fetch created queue")?
        .ok_or_else(|| anyhow!("Queue '{}' not found after creation", name))?;
    Ok(q)
}

/// Delete a queue by name. Returns true if a queue was deleted
pub async fn delete_queue(
    pool: &SqlitePool,
    name: &str,
) -> Result<bool> {
    let deleted = db::delete_queue_by_name(pool, name)
        .await
        .context("Failed to delete queue")?;
    Ok(deleted > 0)
}

/// Show a queue by name
pub async fn show_queue(
    pool: &SqlitePool,
    name: &str,
) -> Result<Queue> {
    let q = db::get_queue_by_name(pool, name)
        .await
        .context("Failed to fetch queue")?
        .ok_or_else(|| anyhow!("Queue '{}' not found", name))?;
    Ok(q)
}

/// Purge all messages from a queue, return count
pub async fn purge_queue(
    pool: &SqlitePool,
    name: &str,
) -> Result<u64> {
    let deleted = db::purge_messages_by_queue(pool, name)
        .await
        .context("Failed to purge messages")?;
    Ok(deleted)
}

/// Peek messages without leasing
pub async fn peek_queue(
    pool: &SqlitePool,
    name: &str,
    limit: i64,
) -> Result<Vec<Message>> {
    let msgs = db::peek_messages(pool, name, limit)
        .await
        .context("Failed to peek messages")?;
    Ok(msgs)
}

/// Compact the database (VACUUM)
pub async fn compact(pool: &SqlitePool) -> Result<()> {
    db::compact_db(pool).await.context("Failed to compact database")
}
/// Statistics for a queue: ready, leased, dlq counts
pub async fn stats(
    pool: &SqlitePool,
    name: &str,
) -> Result<serde_json::Value> {
    // Get queue
    let q = show_queue(pool, name).await?;
    // Current time in ms
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    // Counts
    let ready = db::count_ready_messages(pool, q.id, now)
        .await
        .context("Failed to count ready messages")?;
    Ok(serde_json::json!({ "ready": ready}))
}

use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for queue/database setup
#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub force_recreate: bool,
}

impl Default for Config {
    fn default() -> Self {
        let cwd =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self { db_path: cwd.join("sqew.db"), force_recreate: false }
    }
}

/// Enqueue a message into a queue by name
pub async fn enqueue_message(
    pool: &sqlx::SqlitePool,
    queue_name: &str,
    payload: &Value,
    delay_ms: i64,
) -> Result<Message> {
    let q = db::get_queue_by_name(pool, queue_name)
        .await?
        .ok_or_else(|| anyhow!("Queue '{}' not found", queue_name))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    let msg = Message {
        id: 0,
        queue_id: q.id,
        payload: payload.to_string(),
        attempts: 0,
        available_at: now + delay_ms.max(0),
        created_at: now,
    };
    let id = db::enqueue_message(pool, &msg)
        .await
        .context("Failed to enqueue message")?;
    let created = db::get_message_by_id(pool, id)
        .await
        .context("Failed to fetch enqueued message")?
        .ok_or_else(|| anyhow!("Message not found after enqueue"))?;
    Ok(created)
}

/// Fetch a message by id
pub async fn get_message_by_id(
    pool: &sqlx::SqlitePool,
    id: i64,
) -> Result<Message> {
    db::get_message_by_id(pool, id)
        .await
        .context("Failed to fetch message")?
        .ok_or_else(|| anyhow!("Message '{}' not found", id))
}

/// Poll (lease) up to `limit` visible messages; set visibility to now + visibility_ms
pub async fn poll_messages(
    pool: &sqlx::SqlitePool,
    queue_name: &str,
    limit: i64,
    visibility_ms: i64,
) -> Result<Vec<Message>> {
    let msgs = db::poll_messages(pool, queue_name, limit, visibility_ms)
        .await
        .context("Failed to poll messages")?;
    Ok(msgs)
}

/// Ack (delete) messages by IDs; returns how many were deleted
pub async fn ack_messages(
    pool: &sqlx::SqlitePool,
    ids: &[i64],
) -> Result<u64> {
    let n =
        db::ack_messages(pool, ids).await.context("Failed to ack messages")?;
    Ok(n)
}

/// Nack messages: increment attempts and requeue with delay; drops if attempts exceed max_attempts
pub async fn nack_messages(
    pool: &sqlx::SqlitePool,
    ids: &[i64],
    delay_ms: i64,
) -> Result<(u64, u64)> {
    let (requeued, dropped) = db::nack_messages(pool, ids, delay_ms)
        .await
        .context("Failed to nack messages")?;
    Ok((requeued, dropped))
}

/// Remove a message by ID
pub async fn remove_message(
    pool: &sqlx::SqlitePool,
    id: i64,
) -> Result<bool> {
    let n = db::remove_message_by_id(pool, id)
        .await
        .context("Failed to remove message")?;
    Ok(n > 0)
}

/// Initialize the pool, ensuring the database exists first.
pub async fn init_pool(cfg: &Config) -> Result<SqlitePool> {
    db::create_db_if_needed_at(&cfg.db_path, cfg.force_recreate).await?;
    let pool = db::init_pool_at(&cfg.db_path).await?;
    Ok(pool)
}

/// Execute a queue command
pub async fn run_queue_command(cmd: QueueCommands) -> Result<()> {
    // Initialize database pool
    let pool = init_pool(&Config::default()).await?;

    match cmd {
        QueueCommands::List => {
            let queues: Vec<Queue> =
                list_queues(&pool).await.context("Error listing queues")?;
            if queues.is_empty() {
                println!("No queues found");
            } else {
                println!("{:<5} {:<20} {:<12}", "ID", "NAME", "MAX_ATTEMPTS");
                for q in queues {
                    println!(
                        "{:<5} {:<20} {:<12}",
                        q.id, q.name, q.max_attempts
                    );
                }
            }
        }
        QueueCommands::Add { name, max_attempts } => {
            // Create queue via service
            let q = create_queue(&pool, &name, max_attempts)
                .await
                .context("Error creating queue")?;
            println!("Created queue '{}' with ID {}", q.name, q.id);
        }
        QueueCommands::Remove { name } => {
            // Delete queue via service
            let removed = delete_queue(&pool, &name)
                .await
                .context("Error removing queue")?;
            if removed {
                println!("Removed queue '{}'", name);
            } else {
                eprintln!("Queue '{}' not found", name);
                std::process::exit(1);
            }
        }
        QueueCommands::Show { name } => {
            // Show queue details and stats
            let q = show_queue(&pool, &name)
                .await
                .context("Error fetching queue")?;
            // Compute stats
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
                as i64;
            let ready = db::count_ready_messages(&pool, q.id, now).await?;
            println!("Queue '{}' (ID={})", q.name, q.id);
            println!("  max_attempts: {}", q.max_attempts);
            println!("Stats: ready={}", ready);
        }
        QueueCommands::Purge { name } => {
            // Purge all messages in the queue
            let deleted = purge_queue(&pool, &name)
                .await
                .context("Error purging messages")?;
            println!("Purged {} messages from queue '{}'", deleted, name);
        }
        QueueCommands::Peek { name, limit } => {
            // Peek messages without leasing
            let msgs = peek_queue(&pool, &name, limit)
                .await
                .context("Error peeking messages")?;
            for m in msgs {
                println!("[{}] {}", m.id, m.payload);
            }
        }
        QueueCommands::Compact { name: _ } => {
            // Compact the SQLite database
            compact(&pool).await.context("Error compacting database")?;
            println!("Compacted database (VACUUM)");
        }
    }
    Ok(())
}

/// Execute a message command
pub async fn run_message_command(cmd: MessageCommands) -> Result<()> {
    let pool = init_pool(&Config::default()).await?;

    match cmd {
        MessageCommands::Enqueue { queue, payload, file, delay_ms } => {
            let mut count = 0usize;
            if let Some(path) = file {
                let content =
                    std::fs::read_to_string(&path).with_context(|| {
                        format!("Failed to read file: {}", path.display())
                    })?;
                let mut items: Vec<Value> = Vec::new();
                if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&content) {
                    items = arr;
                } else {
                    for (i, line) in content.lines().enumerate() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let val: Value = serde_json::from_str(line)
                            .with_context(|| {
                                format!("Invalid JSON at line {}", i + 1)
                            })?;
                        items.push(val);
                    }
                }
                for v in items {
                    let _ =
                        enqueue_message(&pool, &queue, &v, delay_ms).await?;
                    count += 1;
                }
            }
            if let Some(raw) = payload {
                let v: Value = serde_json::from_str(&raw)
                    .context("Invalid JSON payload")?;
                let _ = enqueue_message(&pool, &queue, &v, delay_ms).await?;
                count += 1;
            }
            if count == 0 {
                anyhow::bail!("Provide --payload or --file");
            }
            println!("Enqueued {} message(s) into '{}'", count, queue);
        }
        MessageCommands::Poll { queue, batch, visibility_ms } => {
            let msgs =
                poll_messages(&pool, &queue, batch, visibility_ms).await?;
            if msgs.is_empty() {
                println!("No messages available in '{}'", queue);
            } else {
                for m in msgs {
                    println!(
                        "[id={}] attempts={} available_at={} payload={}",
                        m.id, m.attempts, m.available_at, m.payload
                    );
                }
            }
        }
        MessageCommands::Ack { ids } => {
            let n = ack_messages(&pool, &ids).await?;
            println!("Acked {} message(s)", n);
        }
        MessageCommands::Nack { ids, delay_ms } => {
            let (requeued, dropped) =
                nack_messages(&pool, &ids, delay_ms).await?;
            println!("Nacked: requeued={} dropped={}", requeued, dropped);
        }
        MessageCommands::Remove { id } => {
            if remove_message(&pool, id).await? {
                println!("Removed message {}", id);
            } else {
                println!("Message {} not found", id);
            }
        }
        MessageCommands::Peek { queue, limit } => {
            let msgs = peek_queue(&pool, &queue, limit as i64)
                .await
                .context("Error peeking messages")?;
            if msgs.is_empty() {
                println!("No messages available in '{}'", queue);
            } else {
                for m in msgs {
                    println!(
                        "[id={}] attempts={} available_at={} payload={}",
                        m.id, m.attempts, m.available_at, m.payload
                    );
                }
            }
        }
        MessageCommands::PeekId { id } => {
            let m = get_message_by_id(&pool, id).await?;
            println!(
                "[id={}] attempts={} available_at={} payload={}",
                m.id, m.attempts, m.available_at, m.payload
            );
        }
    }
    Ok(())
}
