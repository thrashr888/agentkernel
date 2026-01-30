//! Plugin installer for agent integrations.
//!
//! Embeds plugin files in the binary and writes them to the project
//! or user home directory. This enables Homebrew users who only have
//! the binary to install agent plugins without cloning the repo.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

// --- Embedded plugin files ---

const CLAUDE_SKILL_MD: &str = include_str!("../claude-plugin/.claude/skills/agentkernel/SKILL.md");
const CLAUDE_COMMAND_MD: &str = include_str!("../claude-plugin/.claude/commands/sandbox.md");
const CODEX_MCP_JSON: &str = include_str!("../plugins/codex/mcp.json");
const GEMINI_MCP_JSON: &str = include_str!("../plugins/gemini/mcp.json");
const MCP_GENERIC_JSON: &str = include_str!("../plugins/mcp/mcp.json");
const OPENCODE_PACKAGE_JSON: &str = include_str!("../plugins/opencode/.opencode/package.json");
const OPENCODE_PLUGIN_TS: &str =
    include_str!("../plugins/opencode/.opencode/plugins/agentkernel.ts");

/// Plugin targets that can be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTarget {
    Claude,
    Codex,
    Gemini,
    OpenCode,
    Mcp,
}

impl PluginTarget {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" | "gemini-cli" => Some(Self::Gemini),
            "opencode" | "open-code" => Some(Self::OpenCode),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Mcp => "mcp",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code skill + MCP server config",
            Self::Codex => "Codex MCP server config",
            Self::Gemini => "Gemini CLI MCP server config",
            Self::OpenCode => "OpenCode TypeScript plugin",
            Self::Mcp => "Generic MCP server config",
        }
    }

    pub fn all() -> &'static [PluginTarget] {
        &[
            Self::Claude,
            Self::Codex,
            Self::Gemini,
            Self::OpenCode,
            Self::Mcp,
        ]
    }

    fn supports_global(&self) -> bool {
        matches!(self, Self::Claude | Self::Codex | Self::Gemini | Self::Mcp)
    }
}

/// How to handle writing a file.
#[derive(Debug, Clone, Copy)]
enum WriteStrategy {
    /// Create the file; skip if identical, warn if different.
    Create,
    /// Merge agentkernel entry into existing JSON mcpServers object.
    MergeJsonMcpServer,
}

/// A single file to be installed.
struct PluginFile {
    rel_path: &'static str,
    content: &'static str,
    strategy: WriteStrategy,
}

/// Options controlling installation behavior.
pub struct InstallOptions {
    pub global: bool,
    pub force: bool,
    pub dry_run: bool,
}

