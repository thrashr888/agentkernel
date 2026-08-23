#![recursion_limit = "256"]

mod agent_catalog;
mod agents;
mod apple_backend;
mod asciicast;
mod audit;
mod backend;
mod benchmark;
mod browser_scripts;
mod build;
mod config;
mod daemon;
mod docker_backend;
mod durable_storage;
mod events;
mod firecracker_client;
mod git_utils;
mod http_api;
mod hyperlight_backend;
mod image_builder;
mod images;
mod interactive_permissions;
mod languages;
mod llm_intercept;
mod mcp;
mod metrics;
mod object_runtime;
mod observe;
mod opencode;
mod orchestration_store;
mod permissions;
mod pipeline;
mod plugin_installer;
mod pool;
#[allow(dead_code)]
mod proxy;
#[allow(dead_code)]
mod proxy_hooks;
mod receipt;
mod rootfs;
mod runtime;
mod sandbox_pool;
mod seatbelt;
mod secrets;
mod secure_fs;
mod session;
mod setup;
mod snapshot;
#[allow(dead_code)]
mod ssh;
mod stats;
mod task_worker;
mod task_worker_vmm;
mod tasks;
mod template;
mod tls;
mod validation;
mod vmm;
mod volume;
mod vsock;
#[allow(dead_code)]
mod vsock_secrets;

// Enterprise modules (behind feature flag)
// identity has public API surface for CLI login, middleware, and Cedar helpers
// not all consumed from the HTTP API yet
#[cfg(feature = "enterprise")]
#[allow(dead_code)]
mod identity;
#[cfg(feature = "enterprise")]
pub mod policy;

