#![cfg(target_os = "linux")]

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn create_persists_volumes_before_start_and_when_start_is_deferred() {
    let home = tempfile::tempdir().unwrap();
    let engine = home.path().join("docker");
    let log = home.path().join("engine-run");
    fs::write(
        &engine,
        r#"#!/bin/sh
case "$1" in
    version) printf 'linux\n';;
    run) printf '%s\n' "$@" > "$ENGINE_RUN"; printf 'fake-container-id\n';;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();

    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agentkernel"))
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", home.path())
            .env("AGENTKERNEL_PORT", "0")
            .env("ENGINE_RUN", &log)
            .current_dir(home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let success = |args: &[&str]| {
        let result = invoke(args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    };

    success(&["volume", "create", "test-config"]);
    let data = home.path().join(".local/share/agentkernel");
    let state = |name: &str| {
        let path = data.join("sandboxes").join(format!("{name}.json"));
        serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap()
    };
    let volume = "test-config:/home/sandbox:ro";
    let mount_arg = format!(
        "{}:/home/sandbox:ro",
        home.path()
            .join(".agentkernel/volumes/test-config")
            .display()
    );

    success(&[
        "sandbox",
        "create",
        "deferred",
        "--backend",
        "docker",
        "--volume",
        volume,
        "--no-start",
    ]);
    assert_eq!(state("deferred")["volumes"], serde_json::json!([volume]));
    assert!(!log.exists());
    success(&["sandbox", "start", "deferred"]);
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .any(|line| line == mount_arg)
    );

    fs::remove_file(&log).unwrap();
    success(&[
        "sandbox",
        "create",
        "immediate",
        "--backend",
        "docker",
        "--volume",
        volume,
    ]);
    assert_eq!(state("immediate")["volumes"], serde_json::json!([volume]));
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .lines()
            .any(|line| line == mount_arg)
    );

    let missing = invoke(&[
        "sandbox",
        "create",
        "missing",
        "--backend",
        "docker",
        "--volume",
        "no-such-volume:/home/sandbox",
        "--no-start",
    ]);
    assert!(!missing.status.success());
    assert!(!data.join("sandboxes/missing.json").exists());
}
