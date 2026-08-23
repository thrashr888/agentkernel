//! Apple Containers lifecycle smoke test.
//!
//! This is kept separate from the Docker/Podman test because the Apple
//! container CLI and system service are only available on macOS 26+.

#![cfg(target_os = "macos")]

use std::process::{Command, Output};

fn agentkernel_bin() -> String {
    format!("{}/target/debug/agentkernel", env!("CARGO_MANIFEST_DIR"))
}

fn run_agentkernel(args: &[&str]) -> Output {
    Command::new(agentkernel_bin())
        .args(args)
        .output()
        .expect("failed to execute agentkernel")
}

fn cleanup(name: &str) {
    let _ = run_agentkernel(["sandbox", "stop", name].as_slice());
    let _ = run_agentkernel(["sandbox", "remove", name].as_slice());
    let _ = Command::new("container")
        .args(["delete", "-f", &format!("agentkernel-{name}")])
        .output();
}

struct CleanupGuard {
    name: String,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        cleanup(&self.name);
    }
}

#[test]
#[ignore = "requires Apple Containers on macOS 26+"]
fn apple_container_lifecycle_smoke() {
    let version = Command::new("sw_vers")
        .args(["-productVersion"])
        .output()
        .expect("failed to query macOS version");
    assert!(version.status.success());
    let major = String::from_utf8_lossy(&version.stdout)
        .trim()
        .split('.')
        .next()
        .expect("macOS version is empty")
        .parse::<u32>()
        .expect("macOS version is not numeric");
    assert!(major >= 26, "Apple Containers requires macOS 26+");

    let container = Command::new("container")
        .arg("--version")
        .output()
        .expect("Apple container CLI is not installed");
    assert!(container.status.success());

    let name = format!("compat-apple-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let _cleanup = CleanupGuard { name: name.clone() };

    let created = run_agentkernel(
        [
            "sandbox",
            "create",
            &name,
            "--backend",
            "apple",
            "--no-start",
        ]
        .as_slice(),
    );
    assert!(
        created.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let started = run_agentkernel(["sandbox", "start", &name].as_slice());
    assert!(
        started.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&started.stderr)
    );
    let executed =
        run_agentkernel(["exec", &name, "--", "sh", "-c", "printf apple-smoke"].as_slice());
    assert!(
        executed.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout).trim(),
        "apple-smoke"
    );
}
