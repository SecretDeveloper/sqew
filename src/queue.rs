use clap::Subcommand;

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
use crate::models::Queue;
use sqlx::SqlitePool;
use std::env;
use anyhow::{Context, Result};

/// Execute a queue command
pub async fn run_queue_command(cmd: QueueCommands) -> Result<()> {
    // Initialize database connection
    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let db_file = current_dir.join("sqew.db");
    if !db_file.exists() {
        std::fs::File::create(&db_file)
            .with_context(|| format!("Failed to create DB file at {}", db_file.display()))?;
    }
    let db_url = format!("sqlite://{}", db_file.to_string_lossy());
    let pool = SqlitePool::connect(&db_url)
        .await
        .context("Failed to connect to the database")?;

    match cmd {
        QueueCommands::List => {
            let queues: Vec<Queue> = db::list_queues(&pool)
                .await
                .context("Error listing queues")?;
            if queues.is_empty() {
                println!("No queues found");
            } else {
                println!("{:<5} {:<20} {:<12} {:<14} {}", "ID", "NAME", "MAX_ATTEMPTS", "VISIBILITY_MS", "DLQ_ID");
                for q in queues {
                    println!(
                        "{:<5} {:<20} {:<12} {:<14} {:?}",
                        q.id,
                        q.name,
                        q.max_attempts,
                        q.visibility_ms,
                        q.dlq_id
                    );
                }
            }
        }
        QueueCommands::Add { name, max_attempts, visibility_ms } => {
            // Check for existing queue
            if let Some(_) = db::get_queue_by_name(&pool, &name)
                .await
                .context("Error checking existing queue")? {
                eprintln!("Queue '{}' already exists", name);
                std::process::exit(1);
            }
            let id = db::create_queue(&pool, &name, None, max_attempts, visibility_ms)
                .await
                .context("Error creating queue")?;
            println!("Created queue '{}' with ID {}", name, id);
        }
        QueueCommands::Remove { name } => {
            let deleted = db::delete_queue_by_name(&pool, &name)
                .await
                .context("Error removing queue")?;
            if deleted > 0 {
                println!("Removed queue '{}'", name);
            } else {
                eprintln!("Queue '{}' not found", name);
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// Execute a message command
pub async fn run_message_command(cmd: MessageCommands) -> anyhow::Result<()> {
    match cmd {
        MessageCommands::Peek { queue, limit } => {
            println!("Peeking {} messages from queue '{}'", limit, queue);
        }
    }
    Ok(())
}