/// Result of installing a single file.
enum InstallResult {
    Created(PathBuf),
    Updated(PathBuf),
    Skipped(PathBuf, &'static str),
    Error(PathBuf, String),
}

/// Get the plugin files for a target.
fn plugin_files(target: PluginTarget) -> Vec<PluginFile> {
    match target {
        PluginTarget::Claude => vec![
            PluginFile {
                rel_path: ".claude/skills/agentkernel/SKILL.md",
                content: CLAUDE_SKILL_MD,
                strategy: WriteStrategy::Create,
            },
            PluginFile {
                rel_path: ".claude/commands/sandbox.md",
                content: CLAUDE_COMMAND_MD,
                strategy: WriteStrategy::Create,
            },
            PluginFile {
                rel_path: ".mcp.json",
                content: MCP_GENERIC_JSON,
                strategy: WriteStrategy::MergeJsonMcpServer,
            },
        ],
        PluginTarget::Codex => vec![PluginFile {
            rel_path: ".mcp.json",
            content: CODEX_MCP_JSON,
            strategy: WriteStrategy::MergeJsonMcpServer,
        }],
        PluginTarget::Gemini => vec![PluginFile {
            rel_path: ".gemini/settings.json",
            content: GEMINI_MCP_JSON,
            strategy: WriteStrategy::MergeJsonMcpServer,
        }],
        PluginTarget::OpenCode => vec![
            PluginFile {
                rel_path: ".opencode/package.json",
                content: OPENCODE_PACKAGE_JSON,
                strategy: WriteStrategy::Create,
            },
            PluginFile {
                rel_path: ".opencode/plugins/agentkernel.ts",
                content: OPENCODE_PLUGIN_TS,
                strategy: WriteStrategy::Create,
            },
        ],
        PluginTarget::Mcp => vec![PluginFile {
            rel_path: ".mcp.json",
            content: MCP_GENERIC_JSON,
            strategy: WriteStrategy::MergeJsonMcpServer,
        }],
    }
}

/// Install plugin files for a target.
pub fn install_plugin(target: PluginTarget, opts: &InstallOptions) -> Result<()> {
    let root = if opts.global {
        if !target.supports_global() {
            bail!(
                "{} plugins are per-project only. Run from your project directory without --global.",
                target.name()
            );
        }
        global_root(target)?
    } else {
        std::env::current_dir()?
    };

    let files = plugin_files(target);
    let mut results = Vec::new();
    let mut has_error = false;

    if opts.dry_run {
        println!("Installing {} plugin... (dry run)\n", target.name());
    } else {
        println!("Installing {} plugin...\n", target.name());
    }

    for file in &files {
        let dest = root.join(file.rel_path);
        let result = match file.strategy {
            WriteStrategy::Create => install_create(&dest, file.content, opts),
            WriteStrategy::MergeJsonMcpServer => install_merge_mcp(&dest, file.content, opts),
        };
        if matches!(result, InstallResult::Error(..)) {
            has_error = true;
        }
        results.push(result);
    }

    print_results(&results);
    print_next_steps(target);

    if has_error {
        bail!("Some files failed to install");
    }
    Ok(())
}

/// List available plugins and their install status.
pub fn list_plugins() {
    println!("{:<12} {:<35} STATUS", "TARGET", "DESCRIPTION");
    println!("{:-<62}", "");

    let cwd = std::env::current_dir().unwrap_or_default();

    for target in PluginTarget::all() {
        let installed = check_installed(*target, &cwd);
        let status = if installed {
            "installed"
        } else {
            "not installed"
        };

        println!(
            "{:<12} {:<35} {}",
            target.name(),
            target.description(),
            status
        );
    }
}

/// Determine root directory for global installation.
fn global_root(target: PluginTarget) -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    match target {
        PluginTarget::Claude | PluginTarget::Codex | PluginTarget::Gemini | PluginTarget::Mcp => {
            Ok(home)
        }
        _ => bail!("{} plugins are per-project only", target.name()),
    }
}

/// Create a file, optionally overwriting.
fn install_create(dest: &Path, content: &str, opts: &InstallOptions) -> InstallResult {
    if dest.exists() {
        match std::fs::read_to_string(dest) {
            Ok(existing) if existing == content => {
                return InstallResult::Skipped(dest.to_path_buf(), "already up to date");
            }
            Ok(_) if !opts.force => {
                return InstallResult::Skipped(
                    dest.to_path_buf(),
                    "already exists (use --force to overwrite)",
                );
            }
            _ => {} // force mode or read error, proceed
        }
    }

    if opts.dry_run {
        if dest.exists() {
            return InstallResult::Updated(dest.to_path_buf());
        }
        return InstallResult::Created(dest.to_path_buf());
    }

    if let Some(parent) = dest.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return InstallResult::Error(
            dest.to_path_buf(),
            format!("Cannot create directory: {}", e),
        );
    }

    match std::fs::write(dest, content) {
        Ok(()) => InstallResult::Created(dest.to_path_buf()),
        Err(e) => InstallResult::Error(dest.to_path_buf(), e.to_string()),
    }
}

