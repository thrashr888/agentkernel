mod config;
mod vmm;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::Config;
use crate::vmm::VmManager;

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
            dir: _,
        } => {
            let cfg = if let Some(config_path) = config {
                Config::from_file(&config_path)?
            } else {
                Config::minimal(&name, &agent)
            };

            // Check platform
            #[cfg(not(target_os = "linux"))]
            {
                eprintln!("Warning: Firecracker requires Linux with KVM.");
                eprintln!("On macOS, use the Docker-based runner (coming soon).");
                eprintln!();
            }

            let mut manager = VmManager::new()?;

            println!(
                "Creating sandbox '{}' with runtime '{}'...",
                name, cfg.sandbox.runtime
            );
            println!("  vCPUs: {}", cfg.resources.vcpus);
            println!("  Memory: {} MB", cfg.resources.memory_mb);

            manager
                .create(
                    &name,
                    &cfg.sandbox.runtime,
                    cfg.resources.vcpus,
                    cfg.resources.memory_mb,
                )
                .await?;

            println!("\nSandbox '{}' created.", name);
            println!("\nNext steps:");
            println!("  agentkernel start {}", name);
            println!("  agentkernel attach {}", name);
        }
        Commands::Start { name } => {
            let mut manager = VmManager::new()?;

            // Re-create the VM config (in a real impl, we'd persist this)
            manager.create(&name, "base", 1, 512).await?;

            println!("Starting sandbox '{}'...", name);
            manager.start(&name).await?;
            println!("Sandbox '{}' started.", name);
            println!("\nTo attach: agentkernel attach {}", name);
        }
        Commands::Stop { name } => {
            let mut manager = VmManager::new()?;
            manager.create(&name, "base", 1, 512).await?;
            println!("Stopping sandbox '{}'...", name);
            manager.stop(&name).await?;
            println!("Sandbox '{}' stopped.", name);
        }
        Commands::Remove { name } => {
            let mut manager = VmManager::new()?;
            println!("Removing sandbox '{}'...", name);
            manager.remove(&name).await?;
            println!("Sandbox '{}' removed.", name);
        }
        Commands::Attach { name } => {
            let manager = VmManager::new()?;

            if let Some(vm) = manager.get(&name) {
                if !vm.is_running() {
                    bail!(
                        "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                        name,
                        name
                    );
                }

                // TODO: Connect via vsock and spawn interactive shell
                println!("Attaching to sandbox '{}'...", name);
                println!("(vsock communication not yet implemented)");
                println!("\nVM vsock path: {}", vm.vsock_path().display());
            } else {
                bail!("Sandbox '{}' not found", name);
            }
        }
        Commands::Exec { name, command } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }

            let manager = VmManager::new()?;

            if let Some(vm) = manager.get(&name) {
                if !vm.is_running() {
                    bail!(
                        "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                        name,
                        name
                    );
                }

                // TODO: Send command via vsock
                println!("Executing in sandbox '{}': {:?}", name, command);
                println!("(vsock communication not yet implemented)");
            } else {
                bail!("Sandbox '{}' not found", name);
            }
        }
        Commands::List => {
            let manager = VmManager::new()?;
            let vms = manager.list();

            if vms.is_empty() {
                println!("No sandboxes found.");
                println!("\nCreate one with: agentkernel create <name>");
            } else {
                println!("{:<20} {:<10}", "NAME", "STATUS");
                for (name, running) in vms {
                    let status = if running { "running" } else { "stopped" };
                    println!("{:<20} {:<10}", name, status);
                }
            }
        }
    }

    Ok(())
}
