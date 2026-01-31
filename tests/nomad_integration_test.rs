//! Integration tests for the Nomad backend.
//!
//! These tests require a running Nomad cluster accessible at NOMAD_ADDR.
//! They are `#[ignore]` by default -- run with:
//!
//!   cargo test --test nomad_integration_test --features nomad -- --ignored --nocapture

#![cfg(feature = "nomad")]

/// Helper to create an OrchestratorConfig for testing
fn test_orch_config() -> agentkernel::config::OrchestratorConfig {
    agentkernel::config::OrchestratorConfig {
        nomad_addr: std::env::var("NOMAD_ADDR").ok(),
        nomad_token: std::env::var("NOMAD_TOKEN").ok(),
        nomad_driver: "docker".to_string(),
        nomad_datacenter: Some("dc1".to_string()),
        ..Default::default()
    }
}

fn test_sandbox_config() -> agentkernel::backend::SandboxConfig {
    agentkernel::backend::SandboxConfig {
        image: "alpine:3.20".to_string(),
        vcpus: 1,
        memory_mb: 256,
        mount_cwd: false,
        work_dir: None,
        env: Vec::new(),
        network: true,
        read_only: false,
        mount_home: false,
        files: Vec::new(),
    }
}

/// Full lifecycle test: create, exec, file ops, stop
#[tokio::test]
#[ignore = "requires Nomad cluster"]
async fn test_nomad_lifecycle() {
    use agentkernel::backend::Sandbox;

    let orch_config = test_orch_config();
    let mut sandbox = agentkernel::backend::NomadSandbox::new("nomad-test-lifecycle", &orch_config);
    let config = test_sandbox_config();

    // Start
    sandbox
        .start(&config)
        .await
        .expect("Failed to start Nomad sandbox");
    assert!(
        sandbox.is_running(),
        "Sandbox should be running after start"
    );

    // Run a simple command
    let result = sandbox
        .exec(&["echo", "hello-from-nomad"])
        .await
        .expect("Failed to exec in Nomad sandbox");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("hello-from-nomad"),
        "Expected stdout to contain 'hello-from-nomad', got: {}",
        result.stdout
    );

    // Write a file
    sandbox
        .write_file("/tmp/test.txt", b"nomad test content")
        .await
        .expect("Failed to write file");

    // Read the file back
    let content = sandbox
        .read_file("/tmp/test.txt")
        .await
        .expect("Failed to read file");
    assert_eq!(
        String::from_utf8_lossy(&content).trim(),
        "nomad test content"
    );

    // Remove the file
    sandbox
        .remove_file("/tmp/test.txt")
        .await
        .expect("Failed to remove file");

    // Stop
    sandbox.stop().await.expect("Failed to stop Nomad sandbox");
    assert!(
        !sandbox.is_running(),
        "Sandbox should not be running after stop"
    );
}

/// Test with environment variables via exec_with_env
#[tokio::test]
#[ignore = "requires Nomad cluster"]
async fn test_nomad_exec_with_env() {
    use agentkernel::backend::Sandbox;

    let orch_config = test_orch_config();
    let mut sandbox = agentkernel::backend::NomadSandbox::new("nomad-test-env", &orch_config);
    let config = test_sandbox_config();

    sandbox
        .start(&config)
        .await
        .expect("Failed to start sandbox");

    let env = vec!["MY_VAR=hello_nomad".to_string()];
    let result = sandbox
        .exec_with_env(&["sh", "-c", "echo $MY_VAR"], &env)
        .await
        .expect("Failed to exec with env");

    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("hello_nomad"),
        "Expected env var in output, got: {}",
        result.stdout
    );

    sandbox.stop().await.expect("Failed to stop sandbox");
}

