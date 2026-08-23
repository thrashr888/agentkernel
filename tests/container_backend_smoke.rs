//! Minimal lifecycle smoke test for the supported Docker and Podman adapters.
//!
//! The test is ignored in the normal unit suite because it needs a container
//! runtime. CI selects the runtime with `AGENTKERNEL_CONTAINER_BACKEND` and
//! runs this same contract against both adapters.

use std::process::{Command, Output};

fn backend() -> &'static str {
    match std::env::var("AGENTKERNEL_CONTAINER_BACKEND").as_deref() {
        Ok("podman") => "podman",
        Ok("docker") | Err(_) => "docker",
        Ok(other) => panic!("unsupported container backend '{other}'"),
    }
}

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
    let _ = Command::new(backend())
        .args(["rm", "-f", &format!("agentkernel-{name}")])
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
#[ignore = "requires Docker or Podman"]
fn container_backend_lifecycle_smoke() {
    let runtime = backend();
    let version = Command::new(runtime)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("{runtime} is not installed: {error}"));
    assert!(
        version.status.success(),
        "{runtime} --version failed: {}",
        String::from_utf8_lossy(&version.stderr)
    );

    let name = format!(
        "compat-{}-{}",
        runtime,
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let _cleanup = CleanupGuard { name: name.clone() };

    let created = run_agentkernel(&[
        "sandbox",
        "create",
        &name,
        "--backend",
        runtime,
        "--no-start",
    ]);
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
        run_agentkernel(["exec", &name, "--", "sh", "-c", "printf backend-smoke"].as_slice());
    assert!(
        executed.status.success(),
        "exec failed: {}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout).trim(),
        "backend-smoke"
    );

    let stopped = run_agentkernel(["sandbox", "stop", &name].as_slice());
    assert!(
        stopped.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stopped.stderr)
    );
}
