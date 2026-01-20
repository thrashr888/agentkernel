mod agents;
mod config;
mod docker_backend;
mod firecracker_client;
mod http_api;
mod languages;
mod mcp;
mod permissions;
mod seatbelt;
mod setup;
mod validation;
mod vmm;
mod vsock;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::config::Config;
use crate::setup::{check_installation, run_setup};
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
    /// Set up agentkernel (download kernel, rootfs, Firecracker)
    Setup {
        /// Run non-interactively with defaults
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Show installation status
    Status,
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
    /// Run a command in a temporary sandbox (create, start, exec, stop, remove)
    Run {
        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
        /// Path to agentkernel.toml config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Keep the sandbox after execution (don't remove)
        #[arg(short, long)]
        keep: bool,
        /// Docker image to use (overrides config)
        #[arg(short, long)]
        image: Option<String>,
        /// Security profile: permissive, moderate (default), restrictive
        #[arg(short, long, default_value = "moderate")]
        profile: String,
        /// Disable network access
        #[arg(long)]
        no_network: bool,
    },
    /// Start MCP server for Claude Code integration (JSON-RPC over stdio)
    McpServer,
    /// Start HTTP API server for programmatic access
    Serve {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
    /// List supported AI agents and their availability
    Agents,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup { yes } => {
            run_setup(yes).await?;
        }
        Commands::Status => {
            let status = check_installation();
            status.print();

            if status.is_ready() {
                println!("\nAgentkernel is ready to use!");
            } else {
                println!("\nRun 'agentkernel setup' to complete installation.");
            }
        }
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
            // Validate sandbox name first (security: prevents command injection)
            validation::validate_sandbox_name(&name)?;

            // Check setup status first
            let status = check_installation();
            if !status.is_ready() {
                bail!(
                    "Agentkernel is not fully set up. Run 'agentkernel setup' first.\n\
                     Missing: {}",
                    missing_components(&status)
                );
            }

            let cfg = if let Some(config_path) = config {
                Config::from_file(&config_path)?
            } else {
                Config::minimal(&name, &agent)
            };

            let mut manager = VmManager::new()?;

            let docker_image = cfg.docker_image();
            println!(
                "Creating sandbox '{}' with image '{}'...",
                name, docker_image
            );
            println!("  vCPUs: {}", cfg.resources.vcpus);
            println!("  Memory: {} MB", cfg.resources.memory_mb);

            manager
                .create(
                    &name,
                    &docker_image,
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
            validation::validate_sandbox_name(&name)?;

            let status = check_installation();
            if !status.is_ready() {
                bail!("Agentkernel is not fully set up. Run 'agentkernel setup' first.");
            }

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!(
                    "Sandbox '{}' not found. Create it first with: agentkernel create {}",
                    name,
                    name
                );
            }

            println!("Starting sandbox '{}'...", name);
            manager.start(&name).await?;
            println!("Sandbox '{}' started.", name);
            println!("\nTo attach: agentkernel attach {}", name);
        }
        Commands::Stop { name } => {
            validation::validate_sandbox_name(&name)?;

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!("Sandbox '{}' not found", name);
            }

            println!("Stopping sandbox '{}'...", name);
            manager.stop(&name).await?;
            println!("Sandbox '{}' stopped.", name);
        }
        Commands::Remove { name } => {
            validation::validate_sandbox_name(&name)?;

            let mut manager = VmManager::new()?;
            println!("Removing sandbox '{}'...", name);
            manager.remove(&name).await?;
            println!("Sandbox '{}' removed.", name);
        }
        Commands::Attach { name } => {
            validation::validate_sandbox_name(&name)?;

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
            validation::validate_sandbox_name(&name)?;

            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!("Sandbox '{}' not found", name);
            }

            let output = manager.exec_cmd(&name, &command).await?;
            print!("{}", output);
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
        Commands::Run {
            command,
            config,
            keep,
            image,
            profile,
            no_network,
        } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel run [OPTIONS] <command...>");
            }

            // Generate a unique sandbox name
            let run_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
            let sandbox_name = format!("run-{}", run_id);

            // Determine Docker image: --image > --config > command > ./agentkernel.toml > project files > default
            // For `run`, command detection has higher priority than project files
            // because user is explicitly specifying what to run
            let docker_image = if let Some(img) = image {
                img
            } else if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                cfg.docker_image()
            } else if let Some(img) = languages::detect_from_command(&command) {
                // Command-based detection first for `run`
                img
            } else {
                // Try current directory config
                let default_config = PathBuf::from("agentkernel.toml");
                if default_config.exists() {
                    let cfg = Config::from_file(&default_config)?;
                    cfg.docker_image()
                } else {
                    // Fall back to project file detection or default
                    languages::detect_image(&command)
                }
            };

            // Get permissions from profile
            let mut perms = permissions::SecurityProfile::from_str(&profile)
                .unwrap_or_default()
                .permissions();

            // Apply --no-network override
            if no_network {
                perms.network = false;
            }

            // Apply config overrides if present
            if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                let cfg_perms = cfg.get_permissions();
                // Config overrides take precedence over CLI profile
                if cfg.security.network.is_some() {
                    perms.network = cfg_perms.network;
                }
            }

            let mut manager = VmManager::new()?;

            // Create
            manager.create(&sandbox_name, &docker_image, 1, 512).await?;

            // Start with permissions
            if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
                // Cleanup on failure
                let _ = manager.remove(&sandbox_name).await;
                bail!("Failed to start sandbox: {}", e);
            }

            // Execute command
            let result = manager.exec_cmd(&sandbox_name, &command).await;

            // Print output
            match &result {
                Ok(output) => print!("{}", output),
                Err(e) => eprintln!("Error: {}", e),
            }

            // Stop
            let _ = manager.stop(&sandbox_name).await;

            // Remove (unless --keep)
            if !keep {
                let _ = manager.remove(&sandbox_name).await;
            } else {
                println!(
                    "\nSandbox '{}' kept. Remove with: agentkernel remove {}",
                    sandbox_name, sandbox_name
                );
            }

            // Return error if command failed
            result?;
        }
        Commands::McpServer => {
            mcp::run_server().await?;
        }
        Commands::Serve { host, port } => {
            let addr: std::net::SocketAddr = format!("{}:{}", host, port)
                .parse()
                .expect("Invalid address");
            http_api::run_server(addr).await?;
        }
        Commands::Agents => {
            println!("{:<15} {:<15} API KEY", "AGENT", "STATUS");
            println!("{:-<45}", "");
            for status in agents::list_agents() {
                let install_status = if status.installed {
                    "installed"
                } else {
                    "not installed"
                };
                let key_status = if status.api_key_set { "set" } else { "missing" };
                println!(
                    "{:<15} {:<15} {}",
                    status.agent_type.name(),
                    install_status,
                    key_status
                );
                if !status.installed {
                    println!("  → {}", status.install_instructions);
                }
            }
        }
    }

    Ok(())
}

fn missing_components(status: &setup::SetupStatus) -> String {
    let mut missing = Vec::new();
    if !status.kernel_installed {
        missing.push("kernel");
    }
    if !status.rootfs_base_installed {
        missing.push("rootfs");
    }
    if !status.firecracker_installed {
        missing.push("firecracker");
    }
    if !status.kvm_available && !status.docker_available {
        missing.push("KVM or Docker");
    }
    missing.join(", ")
}
