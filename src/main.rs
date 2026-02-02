mod agents;
mod apple_backend;
mod asciicast;
mod audit;
mod backend;
mod benchmark;
mod build;
mod config;
mod daemon;
mod docker_backend;
mod firecracker_client;
mod git_utils;
mod http_api;
mod hyperlight_backend;
mod languages;
mod mcp;
mod permissions;
mod pipeline;
mod plugin_installer;
mod pool;
mod rootfs;
mod sandbox_pool;
mod seatbelt;
mod secrets;
mod session;
mod setup;
mod snapshot;
mod stats;
mod template;
mod validation;
mod vmm;
mod vsock;

// Enterprise modules (behind feature flag)
// identity has public API surface for CLI login, middleware, and Cedar helpers
// not all consumed from the HTTP API yet
#[cfg(feature = "enterprise")]
#[allow(dead_code)]
mod identity;
#[cfg(feature = "enterprise")]
pub mod policy;

use anyhow::{Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use std::path::{Path, PathBuf};

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
        /// Template to use (built-in name, local name, github:owner/repo/path, or file path)
        #[arg(short, long)]
        template: Option<String>,
    },
    /// Create a new sandbox (microVM)
    Create {
        /// Name of the sandbox (auto-derived from git when --branch is used)
        name: Option<String>,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
        /// Path to agentkernel.toml config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Project directory to mount into sandbox
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad (default: auto-detect)
        #[arg(short = 'B', long)]
        backend: Option<String>,
        /// Template to use (built-in name, local name, github:owner/repo/path, or file path)
        #[arg(long)]
        template: Option<String>,
        /// Time-to-live (e.g. 1h, 30m, 3d, 0 for no expiry)
        #[arg(long)]
        ttl: Option<String>,
        /// Auto-name sandbox from git project + branch (e.g. myproject-feature-auth)
        #[arg(long)]
        branch: bool,
    },
    /// Start a sandbox
    Start {
        /// Name of the sandbox to start
        name: String,
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad (default: auto-detect)
        #[arg(short = 'B', long)]
        backend: Option<String>,
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
        /// Environment variables to set (KEY=VALUE format, can be repeated)
        #[arg(short, long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Record session to asciicast v2 file (for replay with asciinema)
        #[arg(long)]
        record: Option<PathBuf>,
    },
    /// Execute a command in a running sandbox
    Exec {
        /// Name of the sandbox
        name: String,
        /// Environment variables to set (KEY=VALUE format, can be repeated)
        #[arg(short, long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Copy files to/from a running sandbox
    ///
    /// Examples:
    ///   agentkernel cp ./local/file my-sandbox:/remote/path
    ///   agentkernel cp my-sandbox:/remote/path ./local/file
    Cp {
        /// Source path (./local/file or sandbox:/path)
        source: String,
        /// Destination path (./local/file or sandbox:/path)
        dest: String,
    },
    /// List all sandboxes
    List {
        /// Filter to sandboxes matching the current git project
        #[arg(long)]
        project: bool,
    },
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
        /// Use container pool for faster execution (skips create/destroy overhead)
        #[arg(short = 'F', long)]
        fast: bool,
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad (default: auto-detect)
        #[arg(short = 'B', long)]
        backend: Option<String>,
        /// Template to use (built-in name, local name, github:owner/repo/path, or file path)
        #[arg(long)]
        template: Option<String>,
        /// Time-to-live for kept sandboxes (e.g. 1h, 30m, 3d, 0 for no expiry; default: 1h for run)
        #[arg(long)]
        ttl: Option<String>,
        /// Use git project+branch as sandbox name (reuses existing sandbox for same branch)
        #[arg(long)]
        branch: bool,
    },
    /// Start MCP server for Claude Code integration (JSON-RPC over stdio)
    McpServer,
    /// Start HTTP API server for programmatic access
    Serve {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on
        #[arg(short, long, default_value = "18888")]
        port: u16,
    },
    /// List supported AI agents and their availability
    Agents,
    /// Manage agent plugins (install integration files for Claude, Codex, Gemini, etc.)
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Manage the daemon (VM pool server)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// View audit log
    Audit {
        /// Show only events for this sandbox
        #[arg(short, long)]
        sandbox: Option<String>,
        /// Show last N entries (default: 20)
        #[arg(short, long, default_value = "20")]
        last: usize,
        /// Show full log path
        #[arg(long)]
        path: bool,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// Replay a recorded session (asciicast v2 format)
    Replay {
        /// Path to the asciicast file
        file: PathBuf,
        /// Playback speed multiplier (1.0 = realtime, 2.0 = 2x speed)
        #[arg(short, long, default_value = "1.0")]
        speed: f64,
        /// Maximum time between frames in seconds (for idle time)
        #[arg(long, default_value = "2.0")]
        max_idle: f64,
    },
    /// Manage sandbox templates
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// System diagnostics and health check
    Doctor,
    /// Show usage statistics from audit log
    Stats {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// Manage secrets (API keys and credentials)
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Garbage-collect expired sandboxes
    Gc {
        /// Show what would be removed without removing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show detailed information about a sandbox
    Info {
        /// Name of the sandbox
        name: String,
    },
    /// Benchmark sandbox backends on your hardware
    Benchmark {
        /// Comma-separated backends to test (default: all available)
        #[arg(short, long)]
        backends: Option<String>,
        /// Number of iterations per backend (default: 1)
        #[arg(short, long, default_value = "1")]
        iterations: usize,
        /// Docker image to use for benchmark
        #[arg(long, default_value = "alpine:3.20")]
        image: String,
    },
    /// Run a multi-step agent pipeline (chain sandboxes with data flow)
    Pipeline {
        /// Path to pipeline.toml file
        file: PathBuf,
        /// Backend to use for pipeline sandboxes
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// Manage agent sessions (tied sandbox + agent lifecycle)
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Snapshot a sandbox (save its current state for later restore)
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },
    /// Restore a sandbox from a snapshot
    Restore {
        /// Name of the snapshot to restore
        name: String,
        /// Name for the restored sandbox (defaults to original name + "-restored")
        #[arg(long, value_name = "NAME")]
        r#as: Option<String>,
        /// Backend to use for the restored sandbox
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// Remove all sandboxes and agentkernel Docker artifacts to free disk space
    Clean {
        /// Also stop and remove running sandboxes
        #[arg(short, long)]
        force: bool,
        /// Remove Docker images and build cache too
        #[arg(long)]
        all: bool,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Enterprise policy management (requires --features enterprise)
    #[cfg(feature = "enterprise")]
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
}

#[derive(Subcommand)]
enum SecretAction {
    /// Set a secret value (reads from stdin if value not provided)
    Set {
        /// Secret key name (e.g. ANTHROPIC_API_KEY)
        key: String,
        /// Secret value (omit to read from stdin)
        value: Option<String>,
    },
    /// Get a secret value
    Get {
        /// Secret key name
        key: String,
    },
    /// List all stored secrets
    List,
    /// Delete a secret
    Delete {
        /// Secret key name
        key: String,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Start a new agent session (creates sandbox + session metadata)
    Start {
        /// Session name
        #[arg(short, long)]
        name: String,
        /// Agent type (claude, gemini, codex, opencode)
        #[arg(short, long, default_value = "claude")]
        agent: String,
        /// Docker image to use
        #[arg(short, long)]
        image: Option<String>,
        /// Backend to use
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// List all sessions
    List,
    /// Stop a session (stops sandbox, keeps session metadata)
    Stop {
        /// Session name
        name: String,
    },
    /// Save a session (snapshot sandbox + mark session as saved)
    Save {
        /// Session name
        name: String,
    },
    /// Resume a saved or stopped session
    Resume {
        /// Session name
        name: String,
    },
    /// Delete a session and its sandbox
    Delete {
        /// Session name
        name: String,
    },
}

#[derive(Subcommand)]
enum SnapshotAction {
    /// Take a snapshot of a sandbox
    Take {
        /// Name of the sandbox to snapshot
        sandbox: String,
        /// Snapshot name (defaults to sandbox-YYYYMMDD)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// List all snapshots
    List,
    /// Delete a snapshot
    Delete {
        /// Name of the snapshot to delete
        name: String,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// List available templates
    List,
    /// Save a running sandbox's config as a template
    Save {
        /// Template name
        name: String,
        /// Sandbox to save config from
        #[arg(long)]
        from: String,
    },
    /// Fetch and cache a GitHub template
    Add {
        /// Template specifier (github:owner/repo/path)
        specifier: String,
    },
    /// Remove a local template
    Remove {
        /// Template name
        name: String,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// Install plugin files for an agent integration
    Install {
        /// Agent target: claude, codex, gemini, opencode, mcp, or all
        target: String,
        /// Install to user-level config instead of current directory
        #[arg(short, long)]
        global: bool,
        /// Overwrite existing files without prompting
        #[arg(short, long)]
        force: bool,
        /// Show what would be written without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// List available plugins and their install status
    List,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon in foreground
    Start {
        /// Run in background (daemonize)
        #[arg(short, long)]
        background: bool,
    },
    /// Stop the running daemon
    Stop,
    /// Show daemon status
    Status,
}

/// Enterprise policy management subcommands
#[cfg(feature = "enterprise")]
#[derive(Subcommand)]
enum PolicyAction {
    /// Check if an action would be permitted by the policy engine
    Check {
        /// Action to check: run, exec, create, attach, mount, network
        #[arg(short, long)]
        action: String,
        /// Sandbox name to check against
        #[arg(short, long)]
        sandbox: String,
    },
    /// Show policy engine status (version, offline mode, server)
    Status,
    /// Show recent policy audit log entries
    AuditLog {
        /// Number of entries to show
        #[arg(short, long, default_value = "20")]
        last: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
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
        Commands::Template { action } => match action {
            TemplateAction::List => {
                let templates = template::list_all();
                if templates.is_empty() {
                    println!("No templates available.");
                } else {
                    println!(
                        "{:<20} {:<10} {:<28} {:<14} {:>6}",
                        "NAME", "SOURCE", "IMAGE", "PROFILE", "MEMORY"
                    );
                    for t in &templates {
                        println!(
                            "{:<20} {:<10} {:<28} {:<14} {:>4}MB",
                            t.name, t.source, t.image, t.profile, t.memory_mb
                        );
                    }
                }
            }
            TemplateAction::Save { name, from } => {
                let manager = VmManager::new()?;
                let vms = manager.list();
                if !vms.iter().any(|(n, _, _)| *n == from) {
                    bail!("sandbox '{}' not found", from);
                }
                // Load the sandbox's config from its state dir
                let data_dir = setup::default_data_dir();
                let state_path = data_dir.join("sandboxes").join(&from).join("state.json");
                let cfg = if state_path.exists() {
                    let content = std::fs::read_to_string(&state_path)?;
                    let state: serde_json::Value = serde_json::from_str(&content)?;
                    if let Some(image) = state.get("image").and_then(|v| v.as_str()) {
                        let mut cfg = Config::minimal(&from, "claude");
                        cfg.sandbox.base_image = Some(image.to_string());
                        cfg
                    } else {
                        Config::minimal(&from, "claude")
                    }
                } else {
                    // Try loading from the project's agentkernel.toml if it exists
                    let project_config = PathBuf::from("agentkernel.toml");
                    if project_config.exists() {
                        Config::from_file(&project_config)?
                    } else {
                        Config::minimal(&from, "claude")
                    }
                };
                let path = template::save(&name, &cfg)?;
                println!("Template '{}' saved to {}", name, path.display());
            }
            TemplateAction::Add { specifier } => {
                let resolved = template::add_github(&specifier)?;
                println!(
                    "Template '{}' fetched and cached ({})",
                    resolved.name, resolved.source
                );
            }
            TemplateAction::Remove { name } => {
                template::remove(&name)?;
                println!("Template '{}' removed", name);
            }
        },
        Commands::Doctor => {
            run_doctor().await?;
        }
        Commands::Stats { json } => {
            let s = stats::compute_stats()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                s.print();
            }
        }
        Commands::Init {
            name,
            agent,
            template: tmpl,
        } => {
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

            let config_content = if let Some(tmpl_name) = tmpl {
                let resolved = template::resolve(&tmpl_name)?;
                println!("Using template '{}' ({})", resolved.name, resolved.source);
                // Parse, override sandbox name, re-serialize
                let mut cfg = resolved.parse()?;
                cfg.sandbox.name = sandbox_name.clone();
                format!(
                    "# Generated from template: {}\n{}",
                    resolved.name,
                    toml::to_string_pretty(&cfg)?
                )
            } else {
                format!(
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
                )
            };

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
            backend,
            template: tmpl,
            ttl,
            branch,
        } => {
            // Resolve sandbox name: --branch auto-derives from git, otherwise require explicit name
            let name = if branch {
                if name.is_some() {
                    bail!("Cannot use both --branch and an explicit name");
                }
                let ctx = git_utils::detect()
                    .map_err(|_| anyhow::anyhow!("--branch requires a git repository"))?;
                let derived = ctx.sandbox_name();
                println!("Using git-derived sandbox name: {}", derived);
                derived
            } else {
                name.ok_or_else(|| {
                    anyhow::anyhow!("Sandbox name required. Use --branch to auto-derive from git.")
                })?
            };

            // Validate sandbox name (security: prevents command injection)
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

            // Load config: --config > --template > minimal default
            let (cfg, config_base_dir) = if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                let base_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                (cfg, Some(base_dir))
            } else if let Some(ref tmpl_name) = tmpl {
                let resolved = template::resolve(tmpl_name)?;
                println!("Using template '{}' ({})", resolved.name, resolved.source);
                let mut cfg = resolved.parse()?;
                cfg.sandbox.name = name.clone();
                (cfg, None)
            } else {
                (Config::minimal(&name, &agent), None)
            };

            // Validate config and print warnings
            for warning in cfg.validate() {
                eprintln!("Warning: {}", warning);
            }

            // Parse backend option if provided
            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };
            let mut manager = VmManager::with_backend(backend_type)?;

            // Build from Dockerfile if configured, otherwise use base image
            let docker_image = if let Some(ref base_dir) = config_base_dir {
                let base_image = cfg.docker_image();
                build::build_or_use_image(&name, &base_image, base_dir, &cfg)?
            } else {
                cfg.docker_image()
            };

            println!(
                "Creating sandbox '{}' with image '{}'...",
                name, docker_image
            );
            println!("  vCPUs: {}", cfg.resources.vcpus);
            println!("  Memory: {} MB", cfg.resources.memory_mb);

            let ttl_secs = ttl.map(|t| parse_ttl(&t)).transpose()?;
            manager
                .create_with_ttl(
                    &name,
                    &docker_image,
                    cfg.resources.vcpus,
                    cfg.resources.memory_mb,
                    ttl_secs,
                )
                .await?;

            println!("\nSandbox '{}' created.", name);
            if let Some(secs) = ttl_secs {
                println!("  TTL: {} (expires automatically)", format_ttl(secs));
            }
            println!("\nNext steps:");
            println!("  agentkernel start {}", name);
            println!("  agentkernel attach {}", name);
        }
        Commands::Start { name, backend } => {
            validation::validate_sandbox_name(&name)?;

            let status = check_installation();
            if !status.is_ready() {
                bail!("Agentkernel is not fully set up. Run 'agentkernel setup' first.");
            }

            // Parse backend option if provided
            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };
            let mut manager = VmManager::with_backend(backend_type)?;

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
        Commands::Attach { name, env, record } => {
            validation::validate_sandbox_name(&name)?;

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!("Sandbox '{}' not found", name);
            }

            if !manager.is_running(&name) {
                bail!(
                    "Sandbox '{}' is not running. Start it with: agentkernel start {}",
                    name,
                    name
                );
            }

            // Set up recording if requested
            let record_path = record.map(|p| {
                if p.is_dir() {
                    p.join(asciicast::generate_recording_name(&name))
                } else {
                    p
                }
            });

            // Use a temp file for raw script output, then convert to asciicast
            let script_tmp = record_path.as_ref().map(|p| p.with_extension("typescript"));

            if let Some(ref tmp) = script_tmp {
                eprintln!(
                    "Recording session to: {}",
                    record_path.as_ref().unwrap().display()
                );
                if let Some(parent) = tmp.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // Tell the Docker backend to wrap with `script`
                // SAFETY: single-threaded at this point (before spawning attach)
                unsafe { std::env::set_var("AGENTKERNEL_RECORD", tmp.to_string_lossy().as_ref()) };
            }

            // Attach to the sandbox's shell with environment variables
            let exit_code = manager.attach_with_env(&name, &env).await?;

            // Convert script typescript to asciicast format
            if let (Some(tmp), Some(cast_path)) = (&script_tmp, &record_path) {
                // SAFETY: single-threaded at this point (attach just returned)
                unsafe { std::env::remove_var("AGENTKERNEL_RECORD") };
                if tmp.exists() {
                    let raw = std::fs::read_to_string(tmp).unwrap_or_default();
                    let mut recorder = asciicast::AsciicastRecorder::with_header(
                        cast_path,
                        asciicast::AsciicastHeader::from_terminal()
                            .with_title(format!("agentkernel attach {}", name))
                            .with_command(format!("agentkernel attach {}", name)),
                    );
                    // Record the raw typescript output as a single asciicast event
                    recorder.record_output(&raw);
                    if let Err(e) = recorder.save() {
                        eprintln!("Warning: Failed to save recording: {}", e);
                    } else {
                        eprintln!("Session saved to: {}", cast_path.display());
                        eprintln!("  Replay with: agentkernel replay {}", cast_path.display());
                    }
                    // Clean up temp file
                    let _ = std::fs::remove_file(tmp);
                }
            }

            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        Commands::Exec { name, env, command } => {
            validation::validate_sandbox_name(&name)?;

            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!("Sandbox '{}' not found", name);
            }

            let output = manager.exec_cmd_with_env(&name, &command, &env).await?;
            print!("{}", output);
        }
        Commands::Cp { source, dest } => {
            // Parse source and destination to determine direction
            // Format: sandbox:/path or ./local/path
            let (src_sandbox, src_path) = parse_cp_path(&source);
            let (dst_sandbox, dst_path) = parse_cp_path(&dest);

            match (src_sandbox, dst_sandbox) {
                (Some(sandbox), None) => {
                    // Copy from sandbox to local
                    validation::validate_sandbox_name(&sandbox)?;
                    let mut manager = VmManager::new()?;

                    if !manager.exists(&sandbox) {
                        bail!("Sandbox '{}' not found", sandbox);
                    }
                    if !manager.is_running(&sandbox) {
                        bail!("Sandbox '{}' is not running", sandbox);
                    }

                    let content = manager.read_file(&sandbox, &src_path).await?;
                    std::fs::write(&dst_path, content)?;
                    println!(
                        "Copied {} bytes from {}:{} to {}",
                        std::fs::metadata(&dst_path)?.len(),
                        sandbox,
                        src_path,
                        dst_path
                    );
                }
                (None, Some(sandbox)) => {
                    // Copy from local to sandbox
                    validation::validate_sandbox_name(&sandbox)?;
                    let mut manager = VmManager::new()?;

                    if !manager.exists(&sandbox) {
                        bail!("Sandbox '{}' not found", sandbox);
                    }
                    if !manager.is_running(&sandbox) {
                        bail!("Sandbox '{}' is not running", sandbox);
                    }

                    let content = std::fs::read(&src_path)?;
                    manager.write_file(&sandbox, &dst_path, &content).await?;
                    println!(
                        "Copied {} bytes from {} to {}:{}",
                        content.len(),
                        src_path,
                        sandbox,
                        dst_path
                    );
                }
                (Some(_), Some(_)) => {
                    bail!("Cannot copy between sandboxes. Copy to local first.");
                }
                (None, None) => {
                    bail!("At least one path must be a sandbox path (sandbox:/path)");
                }
            }
        }
        Commands::List { project } => {
            let manager = VmManager::new()?;
            let vms = manager.list();

            // Optionally filter by current git project prefix
            let project_prefix = if project {
                match git_utils::detect() {
                    Ok(ctx) => {
                        println!("Filtering by git project: {}\n", ctx.project);
                        Some(ctx.project)
                    }
                    Err(_) => {
                        bail!("--project requires a git repository");
                    }
                }
            } else {
                None
            };

            let filtered: Vec<_> = vms
                .into_iter()
                .filter(|(name, _, _)| {
                    if let Some(ref prefix) = project_prefix {
                        name.starts_with(&format!("{}-", prefix)) || name == prefix
                    } else {
                        true
                    }
                })
                .collect();

            if filtered.is_empty() {
                if project_prefix.is_some() {
                    println!("No sandboxes found for this project.");
                } else {
                    println!("No sandboxes found.");
                }
                println!("\nCreate one with: agentkernel create <name>");
            } else {
                println!("{:<30} {:<10} {:<10}", "NAME", "STATUS", "BACKEND");
                for (name, running, backend) in filtered {
                    let status = if running { "running" } else { "stopped" };
                    let backend_str = backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "unknown".to_string());
                    println!("{:<30} {:<10} {:<10}", name, status, backend_str);
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
            fast,
            backend,
            template: tmpl,
            ttl,
            branch,
        } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel run [OPTIONS] <command...>");
            }

            // Fast path: use container pool for ephemeral runs
            if fast {
                if keep {
                    bail!("Cannot use --fast with --keep (pooled containers are ephemeral)");
                }
                if image.is_some() || config.is_some() {
                    eprintln!(
                        "Warning: --image and --config are ignored with --fast (pool uses alpine:3.20)"
                    );
                }

                let output = VmManager::run_pooled(&command).await?;
                print!("{}", output);
                return Ok(());
            }

            // Daemon path: try daemon VM pool first (single round-trip)
            // Skip is_available() check - just try and fall back on error
            if !keep {
                let daemon_client = daemon::DaemonClient::new();

                // Determine runtime from image/config
                let runtime = if let Some(ref img) = image {
                    languages::docker_image_to_firecracker_runtime(img).to_string()
                } else if let Some(ref config_path) = config {
                    let cfg = Config::from_file(config_path)?;
                    languages::docker_image_to_firecracker_runtime(&cfg.docker_image()).to_string()
                } else {
                    "base".to_string()
                };

                // Try daemon (single round-trip: acquire + exec + release)
                if let Ok(result) = daemon_client.run_in_pool(&runtime, &command).await {
                    eprintln!("Using daemon ({})", runtime);
                    print!("{}", result.stdout);
                    if !result.stderr.is_empty() {
                        eprint!("{}", result.stderr);
                    }
                    if result.exit_code != 0 {
                        std::process::exit(result.exit_code);
                    }
                    return Ok(());
                }
                // Daemon not available or failed, fall through to ephemeral mode
            }

            // Determine Docker image: --image > --config > --template > Dockerfile > command > ./agentkernel.toml > project files > default
            // For `run`, command detection has higher priority than project files
            // because user is explicitly specifying what to run
            let (docker_image, cfg_for_build) = if let Some(img) = image {
                (img, None)
            } else if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                (cfg.docker_image(), Some(cfg))
            } else if let Some(ref tmpl_name) = tmpl {
                let resolved = template::resolve(tmpl_name)?;
                eprintln!("Using template '{}' ({})", resolved.name, resolved.source);
                let cfg = resolved.parse()?;
                (cfg.docker_image(), Some(cfg))
            } else if let Some(img) = languages::detect_from_command(&command) {
                // Command-based detection first for `run`
                (img, None)
            } else {
                // Try current directory config
                let default_config = PathBuf::from("agentkernel.toml");
                if default_config.exists() {
                    let cfg = Config::from_file(&default_config)?;
                    (cfg.docker_image(), Some(cfg))
                } else {
                    // Fall back to project file detection or default
                    (languages::detect_image(&command), None)
                }
            };

            // Check for Dockerfile and build if present
            let current_dir = std::env::current_dir()?;
            let is_firecracker_backend = backend
                .as_ref()
                .is_some_and(|b| b == "firecracker" || b == "fc");

            // Build from Dockerfile if configured or auto-detected
            let docker_image = if let Some(ref cfg) = cfg_for_build {
                // Use config's build settings
                if cfg.requires_build(&current_dir) {
                    let project_name = &cfg.sandbox.name;
                    build::build_or_use_image(project_name, &docker_image, &current_dir, cfg)?
                } else {
                    docker_image
                }
            } else {
                // Auto-detect Dockerfile in current directory
                if languages::detect_dockerfile(&current_dir).is_some() {
                    let project_name = current_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "project".to_string());
                    let default_cfg = Config::minimal(&project_name, "claude");
                    build::build_or_use_image(
                        &project_name,
                        &docker_image,
                        &current_dir,
                        &default_cfg,
                    )?
                } else {
                    docker_image
                }
            };

            // For Firecracker backend with custom images, convert to rootfs
            let docker_image = if is_firecracker_backend && docker_image.starts_with("agentkernel-")
            {
                // This is a custom-built image, convert to ext4 rootfs
                let rootfs_dir = current_dir.join("images/rootfs");
                let result = rootfs::convert_image_to_rootfs(&docker_image, &rootfs_dir, None)?;
                // Return a special marker that the Firecracker backend will recognize
                format!("rootfs:{}", result.rootfs_path.display())
            } else {
                docker_image
            };

            // Get permissions from profile
            let mut perms = permissions::SecurityProfile::from_str(&profile)
                .unwrap_or_default()
                .permissions();

            // Apply --no-network override
            if no_network {
                perms.network = false;
            }

            // Apply config overrides if present and load files
            let files = if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                for warning in cfg.validate() {
                    eprintln!("Warning: {}", warning);
                }
                let cfg_perms = cfg.get_permissions();
                // Config overrides take precedence over CLI profile
                if cfg.security.network.is_some() {
                    perms.network = cfg_perms.network;
                }
                // Load files relative to config file directory
                let config_dir = config_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                cfg.load_files(config_dir)?
            } else {
                // Check for default config file and load files if present
                let default_config = PathBuf::from("agentkernel.toml");
                if default_config.exists() {
                    let cfg = Config::from_file(&default_config)?;
                    cfg.load_files(std::path::Path::new("."))?
                } else {
                    Vec::new()
                }
            };

            // Parse backend option if provided
            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };
            let mut manager = VmManager::with_backend(backend_type)?;

            // Optimized path: use run_ephemeral for single-operation execution
            // This is faster than create→start→exec→stop→remove cycle:
            // - Docker: single `docker run --rm` command
            // - Apple containers: single `container run --rm` (~940ms vs ~2200ms)
            // Only used when --keep is not specified
            if !keep {
                match manager
                    .run_ephemeral_with_files(&docker_image, &command, &perms, &files)
                    .await
                {
                    Ok(output) => {
                        print!("{}", output);
                        return Ok(());
                    }
                    Err(e) => {
                        // Firecracker doesn't support ephemeral mode, fall through to multi-step
                        if !e.to_string().contains("Ephemeral mode not supported") {
                            // Real error, bail out
                            bail!("{}", e);
                        }
                        // Fall through to multi-step cycle for Firecracker
                    }
                }
            }

            // Fallback: multi-step VM mode (for --keep or Firecracker backend)
            // Generate sandbox name: --branch derives from git, otherwise random
            let sandbox_name = if branch {
                let ctx = git_utils::detect()
                    .map_err(|_| anyhow::anyhow!("--branch requires a git repository"))?;
                let name = ctx.sandbox_name();
                // Reuse existing sandbox if it already exists for this branch
                if manager.exists(&name) {
                    eprintln!("Reusing existing sandbox for branch: {}", name);
                    // Just exec in the existing sandbox
                    if !manager.is_running(&name) {
                        manager.start(&name).await?;
                    }
                    let output = manager.exec_cmd(&name, &command).await?;
                    print!("{}", output);
                    return Ok(());
                }
                eprintln!("Using git-derived sandbox name: {}", name);
                name
            } else {
                let run_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
                format!("run-{}", run_id)
            };

            // Create
            // For `run`, default TTL is 1h when --keep is used
            let ttl_secs = if let Some(ref t) = ttl {
                Some(parse_ttl(t)?)
            } else if keep {
                Some(3600) // 1h default for kept run sandboxes
            } else {
                None // ephemeral, will be removed after exec
            };
            manager
                .create_with_ttl(&sandbox_name, &docker_image, 1, 512, ttl_secs)
                .await?;

            // Start with permissions and inject files
            if let Err(e) = manager
                .start_with_permissions_and_files(&sandbox_name, &perms, &files)
                .await
            {
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
        Commands::Plugin { action } => match action {
            PluginAction::Install {
                target,
                global,
                force,
                dry_run,
            } => {
                let opts = plugin_installer::InstallOptions {
                    global,
                    force,
                    dry_run,
                };

                if target == "all" {
                    for t in plugin_installer::PluginTarget::all() {
                        plugin_installer::install_plugin(*t, &opts)?;
                        println!();
                    }
                } else {
                    let t = plugin_installer::PluginTarget::from_str(&target).ok_or_else(
                        || {
                            anyhow::anyhow!(
                            "Unknown plugin target: '{}'. Valid targets: claude, codex, gemini, opencode, mcp, all",
                            target
                        )
                        },
                    )?;
                    plugin_installer::install_plugin(t, &opts)?;
                }
            }
            PluginAction::List => {
                plugin_installer::list_plugins();
            }
        },
        Commands::Daemon { action } => {
            match action {
                DaemonAction::Start { background } => {
                    // Check setup status first
                    let status = check_installation();
                    if !status.kvm_available {
                        bail!("Daemon mode requires KVM. Run 'agentkernel status' to check.");
                    }
                    if !status.kernel_installed || !status.rootfs_base_installed {
                        bail!(
                            "Agentkernel is not fully set up. Run 'agentkernel setup' first.\n\
                             Missing: {}",
                            missing_components(&status)
                        );
                    }

                    let socket_path = daemon::DaemonServer::default_socket_path();
                    if daemon::DaemonServer::is_running(&socket_path) {
                        bail!("Daemon is already running at {}", socket_path.display());
                    }

                    // Find kernel and rootfs paths
                    let base_dir = find_images_dir()?;
                    let kernel_path = find_kernel(&base_dir)?;
                    let rootfs_dir = base_dir.join("rootfs");

                    let config = daemon::PoolConfig::default();
                    let server = daemon::DaemonServer::new(config, kernel_path, rootfs_dir);

                    if background {
                        // TODO: Fork and daemonize
                        bail!("Background mode not yet implemented. Run in foreground for now.");
                    }

                    println!("Starting daemon...");
                    server.run().await?;
                }
                DaemonAction::Stop => {
                    let client = daemon::DaemonClient::new();
                    if !client.is_available() {
                        bail!("Daemon is not running");
                    }

                    println!("Stopping daemon...");
                    client.shutdown().await?;
                    println!("Daemon stopped.");
                }
                DaemonAction::Status => {
                    let client = daemon::DaemonClient::new();
                    if !client.is_available() {
                        println!("Daemon: not running");
                        println!("Socket: {}", client.socket_path().display());
                        return Ok(());
                    }

                    let (warm, in_use, min_warm, max_warm) = client.status().await?;
                    println!("Daemon: running");
                    println!("Socket: {}", client.socket_path().display());
                    println!("Pool:");
                    println!("  Warm VMs:    {}", warm);
                    println!("  In use:      {}", in_use);
                    println!("  Min/Max:     {}/{}", min_warm, max_warm);
                }
            }
        }
        Commands::Audit {
            sandbox,
            last,
            path,
            json,
        } => {
            let audit_log = audit::AuditLog::new();

            if path {
                println!("{}", audit_log.path().display());
                return Ok(());
            }

            let entries = if let Some(ref name) = sandbox {
                audit_log.read_by_sandbox(name)?
            } else {
                audit_log.read_last(last)?
            };

            if entries.is_empty() {
                if let Some(ref name) = sandbox {
                    println!("No audit entries for sandbox '{}'", name);
                } else {
                    println!("No audit entries found");
                }
                println!("Audit log: {}", audit_log.path().display());
                return Ok(());
            }

            if json {
                for entry in &entries {
                    println!("{}", serde_json::to_string(entry)?);
                }
            } else {
                println!(
                    "{:<24} {:<20} {:<15} DETAILS",
                    "TIMESTAMP", "EVENT", "SANDBOX"
                );
                println!("{}", "-".repeat(80));
                for entry in &entries {
                    let (event_type, sandbox_name, details) = match &entry.event {
                        audit::AuditEvent::SandboxCreated { name, image, .. } => {
                            ("sandbox_created", name.as_str(), format!("image={}", image))
                        }
                        audit::AuditEvent::SandboxStarted { name, profile } => (
                            "sandbox_started",
                            name.as_str(),
                            profile
                                .as_ref()
                                .map(|p| format!("profile={}", p))
                                .unwrap_or_default(),
                        ),
                        audit::AuditEvent::SandboxStopped { name } => {
                            ("sandbox_stopped", name.as_str(), String::new())
                        }
                        audit::AuditEvent::SandboxRemoved { name } => {
                            ("sandbox_removed", name.as_str(), String::new())
                        }
                        audit::AuditEvent::CommandExecuted {
                            sandbox,
                            command,
                            exit_code,
                        } => (
                            "command_executed",
                            sandbox.as_str(),
                            format!(
                                "cmd={} exit={}",
                                command.join(" "),
                                exit_code.map(|c| c.to_string()).unwrap_or("?".to_string())
                            ),
                        ),
                        audit::AuditEvent::FileWritten { sandbox, path } => {
                            ("file_written", sandbox.as_str(), format!("path={}", path))
                        }
                        audit::AuditEvent::FileRead { sandbox, path } => {
                            ("file_read", sandbox.as_str(), format!("path={}", path))
                        }
                        audit::AuditEvent::SessionAttached { sandbox } => {
                            ("session_attached", sandbox.as_str(), String::new())
                        }
                        audit::AuditEvent::PolicyViolation {
                            sandbox,
                            policy,
                            details,
                        } => (
                            "policy_violation",
                            sandbox.as_str(),
                            format!("{}: {}", policy, details),
                        ),
                    };
                    println!(
                        "{:<24} {:<20} {:<15} {}",
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                        event_type,
                        sandbox_name,
                        details
                    );
                }
            }
        }
        Commands::Replay {
            file,
            speed,
            max_idle,
        } => {
            if !file.exists() {
                bail!("Recording file not found: {}", file.display());
            }

            let (header, events) = asciicast::read_asciicast(&file)?;

            println!("Playing: {}", file.display());
            println!(
                "Terminal: {}x{}, Duration: {:.1}s, Speed: {}x",
                header.width,
                header.height,
                header.duration.unwrap_or(0.0),
                speed
            );
            println!("{}", "-".repeat(40));

            // Play back the events
            let mut last_time = 0.0;
            for event in &events {
                // Calculate delay (accounting for speed and max_idle)
                let delay = ((event.time - last_time) / speed).min(max_idle);
                if delay > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(delay));
                }
                last_time = event.time;

                // Print output events
                if event.event_type == asciicast::EventType::Output {
                    print!("{}", event.data);
                    std::io::Write::flush(&mut std::io::stdout())?;
                }
            }

            println!();
            println!("{}", "-".repeat(40));
            println!("Playback complete.");
        }
        Commands::Secret { action } => {
            let vault = secrets::SecretVault::new(secrets::SecretBackend::default());
            match action {
                SecretAction::Set { key, value } => {
                    let val = if let Some(v) = value {
                        v
                    } else {
                        eprint!("Enter value for {}: ", key);
                        let mut buf = String::new();
                        std::io::stdin().read_line(&mut buf)?;
                        buf.trim().to_string()
                    };
                    vault.set(&key, &val)?;
                    println!("Stored '{}' (backend: {})", key, vault.backend());
                }
                SecretAction::Get { key } => match vault.get(&key)? {
                    Some(val) => println!("{}", val),
                    None => {
                        bail!("Secret '{}' not found", key);
                    }
                },
                SecretAction::List => {
                    let entries = vault.list()?;
                    if entries.is_empty() {
                        println!("No secrets stored.");
                    } else {
                        println!("{:<30} {:<10} SET AT", "KEY", "BACKEND");
                        for (key, entry) in &entries {
                            println!("{:<30} {:<10} {}", key, entry.backend, entry.set_at);
                        }
                    }
                }
                SecretAction::Delete { key } => {
                    vault.delete(&key)?;
                    println!("Deleted '{}'", key);
                }
            }
        }
        Commands::Gc { dry_run } => {
            let mut manager = VmManager::new()?;
            let expired = manager.expired();
            if expired.is_empty() {
                println!("No expired sandboxes.");
            } else if dry_run {
                println!("Would remove {} expired sandbox(es):", expired.len());
                for name in &expired {
                    println!("  {}", name);
                }
            } else {
                let removed = manager.gc().await?;
                println!("Removed {} expired sandbox(es):", removed.len());
                for name in &removed {
                    println!("  {}", name);
                }
            }
        }
        Commands::Info { name } => {
            validation::validate_sandbox_name(&name)?;
            run_info(&name)?;
        }
        Commands::Benchmark {
            backends,
            iterations,
            image,
        } => {
            let backend_list = if let Some(ref b) = backends {
                benchmark::parse_backends(b)?
            } else {
                benchmark::available_backends()
            };
            if backend_list.is_empty() {
                bail!("No backends available to benchmark.");
            }
            benchmark::run_benchmark(&backend_list, iterations, &image).await?;
        }
        Commands::Pipeline { file, backend } => {
            if !file.exists() {
                bail!("Pipeline file not found: {}", file.display());
            }

            let pipe = pipeline::load(&file)?;
            println!(
                "Running pipeline ({} step{}) from {}\n",
                pipe.step.len(),
                if pipe.step.len() != 1 { "s" } else { "" },
                file.display()
            );

            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };
            let mut manager = VmManager::with_backend(backend_type)?;

            let prefix = format!("pipe-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            pipeline::run(&pipe, &mut manager, &prefix).await?;
        }
        Commands::Session { action } => match action {
            SessionAction::Start {
                name,
                agent,
                image,
                backend,
            } => {
                let sess = session::create(&name, &agent)?;
                let docker_image = image.unwrap_or_else(|| "alpine:3.20".to_string());

                let backend_type = if let Some(ref b) = backend {
                    Some(
                        b.parse::<crate::backend::BackendType>()
                            .map_err(|e| anyhow::anyhow!(e))?,
                    )
                } else {
                    None
                };
                let mut manager = VmManager::with_backend(backend_type)?;

                println!("Starting session '{}' (agent: {})...", name, agent);
                manager.create(&sess.sandbox, &docker_image, 1, 512).await?;
                manager.start(&sess.sandbox).await?;
                println!("Session '{}' started.", name);
                println!("  Sandbox: {}", sess.sandbox);
                println!("\nAttach with: agentkernel attach {}", sess.sandbox);
            }
            SessionAction::List => {
                let sessions = session::list()?;
                if sessions.is_empty() {
                    println!("No sessions found.");
                } else {
                    let now = chrono::Utc::now().to_rfc3339();
                    println!(
                        "{:<20} {:<10} {:<10} {:<12} {:>6}",
                        "NAME", "AGENT", "STATUS", "DURATION", "EXECS"
                    );
                    for s in &sessions {
                        let dur = session::format_duration(&s.created_at, &now);
                        println!(
                            "{:<20} {:<10} {:<10} {:<12} {:>6}",
                            s.name, s.agent, s.status, dur, s.exec_count
                        );
                    }
                }
            }
            SessionAction::Stop { name } => {
                let sess = session::get(&name)?
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;

                let mut manager = VmManager::new()?;
                if manager.exists(&sess.sandbox) && manager.is_running(&sess.sandbox) {
                    manager.stop(&sess.sandbox).await?;
                }
                session::stop(&name)?;
                println!("Session '{}' stopped.", name);
            }
            SessionAction::Save { name } => {
                let sess = session::get(&name)?
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;

                let manager = VmManager::new()?;
                let state = manager
                    .get_state(&sess.sandbox)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", sess.sandbox))?;

                let snap_name = format!("{}-{}", name, chrono::Utc::now().format("%Y%m%d"));
                let input = snapshot::SnapshotInput {
                    image: state.image.clone(),
                    backend: state
                        .backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "docker".to_string()),
                    vcpus: state.vcpus,
                    memory_mb: state.memory_mb,
                };

                println!("Saving session '{}'...", name);
                snapshot::take(&sess.sandbox, &snap_name, &input)?;
                session::mark_saved(&name, &snap_name)?;
                println!("Session '{}' saved (snapshot: {})", name, snap_name);
            }
            SessionAction::Resume { name } => {
                let sess = session::get(&name)?
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;

                let mut manager = VmManager::new()?;

                if sess.status == session::SessionStatus::Saved {
                    // Restore from snapshot
                    if let Some(ref snap_name) = sess.snapshot {
                        let meta = snapshot::get(snap_name)?
                            .ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", snap_name))?;
                        if !manager.exists(&sess.sandbox) {
                            println!("Restoring from snapshot '{}'...", snap_name);
                            manager
                                .create(&sess.sandbox, &meta.image_tag, meta.vcpus, meta.memory_mb)
                                .await?;
                        }
                    }
                }

                if manager.exists(&sess.sandbox) && !manager.is_running(&sess.sandbox) {
                    manager.start(&sess.sandbox).await?;
                }
                session::mark_running(&name)?;
                println!("Session '{}' resumed.", name);
                println!("\nAttach with: agentkernel attach {}", sess.sandbox);
            }
            SessionAction::Delete { name } => {
                let sess = session::get(&name)?
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;

                let mut manager = VmManager::new()?;
                if manager.exists(&sess.sandbox) {
                    if manager.is_running(&sess.sandbox) {
                        let _ = manager.stop(&sess.sandbox).await;
                    }
                    let _ = manager.remove(&sess.sandbox).await;
                }
                session::delete(&name)?;
                println!("Session '{}' deleted.", name);
            }
        },
        Commands::Snapshot { action } => match action {
            SnapshotAction::Take { sandbox, name } => {
                validation::validate_sandbox_name(&sandbox)?;
                let manager = VmManager::new()?;
                let state = manager
                    .get_state(&sandbox)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", sandbox))?;

                let snap_name = name.unwrap_or_else(|| {
                    let date = chrono::Utc::now().format("%Y%m%d");
                    format!("{}-{}", sandbox, date)
                });

                let input = snapshot::SnapshotInput {
                    image: state.image.clone(),
                    backend: state
                        .backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "docker".to_string()),
                    vcpus: state.vcpus,
                    memory_mb: state.memory_mb,
                };

                println!("Snapshotting '{}' → '{}'...", sandbox, snap_name);
                let meta = snapshot::take(&sandbox, &snap_name, &input)?;
                println!(
                    "Snapshot '{}' created (image: {})",
                    meta.name, meta.image_tag
                );
            }
            SnapshotAction::List => {
                let snaps = snapshot::list()?;
                if snaps.is_empty() {
                    println!("No snapshots found.");
                } else {
                    println!(
                        "{:<25} {:<20} {:<12} {:<24}",
                        "NAME", "SANDBOX", "BACKEND", "CREATED"
                    );
                    for s in &snaps {
                        println!(
                            "{:<25} {:<20} {:<12} {:<24}",
                            s.name, s.sandbox, s.backend, s.created_at
                        );
                    }
                }
            }
            SnapshotAction::Delete { name } => {
                snapshot::delete(&name)?;
                println!("Snapshot '{}' deleted.", name);
            }
        },
        Commands::Restore {
            name,
            r#as: as_name,
            backend,
        } => {
            let meta = snapshot::get(&name)?
                .ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", name))?;

            let restore_name = as_name.unwrap_or_else(|| format!("{}-restored", meta.sandbox));
            validation::validate_sandbox_name(&restore_name)?;

            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };
            let mut manager = VmManager::with_backend(backend_type)?;

            println!(
                "Restoring snapshot '{}' as sandbox '{}'...",
                name, restore_name
            );
            manager
                .create(&restore_name, &meta.image_tag, meta.vcpus, meta.memory_mb)
                .await?;

            println!("Sandbox '{}' restored from snapshot.", restore_name);
            println!("\nNext steps:");
            println!("  agentkernel start {}", restore_name);
            println!("  agentkernel attach {}", restore_name);
        }
        Commands::Clean { force, all } => {
            run_clean(force, all).await?;
        }
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "agentkernel",
                &mut std::io::stdout(),
            );
        }
        #[cfg(feature = "enterprise")]
        Commands::Policy { action } => {
            handle_policy_command(action).await?;
        }
    }

    Ok(())
}

