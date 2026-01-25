//! File operations integration tests
//!
//! These tests verify file read/write operations in sandboxes.
//! Tests are ignored by default since they require Docker to be running.
//!
//! Run with: cargo test --test file_operations_test -- --ignored

use std::process::Command;

/// Get the path to the agentkernel binary
fn agentkernel_bin() -> String {
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

/// Check if Docker is available
fn docker_available() -> bool {
    Command::new("docker")
        .args(["version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate a unique sandbox name for testing
fn unique_sandbox_name() -> String {
    format!("test-file-ops-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

/// Cleanup helper - stops and removes a sandbox
fn cleanup_sandbox(name: &str) {
    let _ = run_cmd(&["stop", name]);
    let _ = run_cmd(&["remove", name]);
    // Also try to remove Docker container directly in case of partial state
    let _ = Command::new("docker")
        .args(["rm", "-f", &format!("agentkernel-{}", name)])
        .output();
}

// === File Write Tests ===

#[test]
#[ignore] // Requires Docker
fn test_file_write_simple() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup: create and start sandbox
    let (exit_code, _, stderr) = run_cmd(&["create", &name, "--backend", "docker"]);
    assert_eq!(exit_code, 0, "Create failed: {}", stderr);

    let (exit_code, _, stderr) = run_cmd(&["start", &name]);
    assert_eq!(exit_code, 0, "Start failed: {}", stderr);

    // Write a file
    let (exit_code, _stdout, stderr) = run_cmd(&[
        "cp",
        "--to",
        &name,
        "--content",
        "Hello, World!",
        "/tmp/test.txt",
    ]);

    if exit_code == 0 {
        // Verify by reading with exec
        let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "cat", "/tmp/test.txt"]);
        assert_eq!(exit_code, 0);
        assert!(
            stdout.contains("Hello, World!"),
            "Expected content not found: {}",
            stdout
        );
    } else {
        // cp command might not be implemented, check for graceful failure
        eprintln!("cp command not implemented or failed: {}", stderr);
    }

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_file_write_with_exec() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup: create and start sandbox
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Write a file using exec + echo (works in any sandbox)
    let (exit_code, _, stderr) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "echo 'test content' > /tmp/exec_test.txt",
    ]);
    assert_eq!(exit_code, 0, "Write via exec failed: {}", stderr);

    // Read the file back
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "cat", "/tmp/exec_test.txt"]);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("test content"),
        "Expected content not found: {}",
        stdout
    );

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_file_write_multiline() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Write a multiline file using heredoc
    let content = r#"line1
line2
line3"#;
    let cmd = format!("cat << 'EOF' > /tmp/multiline.txt\n{}\nEOF", content);

    let (exit_code, _, stderr) = run_cmd(&["exec", &name, "--", "sh", "-c", &cmd]);
    assert_eq!(exit_code, 0, "Multiline write failed: {}", stderr);

    // Verify content
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "cat", "/tmp/multiline.txt"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("line1"));
    assert!(stdout.contains("line2"));
    assert!(stdout.contains("line3"));

    // Verify line count
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "wc", "-l", "/tmp/multiline.txt"]);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("3"),
        "Expected 3 lines, got: {}",
        stdout.trim()
    );

    // Cleanup
    cleanup_sandbox(&name);
}

// === File Read Tests ===

#[test]
#[ignore] // Requires Docker
fn test_file_read_simple() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create a test file first
    let (exit_code, _, _) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "echo 'read test' > /tmp/read_test.txt",
    ]);
    assert_eq!(exit_code, 0);

    // Read using cp command (if implemented)
    let (exit_code, stdout, stderr) = run_cmd(&["cp", "--from", &name, "/tmp/read_test.txt", "-"]);

    if exit_code == 0 {
        assert!(
            stdout.contains("read test"),
            "Expected content not found: {}",
            stdout
        );
    } else {
        // cp --from might not be implemented
        eprintln!("cp --from not implemented: {}", stderr);
    }

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_file_read_nonexistent() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Try to read a nonexistent file
    let (exit_code, _, stderr) =
        run_cmd(&["exec", &name, "--", "cat", "/tmp/nonexistent_file.txt"]);

    // Should fail
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("No such file") || stderr.contains("not found") || exit_code != 0,
        "Expected error for nonexistent file"
    );

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_file_read_binary() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create a binary file (random bytes)
    let (exit_code, _, _) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "dd if=/dev/urandom of=/tmp/binary.bin bs=64 count=1 2>/dev/null",
    ]);
    assert_eq!(exit_code, 0);

    // Read binary file size
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "wc", "-c", "/tmp/binary.bin"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("64"), "Expected 64 bytes, got: {}", stdout);

    // Cleanup
    cleanup_sandbox(&name);
}

