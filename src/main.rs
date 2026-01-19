mod config;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::Config;

#[derive(Parser)]
#[command(name = "agentkernel")]
#[command(about = "Run AI coding agents in secure, isolated microVMs")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new agentkernel.toml in the current directory
    Init {
        /// Name of the sandbox (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
    },
    /// Create a new sandbox (microVM)
    Create {
        /// Name of the sandbox
        name: String,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
        /// Path to agentkernel.toml config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Project directory to mount into sandbox
        #[arg(short, long)]
        dir: Option<PathBuf>,
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
    /// Remove a sandbox
    Remove {
        /// Name of the sandbox to remove
        name: String,
    },
    /// Attach to a running sandbox (opens interactive shell)
    Attach {
        /// Name of the sandbox to attach to
        name: String,
    },
    /// Execute a command in a running sandbox
    Exec {
        /// Name of the sandbox
        name: String,
        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// List all sandboxes
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { name, agent } => {
            let current_dir = std::env::current_dir()?;
            let sandbox_name = name.unwrap_or_else(|| {
                current_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "my-sandbox".to_string())
            });

            let config_path = current_dir.join("agentkernel.toml");
            if config_path.exists() {
                bail!("agentkernel.toml already exists in this directory");
            }

            let config_content = format!(
                r#"# Agentkernel configuration
# See: https://github.com/thrashr888/agentkernel

[sandbox]
name = "{}"
runtime = "base"    # base, python, node, go, rust

[agent]
preferred = "{}"    # claude, gemini, codex, opencode

[resources]
vcpus = 1
memory_mb = 512
"#,
                sandbox_name, agent
            );

            std::fs::write(&config_path, config_content)?;
            println!("Created agentkernel.toml for sandbox '{}'", sandbox_name);
            println!("\nNext steps:");
            println!("  agentkernel create {} --dir .", sandbox_name);
            println!("  agentkernel start {}", sandbox_name);
            println!("  agentkernel attach {}", sandbox_name);
        }
        Commands::Create {
            name,
            agent,
            config,
            dir,
        } => {
            let cfg = if let Some(config_path) = config {
                Config::from_file(&config_path)?
            } else {
                Config::minimal(&name, &agent)
            };

            // TODO: Implement Firecracker VMM
            // 1. Create VM configuration
            // 2. Set kernel and rootfs paths based on cfg.sandbox.runtime
            // 3. Configure vsock for communication
            // 4. Create the microVM

            println!(
                "Creating sandbox '{}' with runtime '{}'...",
                name, cfg.sandbox.runtime
            );
            println!("  vCPUs: {}", cfg.resources.vcpus);
            println!("  Memory: {} MB", cfg.resources.memory_mb);
            if let Some(d) = dir {
                println!("  Project: {}", d.display());
            }
            println!();
            bail!("Firecracker VMM not yet implemented. See: plan/firecracker-pivot.md");
        }
        Commands::Start { name } => {
            // TODO: Implement Firecracker VMM
            // 1. Start the microVM
            // 2. Wait for guest agent to be ready
            bail!("Firecracker VMM not yet implemented. Sandbox: {}", name);
        }
        Commands::Stop { name } => {
            // TODO: Implement Firecracker VMM
            // 1. Send shutdown signal via vsock
            // 2. Wait for VM to terminate
            bail!("Firecracker VMM not yet implemented. Sandbox: {}", name);
        }
        Commands::Remove { name } => {
            // TODO: Implement Firecracker VMM
            // 1. Stop VM if running
            // 2. Clean up resources
            bail!("Firecracker VMM not yet implemented. Sandbox: {}", name);
        }
        Commands::Attach { name } => {
            // TODO: Implement vsock communication
            // 1. Connect to VM's vsock
            // 2. Start interactive shell via guest agent
            bail!("Firecracker VMM not yet implemented. Sandbox: {}", name);
        }
        Commands::Exec { name, command } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }

            // TODO: Implement vsock communication
            // 1. Connect to VM's vsock
            // 2. Send command to guest agent
            // 3. Stream stdout/stderr back
            bail!(
                "Firecracker VMM not yet implemented. Sandbox: {}, Command: {:?}",
                name,
                command
            );
        }
        Commands::List => {
            // TODO: Implement Firecracker VMM
            // 1. List running microVMs
            println!("No sandboxes found.");
            println!("\nFirecracker VMM not yet implemented.");
            println!("See: plan/firecracker-pivot.md");
        }
    }

    Ok(())
}
