mod agent;
mod config;
mod mcp;
mod model;
mod state;
mod tools;
mod verification;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;

#[derive(Parser)]
#[command(name = "harness", version, about = "Local LLM harness")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run { prompt: String },
    Models,
    Tools,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;

    match cli.command {
        Commands::Run { prompt } => agent::run(&config, &prompt).await?,
        Commands::Models => model::list_models(&config).await?,
        Commands::Tools => {
            for tool in tools::ToolRegistry::native().all() {
                println!("{} - {}", tool.name, tool.description);
            }
            for server in &config.mcp {
                println!("mcp:{}", server.name);
            }
        }
    }
    Ok(())
}
