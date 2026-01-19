mod config;
mod docker;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::docker::DockerManager;

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
    /// Initialize a new agentkernel.toml in the current directory
    Init {
        /// Name of the sandbox (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
    },
    /// Create a new sandbox environment
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
        /// Force removal even if running
        #[arg(short, long)]
        force: bool,
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
    tracing_subscriber::fmt::init();

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
base_image = "ubuntu:24.04"

[agent]
preferred = "{}"

[dependencies]
system = ["git", "curl"]

[scripts]
# setup = "echo 'Setting up...'"
# test = "echo 'Running tests...'"

[mounts]
"." = "/app"

[network]
ports = []

[resources]
memory_mb = 2048
cpu_limit = 2.0
"#,
                sandbox_name, agent
            );

            std::fs::write(&config_path, config_content)?;
            println!("Created agentkernel.toml for sandbox '{}'", sandbox_name);
            println!("\nNext steps:");
            println!(
                "  agentkernel create {} --config agentkernel.toml --dir .",
                sandbox_name
            );
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

            let docker = DockerManager::new().await?;
            let project_dir = dir.as_deref();
            let container_id = docker.create_sandbox(&cfg, project_dir).await?;
            println!(
                "Created sandbox '{}' (container: {})",
                name,
                &container_id[..12]
            );
            println!("\nNext steps:");
            println!("  agentkernel start {}", name);
            println!("  agentkernel attach {}", name);
        }
        Commands::Start { name } => {
            let docker = DockerManager::new().await?;
            docker.start_sandbox(&name).await?;
            println!("Started sandbox '{}'", name);
            println!("\nTo attach: agentkernel attach {}", name);
        }
        Commands::Stop { name } => {
            let docker = DockerManager::new().await?;
            docker.stop_sandbox(&name).await?;
            println!("Stopped sandbox '{}'", name);
        }
        Commands::Remove { name, force: _ } => {
            let docker = DockerManager::new().await?;
            docker.remove_sandbox(&name).await?;
            println!("Removed sandbox '{}'", name);
        }
        Commands::Attach { name } => {
            let docker = DockerManager::new().await?;

            // Check if sandbox is running
            if !docker.is_sandbox_running(&name).await? {
                bail!(
                    "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                    name,
                    name
                );
            }

            let container_name = DockerManager::container_name(&name);
            println!("Attaching to sandbox '{}' (Ctrl+D to detach)...\n", name);

            // Spawn docker exec with interactive TTY
            let status = Command::new("docker")
                .args(["exec", "-it", &container_name, "/bin/bash"])
                .status()?;

            if !status.success() {
                // Try sh if bash isn't available
                let status = Command::new("docker")
                    .args(["exec", "-it", &container_name, "/bin/sh"])
                    .status()?;

                if !status.success() {
                    bail!("Failed to attach to sandbox");
                }
            }
        }
        Commands::Exec { name, command } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }

            let docker = DockerManager::new().await?;

            // Check if sandbox is running
            if !docker.is_sandbox_running(&name).await? {
                bail!(
                    "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                    name,
                    name
                );
            }

            let container_name = DockerManager::container_name(&name);

            // Build docker exec command - use -it only if we have a TTY
            use std::io::IsTerminal;
            let mut args = if std::io::stdin().is_terminal() {
                vec!["exec", "-it", &container_name]
            } else {
                vec!["exec", &container_name]
            };
            let cmd_strs: Vec<&str> = command.iter().map(|s| s.as_str()).collect();
            args.extend(cmd_strs);

            let status = Command::new("docker").args(&args).status()?;

            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Commands::List => {
            let docker = DockerManager::new().await?;
            let sandboxes = docker.list_sandboxes().await?;

            if sandboxes.is_empty() {
                println!("No sandboxes found.");
                println!("\nCreate one with: agentkernel create <name>");
            } else {
                println!(
                    "{:<20} {:<12} {:<10} {:<20} IMAGE",
                    "NAME", "CONTAINER", "AGENT", "STATUS"
                );
                for s in sandboxes {
                    println!(
                        "{:<20} {:<12} {:<10} {:<20} {}",
                        s.name, s.container_id, s.agent, s.status, s.image
                    );
                }
            }
        }
    }

    Ok(())
}
