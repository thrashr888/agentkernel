//! CLI integration tests for agentkernel
//!
//! These tests verify the CLI commands work correctly.
//! Run with: cargo test --test cli_test

use std::process::Command;

/// Get the path to the agentkernel binary
fn agentkernel_bin() -> String {
    // Use debug build for tests
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{}/target/debug/agentkernel", manifest_dir)
}

/// Run agentkernel with given args and return (exit_code, stdout, stderr)
fn run_cmd(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(agentkernel_bin())
        .args(args)
        .output()
        .expect("Failed to execute command");

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (exit_code, stdout, stderr)
}

// === Help and Version Tests ===

#[test]
fn test_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Run AI coding agents"));
    assert!(stdout.contains("COMMAND"));
    assert!(stdout.contains("create"));
    assert!(stdout.contains("start"));
    assert!(stdout.contains("stop"));
    assert!(stdout.contains("exec"));
}

#[test]
fn test_version() {
    let (exit_code, stdout, _stderr) = run_cmd(&["--version"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("agentkernel"));
}

#[test]
fn test_help_subcommand() {
    let (exit_code, stdout, _stderr) = run_cmd(&["help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Run AI coding agents"));
}

// === Subcommand Help Tests ===

#[test]
fn test_create_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["create", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Create a new sandbox"));
    assert!(stdout.contains("NAME")); // positional argument
    assert!(stdout.contains("--agent")); // agent option
    assert!(stdout.contains("--backend")); // backend option
}

#[test]
fn test_start_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["start", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Start a sandbox"));
}

#[test]
fn test_stop_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["stop", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Stop a running sandbox"));
}

#[test]
fn test_exec_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["exec", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Execute a command"));
}

#[test]
fn test_run_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["run", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Run a command in a temporary sandbox"));
}

#[test]
fn test_list_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["list", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("List all sandboxes"));
}

#[test]
fn test_agents_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["agents", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("List supported AI agents"));
}

// === List and Status Tests ===

#[test]
fn test_list_command() {
    let (exit_code, stdout, stderr) = run_cmd(&["list"]);
    // No backend available is acceptable in CI (macOS without Docker/KVM)
    if stderr.contains("No sandbox backend available") {
        return;
    }
    // Should succeed even with no sandboxes
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    // Output should contain header or empty-state message
    assert!(
        stdout.contains("NAME")
            || stdout.contains("No sandboxes")
            || stdout.is_empty()
            || stderr.contains("Using")
    );
}

#[test]
fn test_agents_command() {
    let (exit_code, stdout, stderr) = run_cmd(&["agents"]);
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    // Should list at least one agent (output has proper names like "Claude Code")
    assert!(
        stdout.contains("Claude")
            || stdout.contains("Gemini")
            || stdout.contains("Codex")
            || stdout.contains("OpenCode")
            || stdout.contains("AGENT") // header
    );
}

#[test]
fn test_status_command() {
    let (exit_code, _stdout, _stderr) = run_cmd(&["status"]);
    // Status should always succeed (shows installation state)
    assert_eq!(exit_code, 0);
}

// === Error Handling Tests ===

#[test]
fn test_invalid_command() {
    let (exit_code, _stdout, stderr) = run_cmd(&["invalid-command"]);
    assert_ne!(exit_code, 0);
    assert!(stderr.contains("error") || stderr.contains("unrecognized"));
}

#[test]
fn test_create_missing_name() {
    let (exit_code, _stdout, stderr) = run_cmd(&["create"]);
    assert_ne!(exit_code, 0);
    assert!(stderr.contains("required") || stderr.contains("error"));
}

#[test]
fn test_start_missing_name() {
    let (exit_code, _stdout, stderr) = run_cmd(&["start"]);
    assert_ne!(exit_code, 0);
    assert!(stderr.contains("required") || stderr.contains("error"));
}

#[test]
fn test_stop_missing_name() {
    let (exit_code, _stdout, stderr) = run_cmd(&["stop"]);
    assert_ne!(exit_code, 0);
    assert!(stderr.contains("required") || stderr.contains("error"));
}

#[test]
fn test_exec_missing_name() {
    let (exit_code, _stdout, stderr) = run_cmd(&["exec"]);
    assert_ne!(exit_code, 0);
    assert!(stderr.contains("required") || stderr.contains("error"));
}

#[test]
fn test_start_nonexistent_sandbox() {
    let (exit_code, _stdout, stderr) = run_cmd(&["start", "nonexistent-sandbox-12345"]);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("not found") || stderr.contains("error") || stderr.contains("Error"),
        "stderr was: {}",
        stderr
    );
}

#[test]
fn test_stop_nonexistent_sandbox() {
    // Stop on nonexistent sandbox should succeed (idempotent)
    // or fail gracefully
    let (exit_code, _stdout, _stderr) = run_cmd(&["stop", "nonexistent-sandbox-12345"]);
    // Either success (idempotent) or clean error is acceptable
    assert!(exit_code == 0 || exit_code == 1);
}

// === Validation Tests ===

#[test]
fn test_create_invalid_name_spaces() {
    let (exit_code, _stdout, stderr) = run_cmd(&["create", "invalid name with spaces"]);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("invalid") || stderr.contains("error") || stderr.contains("Error"),
        "stderr was: {}",
        stderr
    );
}

#[test]
fn test_create_invalid_name_special_chars() {
    let (exit_code, _stdout, stderr) = run_cmd(&["create", "invalid@name!"]);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("invalid") || stderr.contains("error") || stderr.contains("Error"),
        "stderr was: {}",
        stderr
    );
}

#[test]
fn test_backend_option() {
    // Test that --backend option is recognized in create help
    let (exit_code, stdout, _stderr) = run_cmd(&["create", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("--backend") || stdout.contains("-B"));
}

// === Init Tests ===

#[test]
fn test_init_help() {
    let (exit_code, stdout, _stderr) = run_cmd(&["init", "--help"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Initialize"));
}
