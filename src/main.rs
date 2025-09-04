mod cli;
mod db;
mod models;
mod queue;
mod server;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    cli.run().await
}