/// Merge agentkernel MCP server entry into an existing JSON file.
fn install_merge_mcp(dest: &Path, mcp_content: &str, opts: &InstallOptions) -> InstallResult {
    let mcp_value: serde_json::Value = match serde_json::from_str(mcp_content) {
        Ok(v) => v,
        Err(e) => {
            return InstallResult::Error(
                dest.to_path_buf(),
                format!("Invalid embedded MCP config: {}", e),
            );
        }
    };

    let ak_server = &mcp_value["mcpServers"]["agentkernel"];

    if !dest.exists() {
        if opts.dry_run {
            return InstallResult::Created(dest.to_path_buf());
        }

        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let pretty = serde_json::to_string_pretty(&mcp_value).unwrap();
        return match std::fs::write(dest, pretty) {
            Ok(()) => InstallResult::Created(dest.to_path_buf()),
            Err(e) => InstallResult::Error(dest.to_path_buf(), e.to_string()),
        };
    }

    // File exists: read, parse, merge
    let existing_str = match std::fs::read_to_string(dest) {
        Ok(s) => s,
        Err(e) => return InstallResult::Error(dest.to_path_buf(), format!("Cannot read: {}", e)),
    };

    let mut existing: serde_json::Value = match serde_json::from_str(&existing_str) {
        Ok(v) => v,
        Err(e) => return InstallResult::Error(dest.to_path_buf(), format!("Invalid JSON: {}", e)),
    };

    // Check if mcpServers.agentkernel already exists
    if let Some(servers) = existing.get("mcpServers").and_then(|s| s.as_object())
        && let Some(existing_ak) = servers.get("agentkernel")
    {
        if existing_ak == ak_server {
            return InstallResult::Skipped(dest.to_path_buf(), "already configured");
        }
        if !opts.force {
            return InstallResult::Skipped(
                dest.to_path_buf(),
                "agentkernel entry exists with different config (use --force)",
            );
        }
    }

    if opts.dry_run {
        return InstallResult::Updated(dest.to_path_buf());
    }

    // Merge: ensure mcpServers object exists, then insert agentkernel
    let obj = existing.as_object_mut().unwrap();
    if !obj.contains_key("mcpServers") {
        obj.insert("mcpServers".to_string(), serde_json::json!({}));
    }
    let servers = obj.get_mut("mcpServers").unwrap().as_object_mut().unwrap();
    servers.insert("agentkernel".to_string(), ak_server.clone());

    let pretty = serde_json::to_string_pretty(&existing).unwrap();
    match std::fs::write(dest, pretty) {
        Ok(()) => InstallResult::Updated(dest.to_path_buf()),
        Err(e) => InstallResult::Error(dest.to_path_buf(), e.to_string()),
    }
}

/// Check if a plugin is installed in the given directory.
fn check_installed(target: PluginTarget, cwd: &Path) -> bool {
    let files = plugin_files(target);
    files.iter().all(|f| {
        let path = cwd.join(f.rel_path);
        match f.strategy {
            WriteStrategy::Create => path.exists(),
            WriteStrategy::MergeJsonMcpServer => {
                // Check that the file exists AND contains the agentkernel entry
                if let Ok(content) = std::fs::read_to_string(&path)
                    && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
                {
                    return json
                        .get("mcpServers")
                        .and_then(|s| s.get("agentkernel"))
                        .is_some();
                }
                false
            }
        }
    })
}

/// Print file-level results.
fn print_results(results: &[InstallResult]) {
    for result in results {
        match result {
            InstallResult::Created(path) => println!("  + {}", path.display()),
            InstallResult::Updated(path) => println!("  ~ {} (merged)", path.display()),
            InstallResult::Skipped(path, reason) => {
                println!("  - {} ({})", path.display(), reason)
            }
            InstallResult::Error(path, err) => {
                eprintln!("  ! {} ERROR: {}", path.display(), err)
            }
        }
    }
}

/// Print next-step guidance after installation.
fn print_next_steps(target: PluginTarget) {
    println!();
    match target {
        PluginTarget::Claude => {
            println!("Claude Code plugin installed.");
            println!("  Use /sandbox to run commands in isolated sandboxes.");
        }
        PluginTarget::Codex => {
            println!("Codex MCP config written.");
            println!("  The agentkernel MCP server will be available in Codex.");
        }
        PluginTarget::Gemini => {
            println!("Gemini CLI MCP config written.");
            println!("  Restart Gemini CLI to pick up the new MCP server.");
        }
        PluginTarget::OpenCode => {
            println!("OpenCode plugin installed.");
            println!("  Start the agentkernel server first: agentkernel serve");
            println!("  Then launch OpenCode -- the plugin loads automatically.");
        }
        PluginTarget::Mcp => {
            println!("Generic MCP config written.");
            println!("  Any MCP-compatible agent can now use the agentkernel server.");
        }
    }
}