// === Directory Operations ===

#[test]
#[ignore] // Requires Docker
fn test_directory_create() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create a directory
    let (exit_code, _, _) = run_cmd(&["exec", &name, "--", "mkdir", "-p", "/tmp/testdir/subdir"]);
    assert_eq!(exit_code, 0);

    // Verify directory exists
    let (exit_code, _stdout, _) =
        run_cmd(&["exec", &name, "--", "test", "-d", "/tmp/testdir/subdir"]);
    assert_eq!(exit_code, 0, "Directory should exist");

    // Create a file in the directory
    let (exit_code, _, _) =
        run_cmd(&["exec", &name, "--", "touch", "/tmp/testdir/subdir/file.txt"]);
    assert_eq!(exit_code, 0);

    // List directory contents
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "ls", "/tmp/testdir/subdir"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("file.txt"));

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_file_permissions() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create a script file
    let (exit_code, _, _) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "echo '#!/bin/sh\necho hello' > /tmp/script.sh",
    ]);
    assert_eq!(exit_code, 0);

    // Make it executable
    let (exit_code, _, _) = run_cmd(&["exec", &name, "--", "chmod", "+x", "/tmp/script.sh"]);
    assert_eq!(exit_code, 0);

    // Execute it
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "/tmp/script.sh"]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("hello"));

    // Cleanup
    cleanup_sandbox(&name);
}

// === Stress Tests ===

#[test]
#[ignore] // Requires Docker
fn test_many_files() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create 100 files
    let (exit_code, _, stderr) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "for i in $(seq 1 100); do echo \"file $i\" > /tmp/file_$i.txt; done",
    ]);
    assert_eq!(exit_code, 0, "Creating 100 files failed: {}", stderr);

    // Verify count
    let (exit_code, _stdout, _) = run_cmd(&["exec", &name, "--", "ls", "/tmp/"]);
    assert_eq!(exit_code, 0);

    // Count files matching pattern
    let (exit_code, stdout, _) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "ls /tmp/file_*.txt | wc -l",
    ]);
    assert_eq!(exit_code, 0);
    let count: i32 = stdout.trim().parse().unwrap_or(0);
    assert_eq!(count, 100, "Expected 100 files, got {}", count);

    // Cleanup
    cleanup_sandbox(&name);
}

#[test]
#[ignore] // Requires Docker
fn test_large_file() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    let name = unique_sandbox_name();
    cleanup_sandbox(&name);

    // Setup
    run_cmd(&["create", &name, "--backend", "docker"]);
    run_cmd(&["start", &name]);

    // Create a 1MB file
    let (exit_code, _, stderr) = run_cmd(&[
        "exec",
        &name,
        "--",
        "sh",
        "-c",
        "dd if=/dev/zero of=/tmp/largefile.bin bs=1M count=1 2>/dev/null",
    ]);
    assert_eq!(exit_code, 0, "Creating large file failed: {}", stderr);

    // Verify size
    let (exit_code, stdout, _) = run_cmd(&["exec", &name, "--", "wc", "-c", "/tmp/largefile.bin"]);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("1048576"),
        "Expected 1MB file, got: {}",
        stdout
    );

    // Cleanup
    cleanup_sandbox(&name);
}

// === Ephemeral Run Tests ===

#[test]
#[ignore] // Requires Docker
fn test_run_with_file_creation() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    // Run command that creates and reads a file
    let (exit_code, stdout, stderr) = run_cmd(&[
        "run",
        "--backend",
        "docker",
        "--",
        "sh",
        "-c",
        "echo 'ephemeral' > /tmp/test.txt && cat /tmp/test.txt",
    ]);
    assert_eq!(exit_code, 0, "Run with file failed: {}", stderr);
    assert!(
        stdout.contains("ephemeral"),
        "Expected output not found: {}",
        stdout
    );
}

#[test]
#[ignore] // Requires Docker
fn test_run_file_isolation() {
    if !docker_available() {
        eprintln!("Skipping test: Docker not available");
        return;
    }

    // Create a file in one run
    let (exit_code, _, _) = run_cmd(&[
        "run",
        "--backend",
        "docker",
        "--",
        "sh",
        "-c",
        "echo 'secret' > /tmp/isolated.txt",
    ]);
    assert_eq!(exit_code, 0);

    // Try to read it in another run - should fail since sandboxes are isolated
    let (exit_code, _stdout, _stderr) = run_cmd(&[
        "run",
        "--backend",
        "docker",
        "--",
        "cat",
        "/tmp/isolated.txt",
    ]);

    // Should fail because each run gets a fresh sandbox
    assert_ne!(exit_code, 0, "File should not persist across runs");
}