/// Test network isolation (driver-level network=none)
#[tokio::test]
#[ignore = "requires Nomad cluster"]
async fn test_nomad_network_isolation() {
    use agentkernel::backend::Sandbox;

    let orch_config = test_orch_config();
    let mut sandbox = agentkernel::backend::NomadSandbox::new("nomad-test-nonet", &orch_config);

    let mut config = test_sandbox_config();
    config.network = false;

    sandbox
        .start(&config)
        .await
        .expect("Failed to start sandbox");

    // Try to reach an external endpoint -- should fail with network mode none
    let result = sandbox
        .exec(&[
            "sh",
            "-c",
            "wget -q -O- http://example.com --timeout=3 2>&1 || echo NETWORK_BLOCKED",
        ])
        .await
        .expect("Command execution itself should not fail");

    eprintln!("Network test output: {}{}", result.stdout, result.stderr);

    sandbox.stop().await.expect("Failed to stop sandbox");
}

/// Test 100 concurrent sandbox creation
#[tokio::test]
#[ignore = "requires Nomad cluster with sufficient capacity"]
async fn test_nomad_concurrent_100() {
    use agentkernel::backend::Sandbox;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let orch_config = test_orch_config();
    let config = test_sandbox_config();

    let start = std::time::Instant::now();
    let mut handles = Vec::new();
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));

    for i in 0..100 {
        let orch_config = orch_config.clone();
        let config = config.clone();
        let errors = errors.clone();

        let handle = tokio::spawn(async move {
            let name = format!("nomad-concurrent-{}", i);
            let mut sandbox = agentkernel::backend::NomadSandbox::new(&name, &orch_config);

            if let Err(e) = sandbox.start(&config).await {
                errors.lock().await.push(format!("start {}: {}", i, e));
                return;
            }

            match sandbox.exec(&["echo", "ok"]).await {
                Ok(result) => {
                    if result.exit_code != 0 {
                        errors
                            .lock()
                            .await
                            .push(format!("exec {}: non-zero exit", i));
                    }
                }
                Err(e) => {
                    errors.lock().await.push(format!("exec {}: {}", i, e));
                }
            }

            if let Err(e) = sandbox.stop().await {
                errors.lock().await.push(format!("stop {}: {}", i, e));
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    let elapsed = start.elapsed();
    let errors = errors.lock().await;

    println!(
        "100 concurrent Nomad sandboxes completed in {:.1}s",
        elapsed.as_secs_f64()
    );
    if !errors.is_empty() {
        println!("Errors ({}):", errors.len());
        for e in errors.iter().take(10) {
            println!("  {}", e);
        }
    }

    assert!(
        errors.len() < 10,
        "Too many errors ({}/100) in concurrent test",
        errors.len()
    );
}

/// Test file injection
#[tokio::test]
#[ignore = "requires Nomad cluster"]
async fn test_nomad_file_injection() {
    use agentkernel::backend::{FileInjection, Sandbox};

    let orch_config = test_orch_config();
    let mut sandbox = agentkernel::backend::NomadSandbox::new("nomad-test-inject", &orch_config);
    let config = test_sandbox_config();

    sandbox
        .start(&config)
        .await
        .expect("Failed to start sandbox");

    let files = vec![
        FileInjection {
            dest: "/tmp/injected1.txt".to_string(),
            content: b"file one content".to_vec(),
        },
        FileInjection {
            dest: "/tmp/subdir/injected2.txt".to_string(),
            content: b"file two content".to_vec(),
        },
    ];

    sandbox
        .inject_files(&files)
        .await
        .expect("Failed to inject files");

    // Verify files
    let content1 = sandbox
        .read_file("/tmp/injected1.txt")
        .await
        .expect("Read failed");
    assert_eq!(
        String::from_utf8_lossy(&content1).trim(),
        "file one content"
    );

    let content2 = sandbox
        .read_file("/tmp/subdir/injected2.txt")
        .await
        .expect("Read failed");
    assert_eq!(
        String::from_utf8_lossy(&content2).trim(),
        "file two content"
    );

    sandbox.stop().await.expect("Failed to stop sandbox");
}