async fn run_doctor() -> Result<()> {
    let status = check_installation();

    // -- Backend Health --
    println!("Backend Health:");

    // Docker
    let docker_info = std::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if let Some(ver) = docker_info {
        // Check if daemon is actually running
        let daemon_ok = std::process::Command::new("docker")
            .args(["info"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let daemon_str = if daemon_ok {
            "(daemon running)"
        } else {
            "(daemon not running)"
        };
        println!("  Docker .............. v{} {}", ver, daemon_str);
    } else {
        println!("  Docker .............. not installed");
    }

    // Podman
    let podman_info = std::process::Command::new("podman")
        .args(["version", "--format", "{{.Version}}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if let Some(ver) = podman_info {
        println!("  Podman .............. v{}", ver);
    } else {
        println!("  Podman .............. not installed");
    }

    // Firecracker
    if status.firecracker_installed {
        if status.kvm_available {
            println!("  Firecracker ......... installed (KVM available)");
        } else {
            println!("  Firecracker ......... installed (no KVM)");
        }
    } else {
        println!("  Firecracker ......... not installed");
    }

    // Apple Containers
    if cfg!(target_os = "macos") {
        if status.apple_containers_available {
            let ver = std::process::Command::new("sw_vers")
                .args(["-productVersion"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("  Apple Containers .... macOS {}", ver);
        } else {
            println!("  Apple Containers .... not available");
        }
    }

    // -- Daemon --
    println!("\nDaemon:");
    let daemon_client = daemon::DaemonClient::new();
    if daemon_client.is_available() {
        println!(
            "  Status .............. running (socket: {})",
            daemon_client.socket_path().display()
        );
        if let Ok((warm, in_use, _min, _max)) = daemon_client.status().await {
            println!("  Warm VMs ............ {}", warm);
            println!("  In use .............. {}", in_use);
        }
    } else {
        println!("  Status .............. not running");
    }

    // -- Policy Engine --
    #[cfg(feature = "enterprise")]
    {
        println!("\nPolicy Engine:");
        let config_path = std::path::PathBuf::from("agentkernel.toml");
        let cfg = if config_path.exists() {
            Config::from_file(&config_path).ok()
        } else {
            None
        };
        if let Some(ref cfg) = cfg {
            if cfg.enterprise.enabled {
                match policy::PolicyEngine::new(&cfg.enterprise) {
                    Ok(engine) => {
                        let ver = engine.version().await;
                        println!("  Cedar ............... enabled (version: {})", ver);
                    }
                    Err(e) => println!("  Cedar ............... error ({})", e),
                }
            } else {
                println!("  Cedar ............... disabled");
            }
        } else {
            println!("  Cedar ............... disabled (no config)");
        }

        // Policy audit log
        let policy_logger = policy::PolicyAuditLogger::default_path();
        let policy_path = policy_logger.path();
        if policy_path.exists()
            && let Ok(entries) = policy_logger.read_last(usize::MAX)
        {
            println!("  Audit log ........... {} decisions logged", entries.len());
        }
    }

    // -- Sandboxes --
    println!("\nSandboxes:");
    if let Ok(manager) = VmManager::new() {
        let vms = manager.list();
        let running = vms.iter().filter(|(_, r, _)| *r).count();
        let stopped = vms.len() - running;
        println!("  Running ............. {}", running);
        println!("  Stopped ............. {}", stopped);
    } else {
        println!("  (unable to query)");
    }

    // -- Disk --
    println!("\nDisk:");
    let data_dir = setup::default_data_dir();
    if data_dir.exists() {
        let images_dir = data_dir.join("images");
        if images_dir.exists() {
            let size = dir_size(&images_dir);
            println!(
                "  Images .............. {} ({})",
                human_bytes(size),
                images_dir.display()
            );
        }
    }

    let audit_log = audit::AuditLog::new();
    let audit_path = audit_log.path();
    if audit_path.exists() {
        let size = std::fs::metadata(audit_path).map(|m| m.len()).unwrap_or(0);
        let count = audit_log.read_all().map(|e| e.len()).unwrap_or(0);
        println!(
            "  Audit log ........... {} ({} entries)",
            human_bytes(size),
            count
        );
    }

    // -- Config --
    println!("\nConfig:");
    let config_path = std::path::PathBuf::from("agentkernel.toml");
    if config_path.exists() {
        match Config::from_file(&config_path) {
            Ok(cfg) => {
                let warnings = cfg.validate();
                if warnings.is_empty() {
                    println!("  agentkernel.toml .... valid");
                } else {
                    println!("  agentkernel.toml .... {} warning(s)", warnings.len());
                    for w in &warnings {
                        println!("    - {}", w);
                    }
                }
            }
            Err(e) => println!("  agentkernel.toml .... error: {}", e),
        }
    } else {
        println!("  agentkernel.toml .... not found (using defaults)");
    }

    Ok(())
}

/// Calculate total size of a directory recursively
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = entry.metadata();
            if let Ok(meta) = meta {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

/// Format bytes as human-readable string
fn human_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(feature = "enterprise")]
async fn handle_policy_command(action: PolicyAction) -> Result<()> {
    match action {
        PolicyAction::Check { action, sandbox } => {
            // Parse the action string
            let cedar_action = match action.to_lowercase().as_str() {
                "run" => policy::Action::Run,
                "exec" => policy::Action::Exec,
                "create" => policy::Action::Create,
                "attach" => policy::Action::Attach,
                "mount" => policy::Action::Mount,
                "network" => policy::Action::Network,
                other => bail!(
                    "Invalid action '{}'. Use: run, exec, create, attach, mount, network",
                    other
                ),
            };

            // Load config and initialize policy engine
            let config_path = std::path::PathBuf::from("agentkernel.toml");
            let cfg = if config_path.exists() {
                Config::from_file(&config_path)?
            } else {
                Config::minimal("default", "claude")
            };

            if !cfg.enterprise.enabled {
                println!("Enterprise policy engine is not enabled.");
                println!("Set [enterprise] enabled = true in agentkernel.toml");
                return Ok(());
            }

            let engine = policy::PolicyEngine::new(&cfg.enterprise)?;

            // Build principal from local user
            let principal = policy::Principal {
                id: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
                email: String::new(),
                org_id: cfg
                    .enterprise
                    .org_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                roles: cfg.enterprise.default_roles.clone(),
                mfa_verified: false,
            };

            let resource = policy::Resource {
                name: sandbox.clone(),
                agent_type: "cli".to_string(),
                runtime: "unknown".to_string(),
            };

            let decision = engine.evaluate(&principal, cedar_action, &resource).await;

            println!("Policy Check");
            println!("{}", "-".repeat(40));
            println!("Principal:  {} ({})", principal.id, principal.org_id);
            println!("Roles:      {:?}", principal.roles);
            println!("Action:     {}", action);
            println!("Resource:   {}", sandbox);
            println!("{}", "-".repeat(40));
            if decision.is_permit() {
                println!("Decision:   PERMIT");
            } else {
                println!("Decision:   DENY");
            }
            println!("Reason:     {}", decision.reason);
            if !decision.matched_policies.is_empty() {
                println!("Policies:   {}", decision.matched_policies.join(", "));
            }
            println!("Eval time:  {}us", decision.evaluation_time_us);
        }
        PolicyAction::Status => {
            let config_path = std::path::PathBuf::from("agentkernel.toml");
            let cfg = if config_path.exists() {
                Config::from_file(&config_path)?
            } else {
                Config::minimal("default", "claude")
            };

            println!("Enterprise Policy Status");
            println!("{}", "-".repeat(40));
            println!("Enabled:        {}", cfg.enterprise.enabled);
            println!(
                "Org ID:         {}",
                cfg.enterprise.org_id.as_deref().unwrap_or("(not set)")
            );
            println!("Offline mode:   {}", cfg.enterprise.offline_mode);
            println!(
                "Cache max age:  {} hours",
                cfg.enterprise.cache_max_age_hours
            );
            println!(
                "Policy server:  {}",
                cfg.enterprise
                    .policy_server
                    .as_deref()
                    .unwrap_or("(not configured)")
            );
            println!(
                "Trust anchors:  {} key(s)",
                cfg.enterprise.trust_anchors.keys.len()
            );
            println!("Default roles:  {:?}", cfg.enterprise.default_roles);

            if cfg.enterprise.enabled {
                match policy::PolicyEngine::new(&cfg.enterprise) {
                    Ok(engine) => {
                        let version = engine.version().await;
                        println!("Policy ver:     {}", version);
                        println!("Engine:         active");
                    }
                    Err(e) => {
                        println!("Engine:         error ({})", e);
                    }
                }
            }
        }
        PolicyAction::AuditLog { last, json } => {
            let logger = policy::PolicyAuditLogger::default_path();
            let entries = logger.read_last(last)?;

            if entries.is_empty() {
                println!("No policy audit entries found.");
                println!("Log path: {}", logger.path().display());
                return Ok(());
            }

            if json {
                for entry in &entries {
                    println!("{}", serde_json::to_string(entry)?);
                }
            } else {
                println!(
                    "{:<24} {:<10} {:<15} {:<15} RESOURCE",
                    "TIMESTAMP", "DECISION", "PRINCIPAL", "ACTION"
                );
                println!("{}", "-".repeat(80));
                for entry in &entries {
                    println!(
                        "{:<24} {:<10} {:<15} {:<15} {}",
                        entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                        format!("{:?}", entry.decision),
                        entry.principal,
                        entry.action,
                        entry.resource
                    );
                }
            }
        }
    }
    Ok(())
}

/// Find the images directory
fn find_images_dir() -> Result<PathBuf> {
    // Check installed location first (preferred)
    if let Some(home) = std::env::var_os("HOME") {
        let home_path = PathBuf::from(home).join(".local/share/agentkernel/images");
        // Check if it has actual content (kernel or rootfs)
        if home_path.join("kernel").exists() || home_path.join("rootfs").exists() {
            return Ok(home_path);
        }
    }

    // Check relative to current dir (development mode)
    let paths = [PathBuf::from("images"), PathBuf::from("../images")];

    for path in &paths {
        if path.join("kernel").exists() || path.join("rootfs").exists() {
            return Ok(path.clone());
        }
    }

    bail!(
        "Images directory not found. Run 'agentkernel setup' first, or check ~/.local/share/agentkernel/images"
    );
}

/// Find the kernel image
fn find_kernel(base_dir: &Path) -> Result<PathBuf> {
    let kernel_dir = base_dir.join("kernel");

    // Look for vmlinux-*-agentkernel
    if kernel_dir.exists() {
        for entry in std::fs::read_dir(&kernel_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("vmlinux-") && name_str.ends_with("-agentkernel") {
                return Ok(entry.path());
            }
        }
    }

    bail!(
        "Kernel not found in {}. Run 'agentkernel setup' first.",
        kernel_dir.display()
    );
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

/// Parse a cp-style path (sandbox:/path or ./local/path)
/// Returns (Some(sandbox_name), path) for sandbox paths
/// Returns (None, path) for local paths
fn parse_cp_path(path: &str) -> (Option<String>, String) {
    // Check for sandbox:path format (must have : but not be a Windows path like C:\)
    if let Some(colon_pos) = path.find(':') {
        // Make sure it's not a local path starting with / or .
        let before_colon = &path[..colon_pos];
        if !before_colon.is_empty()
            && !before_colon.starts_with('/')
            && !before_colon.starts_with('.')
        {
            let sandbox_name = before_colon.to_string();
            let remote_path = path[colon_pos + 1..].to_string();
            return (Some(sandbox_name), remote_path);
        }
    }
    // Local path
    (None, path.to_string())
}

fn run_info(name: &str) -> Result<()> {
    let manager = VmManager::new()?;
    let state = manager
        .get_state(name)
        .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

    let running = manager.is_running(name);
    let status_str = if running { "running" } else { "stopped" };
    let backend_str = state
        .backend
        .map(|b| format!("{}", b))
        .unwrap_or_else(|| "unknown".to_string());

    println!("Name:           {}", state.name);
    println!("Status:         {}", status_str);
    println!("Backend:        {}", backend_str);
    println!("Image:          {}", state.image);
    println!(
        "Resources:      {} vCPU{}, {}MB RAM",
        state.vcpus,
        if state.vcpus != 1 { "s" } else { "" },
        state.memory_mb
    );
    println!("Created:        {}", state.created_at);
    if let Some(ttl) = state.ttl_seconds {
        println!("TTL:            {}", format_ttl(ttl));
    }
    if let Some(ref exp) = state.expires_at {
        println!("Expires:        {}", exp);
    }
    if let Some(ref rid) = state.remote_id {
        println!("Remote ID:      {}", rid);
    }
    if let Some(ref rns) = state.remote_namespace {
        println!("Namespace:      {}", rns);
    }

    // Show recent audit activity
    let audit = audit::AuditLog::new();
    let entries = audit.read_by_sandbox(name)?;
    if !entries.is_empty() {
        let last = entries.last().unwrap();
        println!(
            "Last active:    {}",
            last.timestamp.format("%Y-%m-%d %H:%M:%S")
        );

        let recent: Vec<_> = entries.iter().rev().take(5).collect();
        println!("\nRecent activity (last {}):", recent.len());
        for entry in recent.iter().rev() {
            let ts = entry.timestamp.format("%H:%M:%S");
            let desc = match &entry.event {
                audit::AuditEvent::SandboxCreated { image, .. } => {
                    format!("create  image={}", image)
                }
                audit::AuditEvent::SandboxStarted { profile, .. } => {
                    let p = profile.as_deref().unwrap_or("default");
                    format!("start   profile={}", p)
                }
                audit::AuditEvent::SandboxStopped { .. } => "stop".to_string(),
                audit::AuditEvent::SandboxRemoved { .. } => "remove".to_string(),
                audit::AuditEvent::CommandExecuted {
                    command, exit_code, ..
                } => {
                    let cmd = command.join(" ");
                    let code = exit_code
                        .map(|c| format!(" exit={}", c))
                        .unwrap_or_default();
                    format!("exec    {}{}", cmd, code)
                }
                audit::AuditEvent::FileWritten { path, .. } => {
                    format!("write   {}", path)
                }
                audit::AuditEvent::FileRead { path, .. } => {
                    format!("read    {}", path)
                }
                audit::AuditEvent::SessionAttached { .. } => "attach".to_string(),
                audit::AuditEvent::PolicyViolation { policy, .. } => {
                    format!("policy  denied: {}", policy)
                }
            };
            println!("  {}  {}", ts, desc);
        }
    }

    Ok(())
}

async fn run_clean(force: bool, all: bool) -> Result<()> {
    let mut manager = VmManager::new()?;
    let sandboxes = manager
        .list()
        .iter()
        .map(|(n, r, _)| (n.to_string(), *r))
        .collect::<Vec<_>>();

    let mut removed = 0usize;
    let mut skipped = 0usize;

    for (name, running) in &sandboxes {
        if *running && !force {
            println!("  Skipping '{}' (running, use --force to remove)", name);
            skipped += 1;
            continue;
        }
        if *running {
            println!("  Stopping '{}'...", name);
        }
        manager.remove(name).await?;
        println!("  Removed sandbox '{}'", name);
        removed += 1;
    }
    println!("Sandboxes: {} removed, {} skipped", removed, skipped);

    // Clean up Docker containers matching agentkernel-*
    let containers_output = std::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            "name=agentkernel-",
            "--format",
            "{{.Names}}",
        ])
        .output();
    if let Ok(output) = containers_output {
        let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        if !names.is_empty() {
            let mut cmd = std::process::Command::new("docker");
            cmd.arg("rm").arg("-f");
            for n in &names {
                cmd.arg(n);
            }
            let _ = cmd.output();
            println!("Docker containers: {} removed", names.len());
        } else {
            println!("Docker containers: none found");
        }
    }

    if all {
        // Clean up Docker images matching agentkernel-*
        let images_output = std::process::Command::new("docker")
            .args([
                "images",
                "--filter",
                "reference=agentkernel-*",
                "--format",
                "{{.Repository}}:{{.Tag}}",
            ])
            .output();
        if let Ok(output) = images_output {
            let imgs: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
            if !imgs.is_empty() {
                let mut cmd = std::process::Command::new("docker");
                cmd.arg("rmi").arg("-f");
                for img in &imgs {
                    cmd.arg(img);
                }
                let _ = cmd.output();
                println!("Docker images: {} removed", imgs.len());
            } else {
                println!("Docker images: none found");
            }
        }

        // Prune dangling build cache
        let prune = std::process::Command::new("docker")
            .args(["builder", "prune", "-f", "--filter", "label=agentkernel"])
            .output();
        if let Ok(output) = prune
            && output.status.success()
        {
            let out = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = out.lines().find(|l| l.contains("reclaimed")) {
                println!("Build cache: {}", line.trim());
            } else {
                println!("Build cache: pruned");
            }
        }
    }

    println!("\nDone.");
    Ok(())
}

/// Parse a human-readable TTL string (e.g. "1h", "30m", "3d", "0") into seconds.
fn parse_ttl(s: &str) -> Result<u64> {
    let s = s.trim();
    if s == "0" {
        return Ok(0);
    }
    let (num, multiplier) = if let Some(n) = s.strip_suffix('d') {
        (n, 86400u64)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1)
    } else {
        // Assume seconds if no suffix
        (s, 1)
    };
    let value: u64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid TTL: '{}'. Use e.g. 1h, 30m, 3d", s))?;
    Ok(value * multiplier)
}

/// Format seconds as a human-readable duration.
fn format_ttl(secs: u64) -> String {
    if secs == 0 {
        return "no expiry".to_string();
    }
    if secs >= 86400 && secs.is_multiple_of(86400) {
        format!("{}d", secs / 86400)
    } else if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}