/// Check if a CLI command exists in PATH.
fn command_in_path(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Agent CLI commands mapped to plugin targets.
const AGENT_COMMANDS: &[(&str, PluginTarget)] = &[
    ("claude", PluginTarget::Claude),
    ("codex", PluginTarget::Codex),
    ("gemini", PluginTarget::Gemini),
    ("opencode", PluginTarget::OpenCode),
];

/// Detect agents whose CLI is installed but whose plugin is missing.
/// Returns targets that should be offered for installation.
pub fn detect_uninstalled_plugins() -> Vec<PluginTarget> {
    let cwd = std::env::current_dir().unwrap_or_default();
    AGENT_COMMANDS
        .iter()
        .filter_map(|(cmd, target)| {
            if command_in_path(cmd) && !check_installed(*target, &cwd) {
                Some(*target)
            } else {
                None
            }
        })
        .collect()
}

/// Install plugins for the given targets (used by setup flow).
pub fn install_detected_plugins(targets: &[PluginTarget]) -> Result<()> {
    let opts = InstallOptions {
        global: false,
        force: false,
        dry_run: false,
    };
    for target in targets {
        install_plugin(*target, &opts)?;
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_target_from_str() {
        assert_eq!(PluginTarget::from_str("claude"), Some(PluginTarget::Claude));
        assert_eq!(
            PluginTarget::from_str("claude-code"),
            Some(PluginTarget::Claude)
        );
        assert_eq!(PluginTarget::from_str("codex"), Some(PluginTarget::Codex));
        assert_eq!(PluginTarget::from_str("gemini"), Some(PluginTarget::Gemini));
        assert_eq!(
            PluginTarget::from_str("gemini-cli"),
            Some(PluginTarget::Gemini)
        );
        assert_eq!(
            PluginTarget::from_str("opencode"),
            Some(PluginTarget::OpenCode)
        );
        assert_eq!(
            PluginTarget::from_str("open-code"),
            Some(PluginTarget::OpenCode)
        );
        assert_eq!(PluginTarget::from_str("mcp"), Some(PluginTarget::Mcp));
        assert_eq!(PluginTarget::from_str("unknown"), None);
        assert_eq!(PluginTarget::from_str("CLAUDE"), Some(PluginTarget::Claude));
    }

    #[test]
    fn test_plugin_target_all() {
        let all = PluginTarget::all();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], PluginTarget::Claude);
        assert_eq!(all[4], PluginTarget::Mcp);
    }

    #[test]
    fn test_embedded_json_files_parse() {
        let _: serde_json::Value =
            serde_json::from_str(CODEX_MCP_JSON).expect("Codex mcp.json should parse");
        let _: serde_json::Value =
            serde_json::from_str(GEMINI_MCP_JSON).expect("Gemini mcp.json should parse");
        let _: serde_json::Value =
            serde_json::from_str(MCP_GENERIC_JSON).expect("Generic mcp.json should parse");
        let _: serde_json::Value = serde_json::from_str(OPENCODE_PACKAGE_JSON)
            .expect("OpenCode package.json should parse");
    }

    #[test]
    fn test_embedded_files_not_empty() {
        assert!(!CLAUDE_SKILL_MD.is_empty());
        assert!(!CLAUDE_COMMAND_MD.is_empty());
        assert!(!CODEX_MCP_JSON.is_empty());
        assert!(!GEMINI_MCP_JSON.is_empty());
        assert!(!MCP_GENERIC_JSON.is_empty());
        assert!(!OPENCODE_PACKAGE_JSON.is_empty());
        assert!(!OPENCODE_PLUGIN_TS.is_empty());
    }

    #[test]
    fn test_create_new_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("test.json");
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_create(&dest, "test content", &opts);
        assert!(matches!(result, InstallResult::Created(_)));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "test content");
    }

    #[test]
    fn test_create_skips_identical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("test.json");
        std::fs::write(&dest, "test content").unwrap();
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_create(&dest, "test content", &opts);
        assert!(matches!(
            result,
            InstallResult::Skipped(_, "already up to date")
        ));
    }

    #[test]
    fn test_create_skips_different_without_force() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("test.json");
        std::fs::write(&dest, "old content").unwrap();
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_create(&dest, "new content", &opts);
        assert!(matches!(result, InstallResult::Skipped(_, _)));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old content");
    }

    #[test]
    fn test_create_overwrites_with_force() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("test.json");
        std::fs::write(&dest, "old content").unwrap();
        let opts = InstallOptions {
            global: false,
            force: true,
            dry_run: false,
        };
        let result = install_create(&dest, "new content", &opts);
        assert!(matches!(result, InstallResult::Created(_)));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new content");
    }

    #[test]
    fn test_create_creates_parent_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("a/b/c/test.json");
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_create(&dest, "nested content", &opts);
        assert!(matches!(result, InstallResult::Created(_)));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "nested content");
    }

    #[test]
    fn test_merge_into_new_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join(".mcp.json");
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_merge_mcp(&dest, CODEX_MCP_JSON, &opts);
        assert!(matches!(result, InstallResult::Created(_)));

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert!(content["mcpServers"]["agentkernel"].is_object());
    }

    #[test]
    fn test_merge_into_empty_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join(".mcp.json");
        std::fs::write(&dest, "{}").unwrap();
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_merge_mcp(&dest, CODEX_MCP_JSON, &opts);
        assert!(matches!(result, InstallResult::Updated(_)));

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert!(content["mcpServers"]["agentkernel"].is_object());
    }

    #[test]
    fn test_merge_preserves_existing_servers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join(".mcp.json");
        std::fs::write(
            &dest,
            r#"{"mcpServers":{"other-tool":{"command":"other","args":[]}}}"#,
        )
        .unwrap();
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_merge_mcp(&dest, CODEX_MCP_JSON, &opts);
        assert!(matches!(result, InstallResult::Updated(_)));

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert!(content["mcpServers"]["other-tool"].is_object());
        assert!(content["mcpServers"]["agentkernel"].is_object());
    }

    #[test]
    fn test_merge_skips_identical() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join(".mcp.json");
        // Write the same content that would be merged
        let pretty = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(CODEX_MCP_JSON).unwrap(),
        )
        .unwrap();
        std::fs::write(&dest, pretty).unwrap();
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: false,
        };
        let result = install_merge_mcp(&dest, CODEX_MCP_JSON, &opts);
        assert!(matches!(
            result,
            InstallResult::Skipped(_, "already configured")
        ));
    }

    #[test]
    fn test_dry_run_does_not_write() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("should-not-exist.json");
        let opts = InstallOptions {
            global: false,
            force: false,
            dry_run: true,
        };
        let result = install_create(&dest, "content", &opts);
        assert!(matches!(result, InstallResult::Created(_)));
        assert!(!dest.exists());
    }

    #[test]
    fn test_check_installed_false_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!check_installed(PluginTarget::Claude, tmp.path()));
    }

    #[test]
    fn test_check_installed_true_when_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Create all Claude plugin files at new paths
        std::fs::create_dir_all(tmp.path().join(".claude/skills/agentkernel")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude/commands")).unwrap();
        std::fs::write(tmp.path().join(".claude/skills/agentkernel/SKILL.md"), "").unwrap();
        std::fs::write(tmp.path().join(".claude/commands/sandbox.md"), "").unwrap();
        // Also need .mcp.json with agentkernel entry for check_installed
        std::fs::write(
            tmp.path().join(".mcp.json"),
            r#"{"mcpServers":{"agentkernel":{}}}"#,
        )
        .unwrap();
        assert!(check_installed(PluginTarget::Claude, tmp.path()));
    }

    #[test]
    fn test_global_supported_for_claude() {
        let result = global_root(PluginTarget::Claude);
        assert!(result.is_ok());
    }

    #[test]
    fn test_global_not_supported_for_opencode() {
        let result = global_root(PluginTarget::OpenCode);
        assert!(result.is_err());
    }
}
