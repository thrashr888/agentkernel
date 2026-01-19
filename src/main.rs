use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agentkernel")]
#[command(about = "A Firecracker-inspired microkernel for AI coding agents")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new sandbox environment
    Create {
        /// Name of the sandbox
        name: String,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
    },
    /// Start a sandbox
    Start {
        /// Name of the sandbox to start
        name: String,
    },
    /// Stop a running sandbox
    Stop {
        /// Name of the sandbox to stop
        name: String,
    },
    /// Attach to a running sandbox
    Attach {
        /// Name of the sandbox to attach to
        name: String,
    },
    /// List all sandboxes
    List,
    /// Configure sandbox settings
    Config {
        /// Name of the sandbox
        name: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Create { name, agent } => {
            println!("Creating sandbox '{}' for agent '{}'...", name, agent);
            // TODO: Implement sandbox creation
        }
        Commands::Start { name } => {
            println!("Starting sandbox '{}'...", name);
            // TODO: Implement sandbox start
        }
        Commands::Stop { name } => {
            println!("Stopping sandbox '{}'...", name);
            // TODO: Implement sandbox stop
        }
        Commands::Attach { name } => {
            println!("Attaching to sandbox '{}'...", name);
            // TODO: Implement sandbox attach
        }
        Commands::List => {
            println!("Listing sandboxes...");
            // TODO: Implement sandbox listing
        }
        Commands::Config { name } => {
            println!("Configuring sandbox '{}'...", name);
            // TODO: Implement sandbox configuration
        }
    }

    Ok(())
}
