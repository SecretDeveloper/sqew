use clap::{Parser, Subcommand};
use crate::server;
use crate::queue::{self, MessageCommands, QueueCommands};

/// Sqew CLI interface
#[derive(Parser, Debug)]
#[command(name = "sqew", about = "Sqew CLI tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the HTTP server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8888)]
        port: u16,
    },
    /// Queue management commands
    #[command(subcommand)]
    Queue(QueueCommands),
    /// Message commands
    #[command(subcommand)]
    Message(MessageCommands),
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Commands::Serve { port } => server::run_server(port).await,
            Commands::Queue(cmd) => queue::run_queue_command(cmd).await,
            Commands::Message(cmd) => queue::run_message_command(cmd).await,
        }
    }
}