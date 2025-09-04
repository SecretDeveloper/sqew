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
        /// Visibility timeout in ms (default: 30000)
        #[arg(long, default_value_t = 30000)]
        visibility_ms: i32,
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
    /// Requeue all DLQ messages back to the main queue
    RequeueDlq {
        /// Queue name
        name: String,
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
    /// Peek messages in a queue
    Peek {
        /// Queue name
        queue: String,
        /// Number of messages to peek (default: 1)
        #[arg(long, default_value_t = 1)]
        limit: u32,
    },
}

/// Execute a queue command
use crate::db;
// Re-export initialization functions
pub use crate::db::{init_pool, create_db_if_needed};
use crate::models::Queue;
use anyhow::{Context, Result, anyhow};
use sqlx::SqlitePool;
use crate::models::Message;

// Service-level queue operations, wrapping the DB layer
/// List all queues
pub async fn list_queues_service(pool: &SqlitePool) -> Result<Vec<Queue>> {
    db::list_queues(pool)
        .await
        .context("Failed to list queues")
}

/// Create a new queue, return the created Queue
pub async fn create_queue_service(
    pool: &SqlitePool,
    name: &str,
    max_attempts: i32,
    visibility_ms: i32,
) -> Result<Queue> {
    if db::get_queue_by_name(pool, name)
        .await?
        .is_some() {
        return Err(anyhow!("Queue '{}' already exists", name));
    }
    db::create_queue(pool, name, None, max_attempts, visibility_ms)
        .await
        .context("Failed to create queue")?;
    let q = db::get_queue_by_name(pool, name)
        .await
        .context("Failed to fetch created queue")?
        .ok_or_else(|| anyhow!("Queue '{}' not found after creation", name))?;
    Ok(q)
}

/// Delete a queue by name. Returns true if a queue was deleted
pub async fn delete_queue_service(pool: &SqlitePool, name: &str) -> Result<bool> {
    let deleted = db::delete_queue_by_name(pool, name)
        .await
        .context("Failed to delete queue")?;
    Ok(deleted > 0)
}

/// Show a queue by name
pub async fn show_queue_service(pool: &SqlitePool, name: &str) -> Result<Queue> {
    let q = db::get_queue_by_name(pool, name)
        .await
        .context("Failed to fetch queue")?
        .ok_or_else(|| anyhow!("Queue '{}' not found", name))?;
    Ok(q)
}

/// Purge all messages from a queue, return count
pub async fn purge_queue_service(pool: &SqlitePool, name: &str) -> Result<u64> {
    let deleted = db::purge_messages_by_queue(pool, name)
        .await
        .context("Failed to purge messages")?;
    Ok(deleted)
}

/// Peek messages without leasing
pub async fn peek_queue_service(
    pool: &SqlitePool,
    name: &str,
    limit: i64,
) -> Result<Vec<Message>> {
    let msgs = db::peek_messages(pool, name, limit)
        .await
        .context("Failed to peek messages")?;
    Ok(msgs)
}

/// Requeue DLQ messages back to main queue, return count
pub async fn requeue_dlq_service(
    pool: &SqlitePool,
    name: &str,
) -> Result<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis() as i64;
    let moved = db::requeue_dlq(pool, name, now)
        .await
        .context("Failed to requeue DLQ messages")?;
    Ok(moved)
}

/// Compact the database (VACUUM)
pub async fn compact_service(pool: &SqlitePool) -> Result<()> {
    db::compact_db(pool)
        .await
        .context("Failed to compact database")
}
/// Statistics for a queue: ready, leased, dlq counts
pub async fn stats_service(
    pool: &SqlitePool,
    name: &str,
) -> Result<serde_json::Value> {
    // Get queue
    let q = show_queue_service(pool, name).await?;
    // Current time in ms
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis() as i64;
    // Counts
    let ready = db::count_ready_messages(pool, q.id, now)
        .await
        .context("Failed to count ready messages")?;
    let leased = db::count_leased_messages(pool, q.id, now)
        .await
        .context("Failed to count leased messages")?;
    let dlq_count = if let Some(dlq_id) = q.dlq_id {
        db::count_messages_by_queue(pool, dlq_id)
            .await
            .context("Failed to count DLQ messages")?
    } else {
        0
    };
    Ok(serde_json::json!({ "ready": ready, "leased": leased, "dlq": dlq_count }))
}
use std::time::{SystemTime, UNIX_EPOCH};

