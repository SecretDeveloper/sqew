use clap::Parser;
use sqew::cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    cli.run().await
}