use anyhow::{Context, Result, bail};
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
    // -- Quick actions (most common, stay at root) --
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
        /// Build the current project's Dockerfile before running
        #[arg(long, conflicts_with_all = ["image", "fast"])]
        build: bool,
        /// Security profile: permissive, moderate (default), restrictive
        #[arg(short, long, default_value = "moderate")]
        profile: String,
        /// Disable network access
        #[arg(long)]
        no_network: bool,
        /// Use container pool for faster execution (skips create/destroy overhead)
        #[arg(short = 'F', long)]
        fast: bool,
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad, daytona, runloop, e2b, modal, agentcomputer (default: auto-detect)
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
        /// Publish a port (host:container, container, or host:container/udp). Can be repeated.
        #[arg(short = 'P', long = "publish")]
        publish: Vec<String>,
        /// Enable SSH access to the sandbox
        #[arg(long)]
        ssh: bool,
        /// Bind a secret to a host via proxy (KEY:host, KEY=value:host, KEY:host:header). Can be repeated.
        #[arg(short = 'S', long = "secret")]
        secrets: Vec<String>,
        /// Inject a secret as a file inside the sandbox (KEY from vault). Can be repeated.
        #[arg(long = "secret-file")]
        secret_files: Vec<String>,
        /// Use placeholder tokens instead of real secret values in file injection.
        /// Real values are substituted by the proxy in outbound traffic only.
        #[arg(long)]
        placeholder_secrets: bool,
        /// Write a verifiable execution receipt JSON to this path
        #[arg(long)]
        receipt: Option<PathBuf>,
    },
    /// Execute a command in a running sandbox
    Exec {
        /// Name of the sandbox
        name: String,
        /// Environment variables to set (KEY=VALUE format, can be repeated)
        #[arg(short, long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Working directory inside the sandbox
        #[arg(short, long)]
        workdir: Option<String>,
        /// Run as root
        #[arg(long)]
        sudo: bool,
        /// Run detached (in background). Returns a command ID for status/logs/kill.
        #[arg(short, long)]
        detach: bool,
        /// Write a verifiable execution receipt JSON to this path
        #[arg(long)]
        receipt: Option<PathBuf>,
        /// Command to execute
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
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

    // -- Setup --
    /// Set up agentkernel (download kernel, rootfs, Firecracker)
    Setup {
        /// Run non-interactively with defaults
        #[arg(short = 'y', long)]
        yes: bool,
    },
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

    // -- Grouped subcommands --
    /// Manage sandboxes (lifecycle, files, export)
    #[command(visible_alias = "sb")]
    Sandbox {
        #[command(subcommand)]
        action: SandboxAction,
    },
    /// SSH access (connect, config, proxy)
    Ssh {
        #[command(subcommand)]
        action: SshAction,
    },
    /// Manage snapshots
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },
    /// Manage agent sessions (tied sandbox + agent lifecycle)
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Manage sandbox templates
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Manage secrets (API keys and credentials)
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Manage agent plugins (install integration files for Claude, Codex, Gemini, etc.)
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Manage persistent volumes
    Volume {
        #[command(subcommand)]
        action: VolumeAction,
    },
    /// Manage Docker image cache (list, prune, pull)
    Images {
        #[command(subcommand)]
        action: ImagesAction,
    },
    /// Manage the daemon (VM pool server)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    // -- Servers --
    /// Start HTTP API server for programmatic access
    Serve {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on
        #[arg(short, long, default_value = "18888")]
        port: u16,
        /// API key for authentication (overrides AGENTKERNEL_API_KEY env var). Can be repeated.
        #[arg(long)]
        api_key: Vec<String>,
        /// Path to file containing API keys (one per line)
        #[arg(long)]
        api_key_file: Option<String>,
        /// Enable TLS for the API server
        #[arg(long)]
        tls: bool,
        /// Path to TLS certificate PEM file
        #[arg(long, requires = "tls")]
        tls_cert: Option<String>,
        /// Path to TLS private key PEM file
        #[arg(long, requires = "tls")]
        tls_key: Option<String>,
        /// Require TLS (reject plain HTTP)
        #[arg(long)]
        require_tls: bool,
        /// OpenTelemetry OTLP endpoint for trace export (e.g. http://localhost:4318)
        #[arg(long)]
        otel_endpoint: Option<String>,
        /// Webhook URL for event notifications (can be repeated)
        #[arg(long)]
        webhook_url: Vec<String>,
    },
    /// Start MCP server for Claude Code integration (JSON-RPC over stdio)
    McpServer,

    // -- Observability --
    /// List supported AI agents and their availability
    Agents,
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
    /// Show usage statistics from audit log
    Stats {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
    /// System diagnostics and health check
    Doctor,
    /// Benchmark sandbox backends on your hardware
    Benchmark {
        /// Comma-separated backends to test (default: all available)
        #[arg(short, long)]
        backends: Option<String>,
        /// Number of measured iterations per backend (default: 1)
        #[arg(short, long, default_value = "1")]
        iterations: usize,
        /// Number of unmeasured warmup iterations per backend (default: 1)
        #[arg(long, default_value = "1")]
        warmup: usize,
        /// Docker image to use for benchmark
        #[arg(long, default_value = "alpine:3.24")]
        image: String,
        /// Output machine-readable JSON instead of the table view
        #[arg(long)]
        json: bool,
        /// Optional file path to write the benchmark report JSON
        #[arg(long)]
        output: Option<PathBuf>,
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
    /// Verify and replay execution receipts
    Receipt {
        #[command(subcommand)]
        action: ReceiptAction,
    },

    // -- Workflows --
    /// Run a multi-step agent pipeline (chain sandboxes with data flow)
    Pipeline {
        /// Path to pipeline.toml file
        file: PathBuf,
        /// Backend to use for pipeline sandboxes
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// Run multiple jobs in parallel (fan-out, fan-in results)
    Parallel {
        /// Jobs in format "name:image:command" or "name:image:tag:command" (repeatable)
        #[arg(short, long, required = true)]
        job: Vec<String>,
        /// Backend to use
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// Build a custom image from a Dockerfile
    Build {
        /// Name/tag for the built image
        #[arg(short = 't', long = "tag")]
        name: String,
        /// Build context directory
        #[arg(default_value = ".")]
        context: PathBuf,
        /// Path to Dockerfile (default: Dockerfile in context)
        #[arg(short = 'f', long)]
        dockerfile: Option<PathBuf>,
    },
    /// Manage LLM API keys (org-level key injection)
    Llm {
        #[command(subcommand)]
        action: LlmAction,
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
enum LlmAction {
    /// Manage LLM provider API keys
    Keys {
        #[command(subcommand)]
        action: LlmKeysAction,
    },
}

#[derive(Subcommand)]
enum LlmKeysAction {
    /// List configured LLM provider keys
    List,
    /// Set an LLM provider key (stored in secrets vault, mapped to a domain)
    Set {
        /// Provider shorthand or domain (e.g. "openai", "anthropic", "api.openai.com")
        provider: String,
        /// Vault key name (e.g. OPENAI_API_KEY). Read from stdin if --key not provided.
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove an LLM provider key mapping
    Remove {
        /// Provider shorthand or domain
        provider: String,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum SandboxAction {
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
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad, daytona, runloop, e2b, modal, agentcomputer (default: auto-detect)
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
        /// Publish a port (host:container, container, or host:container/udp). Can be repeated.
        #[arg(short = 'p', long = "publish")]
        publish: Vec<String>,
        /// Enable SSH access to the sandbox
        #[arg(long)]
        ssh: bool,
        /// Clone a git repo into the sandbox (e.g. git:https://github.com/user/repo or just a URL)
        #[arg(long)]
        source: Option<String>,
        /// Git ref to checkout after cloning (branch, tag, or commit)
        #[arg(long)]
        git_ref: Option<String>,
        /// Mount a persistent volume (slug:/path or slug:/path:ro). Can be repeated.
        #[arg(short = 'v', long = "volume")]
        volumes: Vec<String>,
        /// Bind a secret to a host via proxy (KEY:host, KEY=value:host, KEY:host:header). Can be repeated.
        #[arg(short = 'S', long = "secret")]
        secrets: Vec<String>,
        /// Inject a secret as a file inside the sandbox (KEY from vault). Can be repeated.
        #[arg(long = "secret-file")]
        secret_files: Vec<String>,
        /// Use placeholder tokens instead of real secret values in file injection.
        #[arg(long)]
        placeholder_secrets: bool,
        /// Add a label (key=value). Can be repeated.
        #[arg(short = 'l', long = "label")]
        labels: Vec<String>,
        /// Don't auto-start the sandbox after creation
        #[arg(long)]
        no_start: bool,
    },
    /// Start a sandbox
    Start {
        /// Name of the sandbox to start
        name: String,
        /// Backend to use: docker, podman, firecracker, apple, hyperlight, kubernetes, nomad, daytona, runloop, e2b, modal, agentcomputer (default: auto-detect)
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
    /// List all sandboxes
    List {
        /// Filter to sandboxes matching the current git project
        #[arg(long)]
        project: bool,
        /// Filter by label (key=value). Can be repeated.
        #[arg(short = 'l', long = "label")]
        labels: Vec<String>,
    },
    /// Show detailed information about a sandbox
    Info {
        /// Name of the sandbox
        name: String,
    },
    /// Copy files to/from a running sandbox
    ///
    /// Examples:
    ///   agentkernel sandbox cp ./local/file my-sandbox:/remote/path
    ///   agentkernel sandbox cp my-sandbox:/remote/path ./local/file
    Cp {
        /// Source path (./local/file or sandbox:/path)
        source: String,
        /// Destination path (./local/file or sandbox:/path)
        dest: String,
    },
    /// Extend a sandbox's time-to-live
    ExtendTtl {
        /// Name of the sandbox
        name: String,
        /// Additional time (e.g. 1h, 30m, 2d). Adds to current expiry.
        #[arg(long, default_value = "1h")]
        by: String,
    },
    /// Export a sandbox's filesystem as a tar archive
    Export {
        /// Sandbox name
        name: String,
        /// Output file (default: <name>.tar)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Export a sandbox's configuration as TOML (for sharing/re-import)
    ExportConfig {
        /// Sandbox name
        name: String,
    },
    /// Import a sandbox configuration from TOML
    ImportConfig {
        /// Path to the TOML config file
        file: PathBuf,
        /// Name for the imported sandbox
        #[arg(long, value_name = "NAME")]
        r#as: Option<String>,
        /// Backend to use
        #[arg(short = 'B', long)]
        backend: Option<String>,
    },
    /// List detached commands in a sandbox
    ExecList {
        /// Name of the sandbox
        name: String,
    },
    /// Get logs from a detached command
    ExecLogs {
        /// Name of the sandbox
        name: String,
        /// Command ID (from exec --detach)
        id: String,
        /// Show stderr instead of stdout
        #[arg(long)]
        stderr: bool,
    },
    /// Kill a detached command
    ExecKill {
        /// Name of the sandbox
        name: String,
        /// Command ID (from exec --detach)
        id: String,
    },
    /// Garbage-collect expired sandboxes
    Gc {
        /// Show what would be removed without removing
        #[arg(long)]
        dry_run: bool,
        /// Filter by label (key=value). Only GC sandboxes matching all labels.
        #[arg(short = 'l', long = "label")]
        labels: Vec<String>,
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
}

#[derive(Subcommand)]
enum SshAction {
    /// SSH into a running sandbox
    Connect {
        /// Name of the sandbox
        name: String,
        /// Record session to asciicast v2 file (for replay with asciinema)
        #[arg(long)]
        record: Option<PathBuf>,
        /// Command to execute (instead of interactive shell)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Generate SSH config for IDE integration (VS Code Remote SSH, JetBrains, Cursor)
    Config {
        /// Name of the sandbox (omit for all SSH-enabled sandboxes)
        name: Option<String>,
        /// Generate config for all SSH-enabled sandboxes
        #[arg(long)]
        all: bool,
    },
    /// SSH proxy command for ProxyCommand integration (handles cert signing transparently)
    Proxy {
        /// Name of the sandbox
        name: String,
    },
}

#[derive(Subcommand)]
enum VolumeAction {
    /// Create a new volume
    Create {
        /// Volume slug (e.g., my-data)
        slug: String,
        /// Size limit (e.g., 2GB, 512MB). Default: unlimited
        #[arg(short, long)]
        size: Option<String>,
    },
    /// List all volumes
    List,
    /// Show volume details
    Info {
        /// Volume slug
        slug: String,
    },
    /// Delete a volume
    Delete {
        /// Volume slug
        slug: String,
        /// Force delete without confirmation
        #[arg(short, long)]
        force: bool,
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
enum ImagesAction {
    /// List Docker images (--all shows all images, default shows agentkernel images only)
    List {
        /// Show all images, not just agentkernel-related
        #[arg(short, long)]
        all: bool,
    },
    /// Remove unused images to free disk space
    Prune {
        /// Only prune agentkernel-built images (default: prune all dangling)
        #[arg(long)]
        agentkernel_only: bool,
    },
    /// Pre-pull a Docker image
    Pull {
        /// Image to pull (e.g. python:3.12-alpine)
        image: String,
    },
    /// List locally built images (from 'agentkernel build')
    LocalList,
    /// Delete a locally built image
    LocalDelete {
        /// Image name
        name: String,
    },
    /// Sync local image metadata with Docker (remove stale entries)
    LocalSync,
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
}

#[derive(Subcommand)]
enum ReceiptAction {
    /// Verify receipt integrity (hash-based tamper check)
    Verify {
        /// Path to the receipt JSON file
        file: PathBuf,
        /// Allow verification of legacy unsigned receipts
        #[arg(long)]
        allow_unsigned: bool,
    },
    /// Replay the recorded command and compare output hash
    Replay {
        /// Path to the receipt JSON file
        file: PathBuf,
        /// Allow replay of legacy unsigned receipts
        #[arg(long)]
        allow_unsigned: bool,
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
    runtime::ensure_host_command_path();
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup { yes } => {
            run_setup(yes).await?;
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
# git_name = "AgentKernel Agent"
# git_email = "agent@agentkernel.dev"

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
        Commands::Sandbox { action } => match action {
            SandboxAction::Create {
                name,
                agent,
                config,
                dir,
                backend,
                template: tmpl,
                ttl,
                branch,
                publish,
                ssh: ssh_flag,
                source,
                git_ref,
                volumes,
                secrets: secret_bindings_raw,
                secret_files: secret_file_keys,
                placeholder_secrets,
                labels: label_args,
                no_start,
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
                        anyhow::anyhow!(
                            "Sandbox name required. Use --branch to auto-derive from git."
                        )
                    })?
                };

                // Validate sandbox name (security: prevents command injection)
                validation::validate_sandbox_name(&name)?;
                if git_ref.is_some() && source.is_none() {
                    bail!("--git-ref requires --source");
                }
                if let Some(ref source_url) = source {
                    let url = source_url.strip_prefix("git:").unwrap_or(source_url);
                    validation::validate_git_source_url(url)?;
                }
                if let Some(ref git_ref_val) = git_ref {
                    validation::validate_git_ref(git_ref_val)?;
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

                // Local runtimes require setup. Explicit remote backends do not.
                if backend_type.is_none_or(|backend| !backend.is_remote()) {
                    let status = check_installation();
                    if !status.is_ready() {
                        bail!(
                            "Agentkernel is not fully set up. Run 'agentkernel setup' first.\n\
                         Missing: {}",
                            missing_components(&status)
                        );
                    }
                }

                // Load config: --config > --template > minimal default
                let mut template_metadata: Option<(String, Option<String>)> = None;
                let (cfg, config_base_dir) = if let Some(ref config_path) = config {
                    let cfg = Config::from_file(config_path)?;
                    let base_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    (cfg, Some(base_dir))
                } else if let Some(ref tmpl_name) = tmpl {
                    let resolved = template::resolve(tmpl_name)?;
                    println!("Using template '{}' ({})", resolved.name, resolved.source);
                    template_metadata = Some((
                        resolved.name.clone(),
                        extract_template_help_text(&resolved.content),
                    ));
                    let mut cfg = resolved.parse()?;
                    cfg.sandbox.name = name.clone();
                    (cfg, None)
                } else {
                    (Config::minimal(&name, &agent), None)
                };
                let stored_config_path = config
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string());
                let workspace_root =
                    resolve_workspace_root(config_base_dir.as_deref(), dir.as_deref())?;

                // Validate config and print warnings
                for warning in cfg.validate() {
                    eprintln!("Warning: {}", warning);
                }

                let start_perms = cfg.get_permissions();
                let start_files = if let Some(ref base_dir) = config_base_dir {
                    cfg.load_files(base_dir)?
                } else {
                    Vec::new()
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

                let ttl_secs = ttl.map(|t| parse_ttl(&t)).transpose()?.filter(|&s| s > 0); // 0 means no expiry

                // Parse port mappings from CLI --publish flags and config file [network].ports
                let mut ports: Vec<crate::backend::PortMapping> = publish
                    .iter()
                    .map(|s| crate::backend::PortMapping::parse(s))
                    .collect::<Result<Vec<_>>>()?;

                // Merge config file ports (CLI takes precedence, config adds extras)
                if let Ok(config_ports) = cfg.network.port_mappings() {
                    for cp in config_ports {
                        if !ports.contains(&cp) {
                            ports.push(cp);
                        }
                    }
                }

                // Handle --ssh flag: add SSH port mapping and configure SSH
                let enable_ssh = ssh_flag || cfg.security.transport.ssh;
                if enable_ssh {
                    // Auto-add SSH port mapping if not already present
                    let has_ssh_port = ports.iter().any(|p| p.container_port == 22);
                    if !has_ssh_port {
                        ports.push(crate::backend::PortMapping {
                            host_port: None, // auto-assign
                            container_port: 22,
                            protocol: crate::backend::PortProtocol::Tcp,
                        });
                    }
                    println!("  SSH: enabled (certificate-only auth)");
                }

                if !ports.is_empty() {
                    println!(
                        "  Ports: {}",
                        ports
                            .iter()
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }

                // Parse volume mounts
                let volume_mounts: Vec<volume::VolumeMount> = volumes
                    .iter()
                    .map(|s| volume::VolumeMount::parse(s))
                    .collect::<Result<Vec<_>>>()?;

                // Validate volumes exist
                if !volume_mounts.is_empty() {
                    let vol_manager = volume::VolumeManager::new()?;
                    vol_manager.validate_mounts(&volume_mounts)?;
                    println!(
                        "  Volumes: {}",
                        volume_mounts
                            .iter()
                            .map(|v| format!("{}:{}", v.slug, v.mount_path))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }

                // Parse secret bindings
                let mut parsed_bindings = Vec::new();
                if !secret_bindings_raw.is_empty() {
                    let vault = secrets::SecretVault::new(secrets::SecretBackend::default());
                    for raw in &secret_bindings_raw {
                        let (binding, inline_value) = proxy::SecretBinding::parse_cli(raw)?;
                        // If inline value provided, store it in the vault
                        if let Some(val) = inline_value {
                            vault.set(&binding.secret_key, &val)?;
                        }
                        println!(
                            "  Secret: {} -> {} (header: {})",
                            binding.secret_key, binding.target_host, binding.header_name
                        );
                        parsed_bindings.push(binding);
                    }
                }

                manager
                    .create_with_options(
                        &name,
                        &docker_image,
                        cfg.resources.vcpus,
                        cfg.resources.memory_mb,
                        ttl_secs,
                        ports,
                    )
                    .await?;
                manager.set_config_path(&name, stored_config_path)?;
                if start_perms.mount_cwd {
                    manager
                        .set_work_dir(&name, Some(workspace_root.to_string_lossy().to_string()))?;
                }

                // Parse and set labels
                if !label_args.is_empty() {
                    let mut labels = std::collections::HashMap::new();
                    for raw in &label_args {
                        let (k, v) = raw.split_once('=').ok_or_else(|| {
                            anyhow::anyhow!("Invalid label '{}': expected key=value format", raw)
                        })?;
                        validation::validate_label(k, v)?;
                        labels.insert(k.to_string(), v.to_string());
                    }
                    manager.set_labels(&name, &labels)?;
                    println!("  Labels: {}", label_args.join(", "));
                }

                if let Some((template_name, template_help_text)) = &template_metadata {
                    manager.set_template_metadata(
                        &name,
                        Some(template_name.as_str()),
                        template_help_text.as_deref(),
                    )?;
                }

                // Persist template secret_mappings (env_var → host) so
                // the UI can show which secrets the template expects,
                // even when the env vars aren't set on this host.
                if !cfg.secrets.is_empty() {
                    let mappings: std::collections::HashMap<String, String> = cfg
                        .secrets
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    manager.set_secret_mappings(&name, &mappings)?;
                }

                // Build secret bindings from template config [secrets] section
                let mut template_bindings = Vec::new();
                for (env_var, target_host) in &cfg.secrets {
                    if let Ok(val) = std::env::var(env_var) {
                        template_bindings.push(format!("{}={}:{}", env_var, val, target_host));
                    }
                }

                // Merge CLI-provided and template secret bindings (CLI takes precedence)
                let mut all_bindings: Vec<String> = secret_bindings_raw;
                all_bindings.extend(template_bindings);

                // Store secret bindings in sandbox state
                if !all_bindings.is_empty() {
                    // Deduplicate by env var name (first occurrence wins → CLI over template)
                    let mut seen = std::collections::HashSet::new();
                    all_bindings.retain(|b| {
                        let key = b.split('=').next().unwrap_or(b);
                        seen.insert(key.to_string())
                    });
                    if !parsed_bindings.is_empty() || !all_bindings.is_empty() {
                        manager.set_secret_bindings(&name, &all_bindings)?;
                    }
                }

                // Store secret file keys in sandbox state
                if !secret_file_keys.is_empty() {
                    for key in &secret_file_keys {
                        vsock_secrets::validate_secret_key(key)?;
                    }
                    manager.set_secret_files(&name, &secret_file_keys)?;
                    if placeholder_secrets {
                        manager.set_placeholder_secrets(&name, true)?;
                        println!(
                            "  Secret files: {} key(s) with placeholder tokens at {} (proxy substitution)",
                            secret_file_keys.len(),
                            vsock_secrets::DEFAULT_SECRETS_PATH,
                        );
                    } else {
                        println!(
                            "  Secret files: {} key(s) will be injected at {}",
                            secret_file_keys.len(),
                            vsock_secrets::DEFAULT_SECRETS_PATH,
                        );
                    }
                }

                // If SSH enabled, update the sandbox state
                if enable_ssh {
                    manager.set_ssh_enabled(&name, true)?;
                }

                // Store init script from template config
                if let Some(ref script) = cfg.sandbox.init_script {
                    manager.set_init_script(&name, script)?;
                }

                println!("\nSandbox '{}' created.", name);
                if let Some(secs) = ttl_secs {
                    println!("  TTL: {} (expires automatically)", format_ttl(secs));
                }

                // Auto-start the sandbox unless --no-start was passed
                if !no_start {
                    let has_secrets = !all_bindings.is_empty();
                    let api_port = std::env::var("AGENTKERNEL_PORT")
                        .ok()
                        .and_then(|p| p.parse::<u16>().ok())
                        .unwrap_or(18888);
                    let server_running = try_server_health("127.0.0.1", api_port).await;

                    if has_secrets && server_running {
                        // Delegate to HTTP server so the secrets proxy
                        // lives in the long-running server process
                        println!("Starting sandbox via API server (secrets proxy will persist)...");
                        delegate_start_to_server("127.0.0.1", api_port, &name).await?;
                    } else if has_secrets && !server_running {
                        eprintln!(
                            "Warning: Secrets proxy requires 'agentkernel serve' to be running.\n\
                             Start the server first, then run: agentkernel start {}",
                            name
                        );
                    } else {
                        // No secrets — start locally with config-derived permissions/files.
                        manager
                            .start_with_permissions_and_files(&name, &start_perms, &start_files)
                            .await?;
                    }

                    // If --source provided, clone the repo into the
                    // (now-running) sandbox
                    if let Some(ref source_url) = source {
                        let url = source_url.strip_prefix("git:").unwrap_or(source_url);

                        // Only start here if we skipped auto-start above
                        // (secrets warning case — sandbox not yet running)
                        if has_secrets && !server_running {
                            println!(
                                "\nCannot clone repo: sandbox not started (secrets proxy requires 'agentkernel serve')."
                            );
                        } else {
                            println!("Cloning {}...", url);
                            // Install git if needed, then clone
                            let install_cmd = vec![
                                "sh".to_string(),
                                "-c".to_string(),
                                "which git >/dev/null 2>&1 || apk add --no-cache git >/dev/null 2>&1 || apt-get update -qq && apt-get install -y -qq git >/dev/null 2>&1 || yum install -y git >/dev/null 2>&1 || true".to_string(),
                            ];
                            let _ = manager.exec_cmd(&name, &install_cmd).await;

                            let clone_cmd = vec![
                                "git".to_string(),
                                "clone".to_string(),
                                url.to_string(),
                                "/workspace".to_string(),
                            ];
                            manager.exec_cmd(&name, &clone_cmd).await?;

                            if let Some(ref git_ref_val) = git_ref {
                                let checkout_cmd = vec![
                                    "git".to_string(),
                                    "-C".to_string(),
                                    "/workspace".to_string(),
                                    "checkout".to_string(),
                                    git_ref_val.clone(),
                                ];
                                manager.exec_cmd(&name, &checkout_cmd).await?;
                                println!("Cloned {} (ref: {}) into /workspace", url, git_ref_val);
                            } else {
                                println!("Cloned {} into /workspace", url);
                            }
                        }
                    }

                    // Print connection instructions (only if sandbox was started)
                    if !has_secrets || server_running {
                        println!("\nSandbox '{}' started.", name);
                        println!("To connect:");
                        if enable_ssh {
                            println!("  agentkernel ssh {}", name);
                        } else {
                            println!("  agentkernel attach {}", name);
                        }
                    }
                } else {
                    // --no-start: show manual next steps
                    println!("\nNext steps:");
                    println!("  1. agentkernel start {}", name);
                    if enable_ssh {
                        println!("  2. agentkernel ssh {}", name);
                    } else {
                        println!("  2. agentkernel attach {}", name);
                    }
                }
            }
            SandboxAction::Start { name, backend } => {
                validation::validate_sandbox_name(&name)?;

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

                let effective_backend = manager
                    .get_state(&name)
                    .and_then(|state| state.backend)
                    .unwrap_or(manager.backend());
                if !effective_backend.is_remote() {
                    let status = check_installation();
                    if !status.is_ready() {
                        bail!("Agentkernel is not fully set up. Run 'agentkernel setup' first.");
                    }
                }

                if !manager.exists(&name) {
                    bail!(
                        "Sandbox '{}' not found. Create it first with: agentkernel create {}",
                        name,
                        name
                    );
                }

                // Check if sandbox has secret bindings that need a long-lived proxy
                let has_secrets = manager
                    .get_sandbox_state(&name)
                    .is_some_and(|s| !s.secret_bindings.is_empty());

                if has_secrets {
                    let api_port = std::env::var("AGENTKERNEL_PORT")
                        .ok()
                        .and_then(|p| p.parse::<u16>().ok())
                        .unwrap_or(18888);
                    let server_running = try_server_health("127.0.0.1", api_port).await;
                    if server_running {
                        println!("Starting sandbox via API server (secrets proxy will persist)...");
                        delegate_start_to_server("127.0.0.1", api_port, &name).await?;
                        println!("Sandbox '{}' started.", name);
                        if manager
                            .get_sandbox_state(&name)
                            .is_some_and(|s| s.ssh_enabled)
                        {
                            println!("\nTo connect: agentkernel ssh {}", name);
                        } else {
                            println!("\nTo attach: agentkernel attach {}", name);
                        }
                        return Ok(());
                    } else {
                        eprintln!(
                            "Warning: Sandbox has secrets configured but 'agentkernel serve' is not running.\n\
                             The secrets proxy will stop when this command exits.\n\
                             For persistent proxy, start 'agentkernel serve' first."
                        );
                    }
                }

                let sandbox_config_path = manager
                    .get_state(&name)
                    .and_then(|state| state.config_path.clone())
                    .map(std::path::PathBuf::from);
                let config_path = sandbox_config_path
                    .filter(|path| path.exists())
                    .unwrap_or_else(|| std::path::PathBuf::from("agentkernel.toml"));
                let (start_perms, start_files) = if config_path.exists() {
                    let cfg = Config::from_file(&config_path)?;
                    for warning in cfg.validate() {
                        eprintln!("Warning: {}", warning);
                    }
                    let config_dir = config_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    (cfg.get_permissions(), cfg.load_files(config_dir)?)
                } else {
                    (crate::permissions::Permissions::default(), Vec::new())
                };

                println!("Starting sandbox '{}'...", name);
                manager
                    .start_with_permissions_and_files(&name, &start_perms, &start_files)
                    .await?;
                println!("Sandbox '{}' started.", name);
                if manager
                    .get_sandbox_state(&name)
                    .is_some_and(|s| s.ssh_enabled)
                {
                    println!("\nTo connect: agentkernel ssh {}", name);
                } else {
                    println!("\nTo attach: agentkernel attach {}", name);
                }
            }
            SandboxAction::Stop { name } => {
                validation::validate_sandbox_name(&name)?;

                let mut manager = VmManager::new()?;

                if !manager.exists(&name) {
                    bail!("Sandbox '{}' not found", name);
                }

                println!("Stopping sandbox '{}'...", name);
                manager.stop(&name).await?;
                println!("Sandbox '{}' stopped.", name);
            }
            SandboxAction::Remove { name } => {
                validation::validate_sandbox_name(&name)?;

                let mut manager = VmManager::new()?;
                println!("Removing sandbox '{}'...", name);
                manager.remove(&name).await?;
                println!("Sandbox '{}' removed.", name);
            }
            SandboxAction::ExtendTtl { name, by } => {
                validation::validate_sandbox_name(&name)?;

                let mut manager = VmManager::new()?;
                if !manager.exists(&name) {
                    bail!("Sandbox '{}' not found", name);
                }

                let additional_secs = crate::ssh::parse_ttl_to_secs(&by)?;
                let new_expiry = manager.extend_ttl(&name, additional_secs)?;

                match new_expiry {
                    Some(exp) => println!("Extended TTL for '{}'. New expiry: {}", name, exp),
                    None => println!("Sandbox '{}' now has no expiry (TTL disabled).", name),
                }
            }
            SandboxAction::ExecList { name } => {
                validation::validate_sandbox_name(&name)?;
                let manager = VmManager::new()?;
                let commands = manager.detached_list(Some(&name));
                println!("{}", serde_json::to_string_pretty(&commands)?);
            }
            SandboxAction::ExecLogs { name, id, stderr } => {
                validation::validate_sandbox_name(&name)?;
                let mut manager = VmManager::new()?;
                let stream = if stderr { Some("stderr") } else { None };
                let output = manager.detached_logs(&id, stream).await?;
                print!("{}", output);
            }
            SandboxAction::ExecKill { name, id } => {
                validation::validate_sandbox_name(&name)?;
                let mut manager = VmManager::new()?;
                manager.detached_kill(&id).await?;
                println!("Command {} killed", id);
            }
            SandboxAction::Cp { source, dest } => {
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
            SandboxAction::List {
                project,
                labels: label_filters,
            } => {
                let manager = VmManager::new()?;
                let vms = manager.list();

                // Parse label filters (key=value)
                let mut parsed_labels: Vec<(String, String)> = Vec::new();
                for raw in &label_filters {
                    if let Some((k, v)) = raw.split_once('=') {
                        parsed_labels.push((k.to_string(), v.to_string()));
                    } else {
                        eprintln!(
                            "Warning: ignoring malformed label filter '{}'; expected key=value",
                            raw
                        );
                    }
                }

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
                    .filter(|(name, _, _)| {
                        if parsed_labels.is_empty() {
                            return true;
                        }
                        let state = manager.get_state(name);
                        parsed_labels.iter().all(|(fk, fv)| {
                            state
                                .and_then(|s| s.labels.get(fk))
                                .is_some_and(|v| v == fv)
                        })
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
                    println!(
                        "{:<30} {:<10} {:<10} {:<17} {:<20} PORTS",
                        "NAME", "STATUS", "BACKEND", "IP", "LABELS"
                    );
                    for (name, running, backend) in filtered {
                        let status = if running { "running" } else { "stopped" };
                        let backend_str = backend
                            .map(|b| format!("{}", b))
                            .unwrap_or_else(|| "unknown".to_string());
                        let ip_str = if running {
                            manager
                                .get_container_ip(name)
                                .unwrap_or_else(|| "-".to_string())
                        } else {
                            "-".to_string()
                        };
                        let state = manager.get_state(name);
                        let ports_str = state
                            .as_ref()
                            .map(|s| {
                                s.ports
                                    .iter()
                                    .map(|p| p.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let labels_str = state
                            .as_ref()
                            .map(|s| {
                                s.labels
                                    .iter()
                                    .map(|(k, v)| format!("{}={}", k, v))
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_default();
                        println!(
                            "{:<30} {:<10} {:<10} {:<17} {:<20} {}",
                            name, status, backend_str, ip_str, labels_str, ports_str
                        );
                    }
                }
            }
            SandboxAction::Gc { dry_run, labels } => {
                let mut manager = VmManager::new()?;

                // Parse label filters
                let label_filters: Vec<(String, String)> = labels
                    .iter()
                    .filter_map(|raw| {
                        let (k, v) = raw.split_once('=')?;
                        Some((k.to_string(), v.to_string()))
                    })
                    .collect();

                let candidates = if label_filters.is_empty() {
                    // Default: only expired sandboxes
                    manager.expired()
                } else {
                    // With labels: find sandboxes matching all labels
                    // (expired OR label-matched, since the user explicitly asked for label-based GC)
                    manager.list_matching_labels(&label_filters)
                };

                if candidates.is_empty() {
                    println!("No matching sandboxes.");
                } else if dry_run {
                    println!("Would remove {} sandbox(es):", candidates.len());
                    for name in &candidates {
                        println!("  {}", name);
                    }
                } else {
                    let mut removed = Vec::new();
                    for name in candidates {
                        manager.remove(&name).await?;
                        removed.push(name);
                    }
                    println!("Removed {} sandbox(es):", removed.len());
                    for name in &removed {
                        println!("  {}", name);
                    }
                }
            }
            SandboxAction::Info { name } => {
                validation::validate_sandbox_name(&name)?;
                run_info(&name)?;
            }
            SandboxAction::Export { name, output } => {
                validation::validate_sandbox_name(&name)?;
                let output_file = output.unwrap_or_else(|| format!("{}.tar", name));

                // Use docker export to get the full filesystem
                let container_name = format!("agentkernel-{}", name);
                println!("Exporting sandbox '{}' to {}...", name, output_file);
                let status = std::process::Command::new("docker")
                    .args(["export", "-o", &output_file, &container_name])
                    .status()
                    .map_err(|e| anyhow::anyhow!("Failed to run docker export: {}", e))?;

                if !status.success() {
                    bail!("docker export failed for sandbox '{}'", name);
                }

                let size = std::fs::metadata(&output_file)?.len();
                let size_mb = size as f64 / 1_048_576.0;
                println!("Exported {:.1} MB to {}", size_mb, output_file);
            }
            SandboxAction::ExportConfig { name } => {
                let manager = VmManager::new()?;
                let state = manager
                    .get_state(&name)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

                let config = crate::config::SandboxConfigExport::from_parts(
                    &state.name,
                    &state.image,
                    state.init_script.as_deref(),
                    state.vcpus,
                    state.memory_mb,
                    state.agent.as_deref(),
                    state.ports.iter().map(ToString::to_string).collect(),
                );

                print!("{}", toml::to_string_pretty(&config)?);
            }
            SandboxAction::ImportConfig {
                file,
                r#as: as_name,
                backend,
            } => {
                if !file.exists() {
                    bail!("Config file not found: {}", file.display());
                }

                let cfg = Config::from_file(&file)?;
                let name = as_name.unwrap_or_else(|| cfg.sandbox.name.clone());
                validation::validate_sandbox_name(&name)?;

                let backend_type = if let Some(ref b) = backend {
                    Some(
                        b.parse::<crate::backend::BackendType>()
                            .map_err(|e| anyhow::anyhow!(e))?,
                    )
                } else {
                    None
                };
                let mut manager = VmManager::with_backend(backend_type)?;

                let docker_image = cfg.docker_image();
                println!(
                    "Importing config as sandbox '{}' (image: {})...",
                    name, docker_image
                );
                manager
                    .create(
                        &name,
                        &docker_image,
                        cfg.resources.vcpus,
                        cfg.resources.memory_mb,
                    )
                    .await?;

                println!("Sandbox '{}' created from config.", name);
                println!("\nNext steps:");
                println!("  agentkernel start {}", name);
            }
            SandboxAction::Clean { force, all } => {
                run_clean(force, all).await?;
            }
        },
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
        Commands::Exec {
            name,
            env,
            workdir,
            sudo,
            detach,
            receipt: receipt_path,
            command,
        } => {
            validation::validate_sandbox_name(&name)?;

            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel exec <name> <command...>");
            }
            if let Some(ref dir) = workdir {
                validation::validate_exec_workdir(dir)?;
            }

            let mut manager = VmManager::new()?;

            if !manager.exists(&name) {
                bail!("Sandbox '{}' not found", name);
            }

            let receipt_invocation = receipt_path.as_ref().map(|_| {
                receipt::Invocation::Exec(receipt::ExecInvocation {
                    name: name.clone(),
                    command: command.clone(),
                    env: env.clone(),
                    workdir: workdir.clone(),
                    sudo,
                })
            });
            let error_details = |e: &anyhow::Error| -> (i32, String, Option<String>) {
                if let Some(failed) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                    (failed.exit_code, failed.output.clone(), Some(e.to_string()))
                } else {
                    (1, String::new(), Some(e.to_string()))
                }
            };

            let opts = crate::backend::ExecOptions {
                env: env.clone(),
                workdir: workdir.clone(),
                user: if sudo { Some("root".to_string()) } else { None },
            };

            if detach {
                if receipt_path.is_some() {
                    bail!("--receipt is not supported with --detach");
                }
                let cmd = manager.exec_detached(&name, &command, &opts).await?;
                println!("{}", serde_json::to_string_pretty(&cmd)?);
            } else {
                let result = manager.exec_cmd_full(&name, &command, &opts).await;
                match result {
                    Ok(output) => {
                        print!("{}", output);
                        if let (Some(path), Some(invocation)) =
                            (receipt_path.as_ref(), receipt_invocation.clone())
                        {
                            let outcome =
                                receipt::ExecutionOutcome::from_combined_output(0, &output, None);
                            let rec = receipt::ExecutionReceipt::new(invocation, outcome)?;
                            receipt::write_receipt(path, &rec)?;
                            eprintln!("Execution receipt written to {}", path.display());
                        }
                    }
                    Err(e) => {
                        if let (Some(path), Some(invocation)) =
                            (receipt_path.as_ref(), receipt_invocation.clone())
                        {
                            let (exit_code, combined_output, error_message) = error_details(&e);
                            let outcome = receipt::ExecutionOutcome::from_combined_output(
                                exit_code,
                                &combined_output,
                                error_message,
                            );
                            let rec = receipt::ExecutionReceipt::new(invocation, outcome)?;
                            receipt::write_receipt(path, &rec)?;
                            eprintln!("Execution receipt written to {}", path.display());
                        }
                        return Err(e);
                    }
                }
            }
        }
        Commands::Run {
            command,
            config,
            keep,
            image,
            build: build_image,
            profile,
            no_network,
            fast,
            backend,
            template: tmpl,
            ttl,
            branch,
            publish,
            ssh: ssh_flag,
            receipt: receipt_path,
            ..
        } => {
            if command.is_empty() {
                bail!("No command specified. Usage: agentkernel run [OPTIONS] <command...>");
            }

            let write_run_receipt = |path: &Path,
                                     image: Option<String>,
                                     backend_name: Option<String>,
                                     exit_code: i32,
                                     combined_output: &str,
                                     error_message: Option<String>|
             -> Result<()> {
                let invocation = receipt::Invocation::Run(receipt::RunInvocation {
                    command: command.clone(),
                    image,
                    backend: backend_name,
                    profile: profile.clone(),
                    no_network,
                    fast,
                    keep,
                });
                let outcome = receipt::ExecutionOutcome::from_combined_output(
                    exit_code,
                    combined_output,
                    error_message,
                );
                let rec = receipt::ExecutionReceipt::new(invocation, outcome)?;
                receipt::write_receipt(path, &rec)
            };
            let error_details = |e: &anyhow::Error| -> (i32, String, Option<String>) {
                if let Some(failed) = e.downcast_ref::<crate::vmm::CommandFailed>() {
                    (failed.exit_code, failed.output.clone(), Some(e.to_string()))
                } else {
                    (1, String::new(), Some(e.to_string()))
                }
            };

            // Warn if --ssh and --no-network are both set
            if ssh_flag && no_network {
                eprintln!(
                    "Warning: --ssh and --no-network are both set. \
                     SSH requires network access; port mapping will not work without it."
                );
            }

            // Parse port mappings
            let port_mappings: Vec<crate::backend::PortMapping> = publish
                .iter()
                .map(|s| crate::backend::PortMapping::parse(s))
                .collect::<Result<Vec<_>>>()?;

            // Fast path: use container pool for ephemeral runs
            if fast {
                if keep {
                    bail!("Cannot use --fast with --keep (pooled containers are ephemeral)");
                }
                if !port_mappings.is_empty() {
                    bail!(
                        "Cannot use --fast with -p/--publish (pooled containers don't support port mapping)"
                    );
                }
                if image.is_some() || config.is_some() {
                    eprintln!(
                        "Warning: --image and --config are ignored with --fast (pool uses alpine:3.24)"
                    );
                }

                match VmManager::run_pooled(&command).await {
                    Ok(output) => {
                        print!("{}", output);
                        if let Some(path) = receipt_path.as_ref() {
                            write_run_receipt(
                                path,
                                Some("alpine:3.24".to_string()),
                                None,
                                0,
                                &output,
                                None,
                            )?;
                            eprintln!("Execution receipt written to {}", path.display());
                        }
                    }
                    Err(e) => {
                        if let Some(path) = receipt_path.as_ref() {
                            let (exit_code, combined_output, error_message) = error_details(&e);
                            write_run_receipt(
                                path,
                                Some("alpine:3.24".to_string()),
                                None,
                                exit_code,
                                &combined_output,
                                error_message,
                            )?;
                            eprintln!("Execution receipt written to {}", path.display());
                        }
                        return Err(e);
                    }
                }
                return Ok(());
            }

            // Daemon path: try daemon VM pool first (single round-trip)
            // Skip is_available() check - just try and fall back on error
            if !keep && !build_image {
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
                    if let Some(path) = receipt_path.as_ref() {
                        let combined_output = format!("{}{}", result.stdout, result.stderr);
                        write_run_receipt(
                            path,
                            None,
                            None,
                            result.exit_code,
                            &combined_output,
                            if result.exit_code == 0 {
                                None
                            } else {
                                Some(format!("Command exited with code {}", result.exit_code))
                            },
                        )?;
                        eprintln!("Execution receipt written to {}", path.display());
                    }
                    if result.exit_code != 0 {
                        std::process::exit(result.exit_code);
                    }
                    return Ok(());
                }
                // Daemon not available or failed, fall through to ephemeral mode
            }

            // Determine Docker image: --image > --config > --template > command > ./agentkernel.toml > project files > default
            // For `run`, command detection has higher priority than project files
            // because user is explicitly specifying what to run
            let explicit_image = image.is_some();
            let (docker_image, cfg_for_build, honor_config_dockerfile) = if let Some(img) = image {
                (img, None, false)
            } else if let Some(ref config_path) = config {
                let cfg = Config::from_file(config_path)?;
                (cfg.docker_image(), Some(cfg), true)
            } else if let Some(ref tmpl_name) = tmpl {
                let resolved = template::resolve(tmpl_name)?;
                eprintln!("Using template '{}' ({})", resolved.name, resolved.source);
                let cfg = resolved.parse()?;
                (cfg.docker_image(), Some(cfg), true)
            } else if let Some(img) = languages::detect_from_command(&command) {
                // Command-based detection first for `run`
                (img, None, false)
            } else {
                // Try current directory config
                let default_config = PathBuf::from("agentkernel.toml");
                if default_config.exists() {
                    let cfg = Config::from_file(&default_config)?;
                    let has_build_settings = config_has_build_settings(&cfg);
                    (cfg.docker_image(), Some(cfg), has_build_settings)
                } else {
                    // Fall back to project file detection or default
                    (languages::detect_image(&command), None, false)
                }
            };

            // Resolve whether this invocation intentionally selects a Dockerfile build
            let current_dir = std::env::current_dir()?;
            let is_firecracker_backend = backend
                .as_ref()
                .is_some_and(|b| b == "firecracker" || b == "fc");

            // Build only when configuration selected a Dockerfile or the user
            // explicitly requested an ambient Dockerfile with --build. A plain
            // command-driven run must not turn into a potentially multi-minute
            // project image build just because the working directory has one.
            let should_build_image = should_build_run_image(
                explicit_image,
                build_image,
                honor_config_dockerfile,
                cfg_for_build.as_ref(),
                &current_dir,
            );
            if build_image && !should_build_image {
                bail!(
                    "--build requested but no Dockerfile was found in {}",
                    current_dir.display()
                );
            }

            let docker_image = if should_build_image {
                if let Some(ref cfg) = cfg_for_build {
                    // Use config's build settings
                    let project_name = &cfg.sandbox.name;
                    build::build_or_use_image(project_name, &docker_image, &current_dir, cfg)?
                } else {
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
                }
            } else {
                docker_image
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
            let selected_backend = if let Some(bt) = backend_type {
                bt
            } else {
                crate::backend::detect_best_backend().ok_or_else(|| {
                    anyhow::anyhow!(
                        "No sandbox backend available. Need one of: KVM (Linux), Apple containers (macOS 26+), or Docker/Podman."
                    )
                })?
            };

            // Optimized path: use run_ephemeral for single-operation execution
            // This is faster than create→start→exec→stop→remove cycle:
            // - Docker: single `docker run --rm` command
            // - Apple containers: single `container run --rm` (~940ms vs ~2200ms)
            // Only used when --keep is not specified
            if !keep {
                match VmManager::run_ephemeral_with_backend(
                    selected_backend,
                    &docker_image,
                    &command,
                    &perms,
                    &files,
                )
                .await
                {
                    Ok(output) => {
                        print!("{}", output);
                        if let Some(path) = receipt_path.as_ref() {
                            write_run_receipt(
                                path,
                                Some(docker_image.clone()),
                                Some(selected_backend.to_string()),
                                0,
                                &output,
                                None,
                            )?;
                            eprintln!("Execution receipt written to {}", path.display());
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        // Firecracker doesn't support ephemeral mode, fall through to multi-step
                        if !e.to_string().contains("Ephemeral mode not supported") {
                            if let Some(path) = receipt_path.as_ref() {
                                let (exit_code, combined_output, error_message) = error_details(&e);
                                write_run_receipt(
                                    path,
                                    Some(docker_image.clone()),
                                    Some(selected_backend.to_string()),
                                    exit_code,
                                    &combined_output,
                                    error_message,
                                )?;
                                eprintln!("Execution receipt written to {}", path.display());
                            }
                            bail!("{}", e);
                        }
                    }
                }
            }

            let mut manager = VmManager::with_backend(Some(selected_backend))?;

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
                    let result = manager.exec_cmd(&name, &command).await;
                    match result {
                        Ok(output) => {
                            print!("{}", output);
                            if let Some(path) = receipt_path.as_ref() {
                                let existing_image = manager
                                    .get_state(&name)
                                    .map(|s| s.image.clone())
                                    .unwrap_or_else(|| docker_image.clone());
                                let existing_backend = manager
                                    .get_state(&name)
                                    .and_then(|s| s.backend)
                                    .map(|b| b.to_string())
                                    .unwrap_or_else(|| format!("{}", manager.backend()));
                                write_run_receipt(
                                    path,
                                    Some(existing_image),
                                    Some(existing_backend),
                                    0,
                                    &output,
                                    None,
                                )?;
                                eprintln!("Execution receipt written to {}", path.display());
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            if let Some(path) = receipt_path.as_ref() {
                                let (exit_code, combined_output, error_message) = error_details(&e);
                                let existing_image = manager
                                    .get_state(&name)
                                    .map(|s| s.image.clone())
                                    .unwrap_or_else(|| docker_image.clone());
                                let existing_backend = manager
                                    .get_state(&name)
                                    .and_then(|s| s.backend)
                                    .map(|b| b.to_string())
                                    .unwrap_or_else(|| format!("{}", manager.backend()));
                                write_run_receipt(
                                    path,
                                    Some(existing_image),
                                    Some(existing_backend),
                                    exit_code,
                                    &combined_output,
                                    error_message,
                                )?;
                                eprintln!("Execution receipt written to {}", path.display());
                            }
                            return Err(e);
                        }
                    }
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
                let parsed = parse_ttl(t)?;
                if parsed > 0 { Some(parsed) } else { None } // 0 means no expiry
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

            if let Some(path) = receipt_path.as_ref() {
                let (exit_code, combined_output, error_message) = match &result {
                    Ok(output) => (0, output.clone(), None),
                    Err(e) => error_details(e),
                };
                write_run_receipt(
                    path,
                    Some(docker_image.clone()),
                    Some(format!("{}", manager.backend())),
                    exit_code,
                    &combined_output,
                    error_message,
                )?;
                eprintln!("Execution receipt written to {}", path.display());
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
        Commands::Serve {
            host,
            port,
            api_key,
            api_key_file,
            tls,
            tls_cert,
            tls_key,
            require_tls,
            otel_endpoint,
            webhook_url,
        } => {
            let addr: std::net::SocketAddr = format!("{}:{}", host, port)
                .parse()
                .expect("Invalid address");

            if require_tls && !tls {
                bail!("--require-tls requires --tls to be enabled");
            }

            // Collect API keys from --api-key flags and --api-key-file
            let mut api_keys = api_key;
            if let Some(path) = api_key_file {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read API key file: {}", path))?;
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        api_keys.push(trimmed.to_string());
                    }
                }
            }

            let tls_config = if tls {
                let self_signed = tls_cert.is_none() && tls_key.is_none();
                if self_signed {
                    eprintln!("TLS enabled with self-signed certificate");
                } else {
                    eprintln!(
                        "TLS enabled with cert={} key={}",
                        tls_cert.as_deref().unwrap_or("?"),
                        tls_key.as_deref().unwrap_or("?")
                    );
                }
                Some(tls::TlsConfig {
                    cert_path: tls_cert,
                    key_path: tls_key,
                    self_signed,
                    require_tls,
                })
            } else {
                None
            };

            http_api::run_server_with_tls(addr, tls_config, api_keys, otel_endpoint, webhook_url)
                .await?;
        }
        Commands::Ssh { action } => match action {
            SshAction::Connect {
                name,
                record,
                command,
            } => {
                let manager = VmManager::new()?;

                // 1. Look up the sandbox — it must exist and be SSH-enabled
                let state = manager
                    .get_sandbox_state(&name)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

                if !state.ssh_enabled {
                    bail!(
                        "SSH is not enabled on sandbox '{}'. \
                     Recreate it with --ssh to enable SSH access.",
                        name
                    );
                }

                // 2. Determine the SSH host port
                let host_port = state
                    .ssh_host_port
                    .or_else(|| {
                        state
                            .ports
                            .iter()
                            .find(|p| p.container_port == 22)
                            .and_then(|p| p.host_port)
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No SSH host port found for sandbox '{}'. \
                         The sandbox may need a port mapping for port 22.",
                            name
                        )
                    })?;

                // Audit: log SSH connection
                audit::log_event(audit::AuditEvent::SshConnected {
                    sandbox: name.clone(),
                    host_port,
                    ssh_user: "sandbox".to_string(),
                });
                let start_time = std::time::Instant::now();

                // 3. Read the CA private key saved during sandbox start
                let ca_key_path = manager.get_data_dir().join(format!("{}-ssh-ca.key", name));
                let ca_private_key = std::fs::read_to_string(&ca_key_path).map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to read CA key at {}: {}. \
                     Was the sandbox started with --ssh?",
                        ca_key_path.display(),
                        e
                    )
                })?;

                // 4. Generate an ephemeral client keypair
                let (client_private, client_public) = ssh::generate_client_keypair()?;

                // 5. Sign the client public key with the stored CA key
                let ttl_secs = ssh::parse_ttl_to_secs("30m")?;
                let cert = ssh::sign_client_key_local(
                    &ca_private_key,
                    &client_public,
                    &["sandbox"],
                    ttl_secs,
                )?;

                // 6. Write cert and private key to persistent location
                //    (~/.agentkernel/ssh/{name}/ so raw `ssh -i` works too)
                let ssh_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".agentkernel")
                    .join("ssh")
                    .join(&name);
                std::fs::create_dir_all(&ssh_dir)?;

                let client_key_path = ssh_dir.join("client_key");
                let cert_path = ssh_dir.join("client_key-cert.pub");
                std::fs::write(&client_key_path, &client_private)?;
                std::fs::write(&cert_path, &cert)?;

                // Set permissions on the private key (owner read-only)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &client_key_path,
                        std::fs::Permissions::from_mode(0o600),
                    )?;
                }

                // 7. Build the ssh command
                let mut ssh_cmd = std::process::Command::new("ssh");
                ssh_cmd
                    .arg("-o")
                    .arg("StrictHostKeyChecking=no")
                    .arg("-o")
                    .arg("UserKnownHostsFile=/dev/null")
                    .arg("-o")
                    .arg("LogLevel=ERROR")
                    .arg("-i")
                    .arg(&client_key_path)
                    .arg("-o")
                    .arg(format!("CertificateFile={}", cert_path.display()))
                    .arg("-p")
                    .arg(host_port.to_string());

                // Request PTY for interactive sessions when we have a terminal
                if command.is_empty() {
                    use std::io::IsTerminal;
                    if std::io::stdin().is_terminal() {
                        ssh_cmd.arg("-t");
                    }
                }

                ssh_cmd.arg("sandbox@localhost");

                // Append remote command if provided
                if !command.is_empty() {
                    ssh_cmd.arg("--");
                    for arg in &command {
                        ssh_cmd.arg(arg);
                    }
                }

                // Resolve recording path (if a directory, generate a filename)
                let record_path = record.map(|p| {
                    if p.is_dir() {
                        p.join(asciicast::generate_recording_name(&name))
                    } else {
                        p
                    }
                });

                if let Some(ref record_path) = record_path {
                    // 8a. Recording mode: capture output through pipes
                    if let Some(parent) = record_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let mut recorder = asciicast::AsciicastRecorder::with_header(
                        record_path,
                        asciicast::AsciicastHeader::from_terminal()
                            .with_title(format!("agentkernel ssh {}", name))
                            .with_command(format!("agentkernel ssh {}", name)),
                    );

                    let mut child = ssh_cmd
                        .stdin(std::process::Stdio::inherit())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to execute ssh command: {}. Is OpenSSH installed?",
                                e
                            )
                        })?;

                    // Read stdout and record it
                    if let Some(mut stdout) = child.stdout.take() {
                        use std::io::Read;
                        let mut buf = [0u8; 4096];
                        loop {
                            match stdout.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    let data = String::from_utf8_lossy(&buf[..n]);
                                    recorder.record_output(&*data);
                                    print!("{}", data);
                                    use std::io::Write;
                                    std::io::stdout().flush().ok();
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                                Err(_) => break,
                            }
                        }
                    }

                    let status = child.wait()?;
                    if let Err(e) = recorder.save() {
                        eprintln!("Warning: Failed to save recording: {}", e);
                    } else {
                        eprintln!("Session recording saved to: {}", record_path.display());
                        eprintln!(
                            "  Replay with: agentkernel replay {}",
                            record_path.display()
                        );
                    }

                    // Audit: log SSH disconnect
                    let duration = start_time.elapsed().as_secs();
                    audit::log_event(audit::AuditEvent::SshDisconnected {
                        sandbox: name.clone(),
                        duration_secs: duration,
                        recording: Some(record_path.display().to_string()),
                    });

                    // 9. Temp files cleaned up automatically when temp_dir drops

                    if !status.success() {
                        std::process::exit(status.code().unwrap_or(1));
                    }
                } else {
                    // 8b. Non-recording mode: exec() replaces this process with ssh
                    //     for proper terminal/PTY handling
                    eprintln!(
                        "  or: ssh -i {} -p {} sandbox@localhost",
                        client_key_path.display(),
                        host_port
                    );
                    use std::io::Write;
                    std::io::stderr().flush().ok();

                    #[cfg(unix)]
                    {
                        use std::os::unix::process::CommandExt;
                        let err = ssh_cmd.exec();
                        bail!("Failed to exec ssh: {}", err);
                    }

                    #[cfg(not(unix))]
                    {
                        let status = ssh_cmd
                            .stdin(std::process::Stdio::inherit())
                            .stdout(std::process::Stdio::inherit())
                            .stderr(std::process::Stdio::inherit())
                            .status()
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to execute ssh: {}. Is OpenSSH installed?",
                                    e
                                )
                            })?;
                        if !status.success() {
                            std::process::exit(status.code().unwrap_or(1));
                        }
                    }
                }
            }
            SshAction::Config { name, all } => {
                if name.is_none() && !all {
                    bail!("Specify a sandbox name or use --all");
                }

                let manager = VmManager::new()?;

                // Collect the sandbox names to generate config for
                let names: Vec<String> = if all {
                    manager
                        .list()
                        .into_iter()
                        .filter_map(|(n, _running, _backend)| {
                            let state = manager.get_sandbox_state(n)?;
                            if state.ssh_enabled {
                                Some(n.to_string())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    vec![name.unwrap()]
                };

                if names.is_empty() {
                    bail!("No SSH-enabled sandboxes found");
                }

                // Resolve home directory for cert/key paths
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

                println!("# Generated by agentkernel ssh-config");

                for sandbox_name in &names {
                    let state = match manager.get_sandbox_state(sandbox_name) {
                        Some(s) => s,
                        None => {
                            eprintln!("Warning: sandbox '{}' not found, skipping", sandbox_name);
                            continue;
                        }
                    };

                    if !state.ssh_enabled {
                        eprintln!("Warning: SSH not enabled on '{}', skipping", sandbox_name);
                        continue;
                    }

                    // Resolve host port (same logic as Ssh command)
                    let host_port = state.ssh_host_port.or_else(|| {
                        state
                            .ports
                            .iter()
                            .find(|p| p.container_port == 22)
                            .and_then(|p| p.host_port)
                    });

                    let host_port = match host_port {
                        Some(p) => p,
                        None => {
                            eprintln!("Warning: no SSH host port for '{}', skipping", sandbox_name);
                            continue;
                        }
                    };

                    let ssh_dir = home.join(".agentkernel").join("ssh").join(sandbox_name);

                    println!();
                    println!("Host agentkernel-{}", sandbox_name);
                    println!("    HostName localhost");
                    println!("    Port {}", host_port);
                    println!("    User sandbox");
                    println!("    IdentityFile {}", ssh_dir.join("client_key").display());
                    println!(
                        "    CertificateFile {}",
                        ssh_dir.join("client_key-cert.pub").display()
                    );
                    println!("    ProxyCommand agentkernel ssh-proxy {}", sandbox_name);
                    println!("    StrictHostKeyChecking no");
                    println!("    UserKnownHostsFile /dev/null");
                }
            }
            SshAction::Proxy { name } => {
                let manager = VmManager::new()?;

                // 1. Look up the sandbox — it must exist and be SSH-enabled
                let state = manager
                    .get_sandbox_state(&name)
                    .ok_or_else(|| anyhow::anyhow!("Sandbox '{}' not found", name))?;

                if !state.ssh_enabled {
                    bail!(
                        "SSH is not enabled on sandbox '{}'. \
                     Recreate it with --ssh to enable SSH access.",
                        name
                    );
                }

                // 2. Resolve host port (same logic as Ssh command)
                let host_port = state
                    .ssh_host_port
                    .or_else(|| {
                        state
                            .ports
                            .iter()
                            .find(|p| p.container_port == 22)
                            .and_then(|p| p.host_port)
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No SSH host port found for sandbox '{}'. \
                         The sandbox may need a port mapping for port 22.",
                            name
                        )
                    })?;

                // 3. Read the CA private key saved during sandbox creation
                let ca_key_path = manager.get_data_dir().join(format!("{}-ssh-ca.key", name));
                let ca_private_key = std::fs::read_to_string(&ca_key_path).map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to read CA key at {}: {}. \
                     Was the sandbox started with --ssh?",
                        ca_key_path.display(),
                        e
                    )
                })?;

                // 4. Generate an ephemeral client keypair
                let (client_private, client_public) = ssh::generate_client_keypair()?;

                // 5. Sign the client public key with the stored CA key
                let ttl_secs = ssh::parse_ttl_to_secs("30m")?;
                let cert = ssh::sign_client_key_local(
                    &ca_private_key,
                    &client_public,
                    &["sandbox"],
                    ttl_secs,
                )?;

                // 6. Write cert and key to a well-known location for the SSH config
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                let ssh_dir = home.join(".agentkernel").join("ssh").join(&name);
                std::fs::create_dir_all(&ssh_dir)?;

                let client_key_path = ssh_dir.join("client_key");
                let cert_path = ssh_dir.join("client_key-cert.pub");
                std::fs::write(&client_key_path, &client_private)?;
                std::fs::write(&cert_path, &cert)?;

                // Set permissions on the private key (owner read-only)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &client_key_path,
                        std::fs::Permissions::from_mode(0o600),
                    )?;
                }

                eprintln!(
                    "agentkernel ssh-proxy: signed cert for '{}', connecting to localhost:{}",
                    name, host_port
                );

                // 7. Raw TCP pipe: connect to localhost:{host_port} and pipe stdin/stdout
                let stream =
                    tokio::net::TcpStream::connect(format!("127.0.0.1:{}", host_port)).await?;
                let (mut rd, mut wr) = stream.into_split();
                let mut stdin = tokio::io::stdin();
                let mut stdout = tokio::io::stdout();

                tokio::select! {
                    result = tokio::io::copy(&mut stdin, &mut wr) => { result?; },
                    result = tokio::io::copy(&mut rd, &mut stdout) => { result?; },
                };
            }
        },
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
                        audit::AuditEvent::SshConnected {
                            sandbox,
                            host_port,
                            ssh_user,
                        } => (
                            "ssh_connected",
                            sandbox.as_str(),
                            format!("{}@localhost:{}", ssh_user, host_port),
                        ),
                        audit::AuditEvent::SshDisconnected {
                            sandbox,
                            duration_secs,
                            recording,
                        } => (
                            "ssh_disconnected",
                            sandbox.as_str(),
                            format!(
                                "duration={}s{}",
                                duration_secs,
                                recording
                                    .as_ref()
                                    .map(|r| format!(" recording={}", r))
                                    .unwrap_or_default()
                            ),
                        ),
                        audit::AuditEvent::SandboxError { name, error } => {
                            ("sandbox_error", name.as_str(), error.clone())
                        }
                        audit::AuditEvent::ScheduleTriggered {
                            schedule_name,
                            method,
                            ..
                        } => (
                            "schedule_triggered",
                            schedule_name.as_str(),
                            format!("method={}", method),
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
        Commands::Receipt { action } => match action {
            ReceiptAction::Verify {
                file,
                allow_unsigned,
            } => {
                let rec = receipt::verify_receipt_file(&file, allow_unsigned)?;
                println!("Receipt verified.");
                println!("  ID: {}", rec.receipt_id);
                println!("  Mode: {}", rec.invocation.mode_name());
                println!("  Exit code: {}", rec.outcome.exit_code);
                println!("  Output SHA-256: {}", rec.outcome.output_sha256);
                if let Some(sig) = rec.signature.as_ref() {
                    println!("  Signature: valid (ed25519, key {})", sig.key_id);
                } else {
                    println!("  Signature: none (legacy receipt accepted)");
                }
            }
            ReceiptAction::Replay {
                file,
                allow_unsigned,
            } => {
                let rec = receipt::verify_receipt_file(&file, allow_unsigned)?;
                let args = receipt::replay_args(&rec);
                if args.is_empty() {
                    bail!("Receipt does not contain a replayable invocation");
                }

                eprintln!(
                    "Replaying receipt {} ({})...",
                    rec.receipt_id,
                    rec.invocation.mode_name()
                );
                let exe = std::env::current_exe().context("Failed to locate current executable")?;
                let output = std::process::Command::new(exe)
                    .args(&args)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .context("Failed to replay receipt command")?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                print!("{}", stdout);
                if !stderr.is_empty() {
                    eprint!("{}", stderr);
                }

                let replay_hash = receipt::hash_output(&format!("{}{}", stdout, stderr));
                if replay_hash == rec.outcome.output_sha256 {
                    eprintln!("Replay output hash matches receipt.");
                } else {
                    eprintln!(
                        "Warning: replay output hash differs (expected {}, got {})",
                        rec.outcome.output_sha256, replay_hash
                    );
                }

                let replay_exit = output.status.code().unwrap_or(1);
                if replay_exit != rec.outcome.exit_code {
                    eprintln!(
                        "Warning: replay exit code differs (expected {}, got {})",
                        rec.outcome.exit_code, replay_exit
                    );
                }

                if !output.status.success() {
                    std::process::exit(replay_exit);
                }
            }
        },
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
        Commands::Benchmark {
            backends,
            iterations,
            warmup,
            image,
            json,
            output,
        } => {
            let backend_list = if let Some(ref b) = backends {
                benchmark::parse_backends(b)?
            } else {
                benchmark::available_backends()
            };
            if backend_list.is_empty() {
                bail!("No backends available to benchmark.");
            }
            let report =
                benchmark::run_benchmark(&backend_list, iterations, warmup, &image, !json).await?;
            benchmark::emit_report(&report, json, output.as_deref())?;
        }
        Commands::Parallel { job, backend } => {
            if job.is_empty() {
                bail!("At least one --job is required");
            }

            // Parse jobs: "name:image:command" or "name:image:tag:command"
            let mut parsed_jobs: Vec<(String, String, String)> = Vec::new();
            for j in &job {
                let parts: Vec<&str> = j.splitn(4, ':').collect();
                match parts.len() {
                    3 => {
                        // name:image:command (image without tag)
                        parsed_jobs.push((
                            parts[0].to_string(),
                            parts[1].to_string(),
                            parts[2].to_string(),
                        ));
                    }
                    4 => {
                        // name:image:tag:command (image with tag like alpine:3.24)
                        parsed_jobs.push((
                            parts[0].to_string(),
                            format!("{}:{}", parts[1], parts[2]),
                            parts[3].to_string(),
                        ));
                    }
                    _ => {
                        bail!(
                            "Invalid job format: '{}'. Expected 'name:image:command' or 'name:image:tag:command'",
                            j
                        );
                    }
                }
            }

            println!(
                "Running {} job{} in parallel...\n",
                parsed_jobs.len(),
                if parsed_jobs.len() != 1 { "s" } else { "" }
            );

            let backend_type = if let Some(ref b) = backend {
                Some(
                    b.parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?,
                )
            } else {
                None
            };

            let total_start = std::time::Instant::now();

            // Spawn all jobs concurrently
            let mut handles = Vec::new();
            for (name, image, command) in parsed_jobs {
                let bt = backend_type;
                handles.push(tokio::spawn(async move {
                    let mut mgr = VmManager::with_backend(bt)?;
                    let sandbox = format!(
                        "parallel-{}-{}",
                        name,
                        &uuid::Uuid::new_v4().to_string()[..6]
                    );
                    let cmd: Vec<String> = command.split_whitespace().map(String::from).collect();

                    let start = std::time::Instant::now();

                    // Try ephemeral first
                    let result = mgr
                        .run_ephemeral(&image, &cmd, &crate::permissions::Permissions::default())
                        .await;
                    let elapsed = start.elapsed();

                    match result {
                        Ok(output) => Ok::<_, anyhow::Error>((name, elapsed, true, output)),
                        Err(e) if e.to_string().contains("Ephemeral mode not supported") => {
                            // Fallback to full lifecycle
                            mgr.create(&sandbox, &image, 1, 512).await?;
                            mgr.start(&sandbox).await?;
                            let exec_result = mgr.exec_cmd(&sandbox, &cmd).await;
                            let _ = mgr.stop(&sandbox).await;
                            let _ = mgr.remove(&sandbox).await;
                            let elapsed = start.elapsed();
                            match exec_result {
                                Ok(output) => Ok((name, elapsed, true, output)),
                                Err(e) => Ok((name, elapsed, false, format!("Error: {}", e))),
                            }
                        }
                        Err(e) => Ok((name, elapsed, false, format!("Error: {}", e))),
                    }
                }));
            }

            // Collect results
            let mut all_passed = true;
            for handle in handles {
                match handle.await {
                    Ok(Ok((name, elapsed, success, _output))) => {
                        let status = if success { "done" } else { "FAILED" };
                        println!("  {} {} ({:.1}s)", name, status, elapsed.as_secs_f64());
                        if !success {
                            all_passed = false;
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("  Job error: {}", e);
                        all_passed = false;
                    }
                    Err(e) => {
                        eprintln!("  Job panicked: {}", e);
                        all_passed = false;
                    }
                }
            }

            let total_elapsed = total_start.elapsed();
            if all_passed {
                println!(
                    "\nAll jobs passed ({:.1}s wall time)",
                    total_elapsed.as_secs_f64()
                );
            } else {
                bail!(
                    "Some jobs failed ({:.1}s wall time)",
                    total_elapsed.as_secs_f64()
                );
            }
        }
        Commands::Images { action } => match action {
            ImagesAction::List { all } => {
                let imgs = images::list_images(all)?;
                if imgs.is_empty() {
                    if all {
                        println!("No Docker images found.");
                    } else {
                        println!("No agentkernel images found. Use --all to show all images.");
                    }
                } else {
                    println!(
                        "{:<40} {:<15} {:<12} {:>10}",
                        "REPOSITORY:TAG", "IMAGE ID", "USED BY", "SIZE"
                    );
                    let mut total_sandboxes = 0;
                    for img in &imgs {
                        let usage = images::sandbox_usage(&img.full_name()).unwrap_or(0);
                        total_sandboxes += usage;
                        let usage_str = if usage > 0 {
                            format!("{} sandbox{}", usage, if usage != 1 { "s" } else { "" })
                        } else {
                            "unused".to_string()
                        };
                        println!(
                            "{:<40} {:<15} {:<12} {:>10}",
                            img.full_name(),
                            img.image_id,
                            usage_str,
                            img.size
                        );
                    }
                    println!(
                        "\n{} image{}, {} sandbox reference{}",
                        imgs.len(),
                        if imgs.len() != 1 { "s" } else { "" },
                        total_sandboxes,
                        if total_sandboxes != 1 { "s" } else { "" }
                    );
                }
            }
            ImagesAction::Prune { agentkernel_only } => {
                println!("Pruning images...");
                let result = images::prune(agentkernel_only)?;
                println!("{}", result);
            }
            ImagesAction::Pull { image } => {
                println!("Pulling {}...", image);
                images::pull(&image)?;
                println!("Done.");
            }
            ImagesAction::LocalList => {
                let builder = image_builder::ImageBuilder::new()?;
                let images = builder.list();

                if images.is_empty() {
                    println!("No locally built images.");
                    println!("\nBuild one with: agentkernel build -t <name> <context>");
                } else {
                    println!("{:<20} {:<12} {:<24}", "NAME", "SIZE", "BUILT");
                    for img in images {
                        println!(
                            "{:<20} {:<12} {:<24}",
                            img.name,
                            img.format_size(),
                            &img.built_at[..19] // Trim to date/time only
                        );
                    }
                    println!("\nUse with: agentkernel create <sandbox> --image <name>");
                }
            }
            ImagesAction::LocalDelete { name } => {
                let mut builder = image_builder::ImageBuilder::new()?;
                builder.delete(&name)?;
                println!("Deleted locally built image '{}'", name);
            }
            ImagesAction::LocalSync => {
                let mut builder = image_builder::ImageBuilder::new()?;
                let removed = builder.sync()?;
                if removed.is_empty() {
                    println!("All local images are in sync.");
                } else {
                    println!("Removed {} stale image entries:", removed.len());
                    for name in &removed {
                        println!("  - {}", name);
                    }
                }
            }
        },
        Commands::Volume { action } => match action {
            VolumeAction::Create { slug, size } => {
                let mut manager = volume::VolumeManager::new()?;

                let size_bytes = if let Some(ref s) = size {
                    Some(volume::parse_size(s)?)
                } else {
                    None
                };

                let vol = manager.create(&slug, size_bytes)?;
                println!("Created volume '{}'", vol.slug);
                if vol.size_bytes > 0 {
                    println!(
                        "  Size limit: {}",
                        volume::Volume::format_size(vol.size_bytes)
                    );
                }
                println!("  Path: {}", manager.volumes_dir().join(&slug).display());
            }
            VolumeAction::List => {
                let manager = volume::VolumeManager::new()?;
                let volumes = manager.list();

                if volumes.is_empty() {
                    println!("No volumes found.");
                    println!("\nCreate one with: agentkernel volume create <slug>");
                } else {
                    println!(
                        "{:<20} {:<12} {:<12} {:<10}",
                        "SLUG", "SIZE", "USAGE", "MOUNTS"
                    );
                    for vol in volumes {
                        let usage = vol.disk_usage(manager.volumes_dir()).unwrap_or(0);
                        println!(
                            "{:<20} {:<12} {:<12} {:<10}",
                            vol.slug,
                            volume::Volume::format_size(vol.size_bytes),
                            volume::Volume::format_size(usage),
                            vol.mount_count
                        );
                    }
                }
            }
            VolumeAction::Info { slug } => {
                let manager = volume::VolumeManager::new()?;
                let vol = manager
                    .get(&slug)
                    .ok_or_else(|| anyhow::anyhow!("Volume '{}' not found", slug))?;

                let usage = vol.disk_usage(manager.volumes_dir()).unwrap_or(0);
                println!("Volume: {}", vol.slug);
                println!(
                    "  Size limit:  {}",
                    volume::Volume::format_size(vol.size_bytes)
                );
                println!("  Disk usage:  {}", volume::Volume::format_size(usage));
                println!("  Mount count: {}", vol.mount_count);
                println!("  Created:     {}", vol.created_at);
                if let Some(ref last) = vol.last_used {
                    println!("  Last used:   {}", last);
                }
                println!(
                    "  Path:        {}",
                    manager.volumes_dir().join(&slug).display()
                );
            }
            VolumeAction::Delete { slug, force } => {
                let mut manager = volume::VolumeManager::new()?;

                if !force {
                    let vol = manager
                        .get(&slug)
                        .ok_or_else(|| anyhow::anyhow!("Volume '{}' not found", slug))?;
                    let usage = vol.disk_usage(manager.volumes_dir()).unwrap_or(0);
                    if usage > 0 {
                        eprintln!(
                            "Warning: Volume '{}' contains {} of data.",
                            slug,
                            volume::Volume::format_size(usage)
                        );
                        eprintln!("Use --force to delete anyway.");
                        bail!("Volume not empty. Use --force to delete.");
                    }
                }

                manager.delete(&slug)?;
                println!("Deleted volume '{}'", slug);
            }
        },
        Commands::Build {
            name,
            context,
            dockerfile,
        } => {
            let mut builder = image_builder::ImageBuilder::new()?;

            let df_path = dockerfile.as_deref();
            let image = builder.build(&name, &context, df_path)?;

            println!("\nImage '{}' ready.", name);
            println!("  Docker tag: {}", image.docker_ref());
            println!("  Size: {}", image.format_size());
            println!(
                "\nUse it with: agentkernel create my-sandbox --image {}",
                name
            );
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
                let docker_image = image.unwrap_or_else(|| "alpine:3.24".to_string());

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
                    remote_id: state.remote_id.clone(),
                    remote_namespace: state.remote_namespace.clone(),
                    remote_metadata: state.remote_metadata.clone(),
                    workspace_revision: state.workspace_revision.clone(),
                    work_dir: state.work_dir.clone(),
                    config_path: state.config_path.clone(),
                };

                println!("Saving session '{}'...", name);
                snapshot::take(&sess.sandbox, &snap_name, &input)?;
                session::mark_saved(&name, &snap_name)?;
                println!("Session '{}' saved (snapshot: {})", name, snap_name);
            }
            SessionAction::Resume { name } => {
                let sess = session::get(&name)?
                    .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", name))?;

                let snapshot_backend = if sess.status == session::SessionStatus::Saved {
                    if let Some(ref snap_name) = sess.snapshot {
                        snapshot::get(snap_name)?.and_then(|meta| {
                            meta.backend.parse::<crate::backend::BackendType>().ok()
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                let mut manager = VmManager::with_backend(snapshot_backend)?;

                if sess.status == session::SessionStatus::Saved {
                    // Restore from snapshot
                    if let Some(ref snap_name) = sess.snapshot {
                        let meta = snapshot::get(snap_name)?
                            .ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", snap_name))?;
                        if !manager.exists(&sess.sandbox) {
                            println!("Restoring from snapshot '{}'...", snap_name);
                            if let Ok(snapshot_backend) =
                                meta.backend.parse::<crate::backend::BackendType>()
                            {
                                manager
                                    .create_with_backend(
                                        snapshot_backend,
                                        &sess.sandbox,
                                        meta.restore_image(),
                                        meta.vcpus,
                                        meta.memory_mb,
                                    )
                                    .await?;
                            } else {
                                manager
                                    .create(
                                        &sess.sandbox,
                                        meta.restore_image(),
                                        meta.vcpus,
                                        meta.memory_mb,
                                    )
                                    .await?;
                            }
                            manager.set_work_dir(&sess.sandbox, meta.work_dir.clone())?;
                            manager.set_config_path(&sess.sandbox, meta.config_path.clone())?;
                            if let Some(snapshot_handle) = meta.remote_snapshot.as_deref() {
                                manager
                                    .set_remote_restore_snapshot(&sess.sandbox, snapshot_handle)?;
                            }
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
                    remote_id: state.remote_id.clone(),
                    remote_namespace: state.remote_namespace.clone(),
                    remote_metadata: state.remote_metadata.clone(),
                    workspace_revision: state.workspace_revision.clone(),
                    work_dir: state.work_dir.clone(),
                    config_path: state.config_path.clone(),
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
            SnapshotAction::Restore {
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

                if let (Some(snapshot_handle), Some(explicit_backend)) =
                    (meta.remote_snapshot.as_deref(), backend_type)
                {
                    let snapshot_backend = meta
                        .backend
                        .parse::<crate::backend::BackendType>()
                        .map_err(|e| anyhow::anyhow!(e))?;
                    if explicit_backend != snapshot_backend {
                        anyhow::bail!(
                            "Remote snapshot '{}' was created on backend '{}'; restoring to '{}' is not supported",
                            snapshot_handle,
                            snapshot_backend,
                            explicit_backend
                        );
                    }
                }

                println!(
                    "Restoring snapshot '{}' as sandbox '{}'...",
                    name, restore_name
                );
                if let Ok(snapshot_backend) = meta.backend.parse::<crate::backend::BackendType>() {
                    manager
                        .create_with_backend(
                            snapshot_backend,
                            &restore_name,
                            meta.restore_image(),
                            meta.vcpus,
                            meta.memory_mb,
                        )
                        .await?;
                } else {
                    manager
                        .create(
                            &restore_name,
                            meta.restore_image(),
                            meta.vcpus,
                            meta.memory_mb,
                        )
                        .await?;
                }
                manager.set_work_dir(&restore_name, meta.work_dir.clone())?;
                manager.set_config_path(&restore_name, meta.config_path.clone())?;
                if let Some(snapshot_handle) = meta.remote_snapshot.as_deref() {
                    manager.set_remote_restore_snapshot(&restore_name, snapshot_handle)?;
                }

                println!("Sandbox '{}' restored from snapshot.", restore_name);
                println!("\nNext steps:");
                println!("  agentkernel start {}", restore_name);
                println!("  agentkernel attach {}", restore_name);
            }
        },
        Commands::Llm { action } => match action {
            LlmAction::Keys { action } => match action {
                LlmKeysAction::List => {
                    let keys_path = llm_keys_path();
                    if keys_path.exists() {
                        let content = std::fs::read_to_string(&keys_path)?;
                        let keys: std::collections::BTreeMap<String, String> =
                            serde_json::from_str(&content)?;
                        if keys.is_empty() {
                            println!("No LLM keys configured.");
                        } else {
                            println!("{:<30} VAULT KEY", "DOMAIN");
                            for (domain, vault_key) in &keys {
                                println!("{:<30} {}", domain, vault_key);
                            }
                        }
                    } else {
                        println!("No LLM keys configured.");
                    }
                }
                LlmKeysAction::Set { provider, key } => {
                    let domain = provider_to_domain(&provider);
                    let vault_key = key.unwrap_or_else(|| {
                        format!(
                            "{}_API_KEY",
                            provider.to_uppercase().replace(['.', '-'], "_")
                        )
                    });

                    // Read secret from stdin
                    eprintln!(
                        "Enter the API key value (or set it separately with `agentkernel secret set {}`):",
                        vault_key
                    );
                    let mut value = String::new();
                    std::io::stdin().read_line(&mut value)?;
                    let value = value.trim();
                    if !value.is_empty() {
                        let vault = crate::secrets::SecretVault::new(
                            crate::secrets::SecretBackend::default(),
                        );
                        vault.set(&vault_key, value)?;
                        eprintln!("Secret '{}' stored in vault.", vault_key);
                    }

                    // Save domain -> vault_key mapping
                    let keys_path = llm_keys_path();
                    let mut keys: std::collections::BTreeMap<String, String> = if keys_path.exists()
                    {
                        serde_json::from_str(&std::fs::read_to_string(&keys_path)?)?
                    } else {
                        std::collections::BTreeMap::new()
                    };
                    keys.insert(domain.clone(), vault_key.clone());
                    crate::secure_fs::write_private_json(&keys_path, &keys)?;
                    println!("LLM key mapping: {} -> {}", domain, vault_key);
                }
                LlmKeysAction::Remove { provider } => {
                    let domain = provider_to_domain(&provider);
                    let keys_path = llm_keys_path();
                    if keys_path.exists() {
                        let mut keys: std::collections::BTreeMap<String, String> =
                            serde_json::from_str(&std::fs::read_to_string(&keys_path)?)?;
                        if keys.remove(&domain).is_some() {
                            crate::secure_fs::write_private_json(&keys_path, &keys)?;
                            println!("Removed LLM key mapping for {}", domain);
                        } else {
                            println!("No LLM key mapping found for {}", domain);
                        }
                    } else {
                        println!("No LLM keys configured.");
                    }
                }
            },
        },
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

fn llm_keys_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agentkernel")
        .join("llm_keys.json")
}

fn provider_to_domain(provider: &str) -> String {
    match provider {
        "openai" => "api.openai.com".to_string(),
        "anthropic" => "api.anthropic.com".to_string(),
        "google" | "gemini" => "generativelanguage.googleapis.com".to_string(),
        "deepseek" => "api.deepseek.com".to_string(),
        "groq" => "api.groq.com".to_string(),
        "mistral" => "api.mistral.ai".to_string(),
        "cohere" => "api.cohere.com".to_string(),
        "together" => "api.together.xyz".to_string(),
        "fireworks" => "api.fireworks.ai".to_string(),
        other => other.to_string(), // Assume it's a raw domain
    }
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

    // -- Firecracker daemon --
    println!("\nFirecracker Daemon:");
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
    } else if status.kvm_available {
        println!("  Status .............. not running");
    } else {
        println!(
            "  Status .............. not applicable (requires Linux KVM; use a container backend instead)"
        );
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
                "portmap" => policy::Action::PortMap,
                "ssh" => policy::Action::SSH,
                other => bail!(
                    "Invalid action '{}'. Use: run, exec, create, attach, mount, network, portmap, ssh",
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
    if running && let Some(ip) = manager.get_container_ip(name) {
        println!("IP:             {}", ip);
    }
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
    if !state.ports.is_empty() {
        let ports_str = state
            .ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("Ports:          {}", ports_str);
    }
    if !state.endpoints.is_empty() {
        let endpoints_str = state
            .endpoints
            .iter()
            .map(|endpoint| format!("{} -> {}", endpoint.container_port, endpoint.url))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Endpoints:      {}", endpoints_str);
    }
    if let Some(ref revision) = state.workspace_revision {
        println!("Workspace Rev:  {}", revision);
    }
    if !state.labels.is_empty() {
        let labels_str = state
            .labels
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Labels:         {}", labels_str);
    }
    if let Some(ref desc) = state.description {
        println!("Description:    {}", desc);
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
                audit::AuditEvent::SshConnected {
                    host_port,
                    ssh_user,
                    ..
                } => {
                    format!("ssh     {}@localhost:{}", ssh_user, host_port)
                }
                audit::AuditEvent::SshDisconnected { duration_secs, .. } => {
                    format!("ssh-end {}s", duration_secs)
                }
                audit::AuditEvent::SandboxError { error, .. } => {
                    format!("error   {}", error)
                }
                audit::AuditEvent::ScheduleTriggered {
                    schedule_name,
                    method,
                    ..
                } => {
                    format!("sched   {}:{}", schedule_name, method)
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

fn resolve_workspace_root(config_base_dir: Option<&Path>, dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = dir {
        let workspace = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()?.join(dir)
        };
        if !workspace.exists() {
            bail!(
                "Workspace directory '{}' does not exist",
                workspace.display()
            );
        }
        if !workspace.is_dir() {
            bail!(
                "Workspace path '{}' is not a directory",
                workspace.display()
            );
        }
        return Ok(workspace.canonicalize().unwrap_or(workspace));
    }

    let workspace = if let Some(config_base_dir) = config_base_dir {
        if config_base_dir.is_absolute() {
            config_base_dir.to_path_buf()
        } else {
            std::env::current_dir()?.join(config_base_dir)
        }
    } else {
        std::env::current_dir()?
    };

    Ok(workspace.canonicalize().unwrap_or(workspace))
}

/// Decide whether `run` should build a project image before execution.
///
/// Explicit images always win. Config and template runs preserve their
/// Dockerfile behavior, while command-driven runs require an explicit
/// `--build` before an ambient Dockerfile is considered.
fn should_build_run_image(
    explicit_image: bool,
    build_requested: bool,
    honor_config_dockerfile: bool,
    config: Option<&Config>,
    current_dir: &Path,
) -> bool {
    if explicit_image {
        return false;
    }

    let dockerfile_available = config
        .map(|config| config.requires_build(current_dir))
        .unwrap_or_else(|| languages::detect_dockerfile(current_dir).is_some());

    (build_requested || honor_config_dockerfile) && dockerfile_available
}

/// Whether an implicitly loaded project config contains intentional build
/// settings, rather than merely sharing a directory with a Dockerfile.
fn config_has_build_settings(config: &Config) -> bool {
    config.build.dockerfile.is_some()
        || config.build.context.is_some()
        || config.build.target.is_some()
        || !config.build.args.is_empty()
        || config.build.no_cache
}

fn extract_template_help_text(content: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(content).ok()?;
    value
        .get("template")
        .and_then(|v| v.get("help_text"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
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

/// Check if the HTTP API server is running on the given address.
/// Returns true if a TCP connection succeeds within a short timeout.
async fn try_server_health(host: &str, port: u16) -> bool {
    let addr = format!("{}:{}", host, port);
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            tokio::net::TcpStream::connect(&addr),
        )
        .await,
        Ok(Ok(_))
    )
}

/// Delegate sandbox start to the running HTTP server.
/// Sends POST /sandboxes/{name}/start and checks for a 200 response.
async fn delegate_start_to_server(host: &str, port: u16, name: &str) -> Result<()> {
    use http_body_util::{BodyExt, Empty};
    use hyper::Request;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    let uri: hyper::Uri = format!("http://{}:{}/sandboxes/{}/start", host, port, name)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid URI: {}", e))?;

    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<bytes::Bytes>>();

    let req = Request::builder()
        .method(hyper::Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Empty::<bytes::Bytes>::new())
        .map_err(|e| anyhow::anyhow!("failed to build request: {}", e))?;

    let resp = tokio::time::timeout(std::time::Duration::from_secs(30), client.request(req))
        .await
        .map_err(|_| anyhow::anyhow!("timeout waiting for server to start sandbox"))?
        .map_err(|e| anyhow::anyhow!("HTTP request failed: {}", e))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp
            .into_body()
            .collect()
            .await
            .map(|c| String::from_utf8_lossy(&c.to_bytes()).to_string())
            .unwrap_or_default();
        bail!("Server returned {} when starting sandbox: {}", status, body);
    }
}

#[cfg(test)]
mod tests {
    use super::{config_has_build_settings, resolve_workspace_root, should_build_run_image};
    use crate::config::Config;
    use tempfile::TempDir;

    fn project_with_dockerfile() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("Dockerfile"), "FROM alpine:3.24\n").unwrap();
        temp_dir
    }

    #[test]
    fn resolve_workspace_root_prefers_explicit_dir() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let resolved = resolve_workspace_root(None, Some(&workspace)).unwrap();
        let expected = workspace.canonicalize().unwrap_or(workspace);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_workspace_root_uses_config_base_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();

        let resolved = resolve_workspace_root(Some(&config_dir), None).unwrap();
        let expected = config_dir.canonicalize().unwrap_or(config_dir);
        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_workspace_root_rejects_missing_explicit_dir() {
        let temp_dir = TempDir::new().unwrap();
        let missing = temp_dir.path().join("missing");

        let error = resolve_workspace_root(None, Some(&missing)).unwrap_err();
        assert!(error.to_string().contains("Workspace directory"));
    }

    #[test]
    fn command_run_skips_ambient_dockerfile() {
        let project = project_with_dockerfile();

        assert!(!should_build_run_image(
            false,
            false,
            false,
            None,
            project.path()
        ));
    }

    #[test]
    fn build_flag_enables_ambient_dockerfile() {
        let project = project_with_dockerfile();

        assert!(should_build_run_image(
            false,
            true,
            false,
            None,
            project.path()
        ));
    }

    #[test]
    fn explicitly_selected_config_preserves_dockerfile_build() {
        let project = project_with_dockerfile();
        let config = Config::minimal("test-project", "claude");

        assert!(should_build_run_image(
            false,
            false,
            true,
            Some(&config),
            project.path()
        ));
    }

    #[test]
    fn implicitly_loaded_config_skips_ambient_dockerfile() {
        let project = project_with_dockerfile();
        let config = Config::minimal("test-project", "claude");

        assert!(!config_has_build_settings(&config));
        assert!(!should_build_run_image(
            false,
            false,
            config_has_build_settings(&config),
            Some(&config),
            project.path()
        ));
    }

    #[test]
    fn implicit_config_with_build_settings_preserves_dockerfile_build() {
        let project = project_with_dockerfile();
        let mut config = Config::minimal("test-project", "claude");
        config.build.context = Some(".".to_string());

        assert!(config_has_build_settings(&config));
        assert!(should_build_run_image(
            false,
            false,
            config_has_build_settings(&config),
            Some(&config),
            project.path()
        ));
    }

    #[test]
    fn explicit_image_bypasses_dockerfile_build() {
        let project = project_with_dockerfile();
        let config = Config::minimal("test-project", "claude");

        assert!(!should_build_run_image(
            true,
            true,
            true,
            Some(&config),
            project.path()
        ));
    }
}