/// Execute a queue command
pub async fn run_queue_command(cmd: QueueCommands) -> Result<()> {
    // Ensure database exists and migrations applied
    db::create_db_if_needed().await?;
    // Initialize database pool
    let pool = db::init_pool().await?;

    match cmd {
        QueueCommands::List => {
            let queues: Vec<Queue> = list_queues_service(&pool)
                .await
                .context("Error listing queues")?;
            if queues.is_empty() {
                println!("No queues found");
            } else {
                println!(
                    "{:<5} {:<20} {:<12} {:<14} DLQ_ID",
                    "ID", "NAME", "MAX_ATTEMPTS", "VISIBILITY_MS"
                );
                for q in queues {
                    println!(
                        "{:<5} {:<20} {:<12} {:<14} {:?}",
                        q.id, q.name, q.max_attempts, q.visibility_ms, q.dlq_id
                    );
                }
            }
        }
        QueueCommands::Add {
            name,
            max_attempts,
            visibility_ms,
        } => {
            // Create queue via service
            let q = create_queue_service(&pool, &name, max_attempts, visibility_ms)
                .await
                .context("Error creating queue")?;
            println!("Created queue '{}' with ID {}", q.name, q.id);
        }
        QueueCommands::Remove { name } => {
            // Delete queue via service
            let removed = delete_queue_service(&pool, &name)
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
            let q = show_queue_service(&pool, &name)
                .await
                .context("Error fetching queue")?;
            // Compute stats
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
            let ready = db::count_ready_messages(&pool, q.id, now).await?;
            let leased = db::count_leased_messages(&pool, q.id, now).await?;
            let dlq_count = if let Some(dlq_id) = q.dlq_id {
                db::count_messages_by_queue(&pool, dlq_id).await?
            } else {
                0
            };
            println!("Queue '{}' (ID={})", q.name, q.id);
            println!("  max_attempts: {}", q.max_attempts);
            println!("  visibility_ms: {}", q.visibility_ms);
            println!("  dlq_id: {:?}", q.dlq_id);
            println!(
                "Stats: ready={}, leased={}, dlq={}",
                ready, leased, dlq_count
            );
        }
        QueueCommands::Purge { name } => {
            // Purge all messages in the queue
            let deleted = purge_queue_service(&pool, &name)
                .await
                .context("Error purging messages")?;
            println!("Purged {} messages from queue '{}'", deleted, name);
        }
        QueueCommands::Peek { name, limit } => {
            // Peek messages without leasing
            let msgs = peek_queue_service(&pool, &name, limit)
                .await
                .context("Error peeking messages")?;
            for m in msgs {
                println!("[{}] {}", m.id, m.payload_json);
            }
        }
        QueueCommands::RequeueDlq { name } => {
            // Requeue DLQ messages back to main queue
            let moved = requeue_dlq_service(&pool, &name)
                .await
                .context("Error requeueing DLQ messages")?;
            println!("Requeued {} messages from DLQ to queue '{}'", moved, name);
        }
        QueueCommands::Compact { name: _ } => {
            // Compact the SQLite database
            compact_service(&pool)
                .await
                .context("Error compacting database")?;
            println!("Compacted database (VACUUM)");
        }
    }
    Ok(())
}

/// Execute a message command
/// Execute a message command
pub async fn run_message_command(cmd: MessageCommands) -> Result<()> {
    match cmd {
        MessageCommands::Peek { queue, limit } => {
            println!("Peeking {} messages from queue '{}'", limit, queue);
        }
    }
    Ok(())
}
