mod agent;
mod config;
mod mcp;
mod model;
mod orchestration;
mod planner;
mod project_context;
mod roles;
mod state;
mod tools;
mod verification;
mod workflow;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;

#[derive(Parser)]
#[command(name = "harness", version, about = "Локальный LLM harness")]
struct Cli { #[command(subcommand)] command: Commands }

#[derive(Subcommand)]
enum Commands {
    Run { prompt: String },
    Delegate { role: String, task: String },
    Task { prompt: String },
    Models,
    Tools,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    match cli.command {
        Commands::Run { prompt } => agent::run(&config, &prompt).await?,
        Commands::Delegate { role, task } => {
            let workspace = config.workspace.path.canonicalize()?;
            let result = orchestration::run_named(&config, &role, &workspace, &task).await?;
            println!("{result}");
        }
        Commands::Task { prompt } => {
            let workspace = config.workspace.path.canonicalize()?;
            let result = workflow::run(&config, &workspace, &prompt).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Models => model::list_models(&config).await?,
        Commands::Tools => {
            for tool in tools::ToolRegistry::native().all() { println!("{} - {}", tool.name, tool.description); }
            for server in &config.mcp { println!("mcp:{}", server.name); }
        }
    }
    Ok(())
}
