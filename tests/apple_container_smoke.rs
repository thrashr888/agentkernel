//! Apple Containers lifecycle smoke test.
//!
//! This is kept separate from the Docker/Podman test because the Apple
//! container CLI and system service are only available on macOS 26+.

#![cfg(target_os = "macos")]

use std::process::{Command, Output};

fn run_agentkernel(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agentkernel"))
        .args(args)
        .output()
        .expect("failed to execute agentkernel")
}

fn listed_container_has_id(output: &Output, id: &str) -> Result<bool, String> {
    if !output.status.success() {
        return Err(format!(
            "container list failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let containers: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid container list JSON: {error}"))?;
    // Validate every entry, even if a previous one matched the requested ID.
    let mut found = false;
    for container in containers {
        let container_id = container
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("invalid container list JSON: expected a nonempty string ID")?;
        found |= container_id == id;
    }
    Ok(found)
}

fn runtime_container_exists(id: &str) -> bool {
    let output = Command::new("container")
        .args(["list", "--all", "--format", "json"])
        .output()
        .expect("failed to execute container list");
    listed_container_has_id(&output, id).unwrap_or_else(|error| panic!("{error}"))
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

    let name = format!("compat-apple-{}", uuid::Uuid::new_v4());
    let runtime_id = format!("agentkernel-{name}");
    assert!(
        !runtime_container_exists(&runtime_id),
        "smoke runtime ID already exists: {runtime_id}"
    );
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
    assert!(
        runtime_container_exists(&runtime_id),
        "runtime did not list {runtime_id} after AgentKernel start"
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

    let stopped = run_agentkernel(&["sandbox", "stop", &name]);
    assert!(
        stopped.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    let removed = run_agentkernel(&["sandbox", "remove", &name]);
    assert!(
        removed.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    // Check while the guard is still alive; its force-delete fallback must
    // not hide a successful CLI removal that left an Apple container behind.
    assert!(
        !runtime_container_exists(&runtime_id),
        "runtime still lists {runtime_id} after AgentKernel remove"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn listing(status_code: i32, stdout: &[u8]) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(status_code << 8),
            stdout: stdout.to_vec(),
            stderr: if status_code != 0 {
                b"runtime unavailable".to_vec()
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn listing_requires_an_exact_container_id_even_for_stopped_entries() {
        let output = listing(
            0,
            br#"[{"id":"agentkernel-smoke-old","status":{"state":"running"}},
                 {"id":"agentkernel-smoke","status":{"state":"stopped"}}]"#,
        );
        assert_eq!(
            listed_container_has_id(&output, "agentkernel-smoke"),
            Ok(true)
        );
        assert_eq!(listed_container_has_id(&output, "agentkernel"), Ok(false));
        assert_eq!(
            listed_container_has_id(&listing(0, b"[]"), "agentkernel-smoke"),
            Ok(false)
        );
    }

    #[test]
    fn malformed_listing_and_runtime_error_never_mean_absent() {
        for invalid in [
            &b""[..],
            b"ID   STATE",
            b"{}",
            br#"[{"id":"agentkernel-smoke"},{"wrongKey":"missing"}]"#,
            br#"[["agentkernel-smoke"]]"#,
            br#"[{"id":123}]"#,
            br#"[{"id":""}]"#,
        ] {
            assert!(listed_container_has_id(&listing(0, invalid), "agentkernel-smoke").is_err());
        }
        let error = listed_container_has_id(&listing(1, b"[]"), "agentkernel-smoke").unwrap_err();
        assert!(error.contains("runtime unavailable"), "{error}");
    }
}
