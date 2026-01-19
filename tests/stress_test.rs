//! Stress test: Spin up multiple microVMs in parallel, run commands, verify output.
//!
//! This test validates that agentkernel can handle concurrent VM operations.
//! Target: <125ms per VM boot, all VMs complete successfully.
//!
//! Run with: cargo test --test stress_test -- --nocapture --ignored
//!
//! Requirements:
//!   - agentkernel binary built (cargo build --release)
//!   - Setup complete (agentkernel setup -y)

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const VM_COUNT: usize = 10;
const EXPECTED_OUTPUT: &str = "hello";
const MAX_TOTAL_TIME: Duration = Duration::from_secs(60);

/// Results from a single sandbox test
#[derive(Debug)]
#[allow(dead_code)]
struct SandboxTestResult {
    sandbox_id: usize,
    name: String,
    create_time: Duration,
    start_time: Duration,
    exec_time: Duration,
    stop_time: Duration,
    remove_time: Duration,
    output_correct: bool,
    error: Option<String>,
}

/// Aggregate results from the stress test
#[derive(Debug)]
struct StressTestResults {
    total_time: Duration,
    successful: usize,
    failed: usize,
    avg_create_time: Duration,
    avg_start_time: Duration,
    avg_exec_time: Duration,
    avg_stop_time: Duration,
    avg_remove_time: Duration,
    max_start_time: Duration,
    errors: Vec<String>,
}

fn get_binary_path() -> String {
    std::env::current_dir()
        .unwrap()
        .join("target/release/agentkernel")
        .to_string_lossy()
        .to_string()
}

fn run_cmd(args: &[&str]) -> Result<String, String> {
    let binary = get_binary_path();
    let output = Command::new(&binary)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!(
            "Command failed: {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            stdout,
            stderr
        ))
    }
}

#[tokio::test]
#[ignore] // Run manually: cargo test --test stress_test -- --nocapture --ignored
async fn test_parallel_sandboxes() {
    println!(
        "\n=== Agentkernel Stress Test: {} Sandboxes ===\n",
        VM_COUNT
    );

    // Check that binary exists
    let binary = get_binary_path();
    if !std::path::Path::new(&binary).exists() {
        panic!(
            "Binary not found at {}. Run 'cargo build --release' first.",
            binary
        );
    }

    // Check setup status
    println!("Checking setup status...");
    match run_cmd(&["status"]) {
        Ok(output) => println!("{}", output),
        Err(e) => panic!(
            "Setup check failed: {}. Run 'agentkernel setup -y' first.",
            e
        ),
    }

    let start = Instant::now();
    let success_count = Arc::new(AtomicUsize::new(0));
    let fail_count = Arc::new(AtomicUsize::new(0));

    // Spawn sandbox tasks concurrently
    let mut handles = Vec::with_capacity(VM_COUNT);

    for i in 0..VM_COUNT {
        let success = Arc::clone(&success_count);
        let fail = Arc::clone(&fail_count);

        let handle = tokio::spawn(async move {
            let result = run_single_sandbox_test(i).await;

            if result.error.is_none() && result.output_correct {
                success.fetch_add(1, Ordering::SeqCst);
            } else {
                fail.fetch_add(1, Ordering::SeqCst);
            }

            result
        });

        handles.push(handle);
    }

    // Wait for all sandboxes to complete
    let mut results = Vec::with_capacity(VM_COUNT);
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => {
                fail_count.fetch_add(1, Ordering::SeqCst);
                results.push(SandboxTestResult {
                    sandbox_id: 0,
                    name: "unknown".to_string(),
                    create_time: Duration::ZERO,
                    start_time: Duration::ZERO,
                    exec_time: Duration::ZERO,
                    stop_time: Duration::ZERO,
                    remove_time: Duration::ZERO,
                    output_correct: false,
                    error: Some(format!("Task panic: {}", e)),
                });
            }
        }
    }

    let total_time = start.elapsed();

    // Calculate statistics
    let stats = calculate_stats(&results, total_time);
    print_results(&stats);

    // Assertions
    assert!(
        stats.failed == 0,
        "Some sandboxes failed: {} failures out of {}",
        stats.failed,
        VM_COUNT
    );

    assert!(
        stats.total_time < MAX_TOTAL_TIME,
        "Total time {:?} exceeded maximum {:?}",
        stats.total_time,
        MAX_TOTAL_TIME
    );

    println!("\n=== STRESS TEST PASSED ===\n");
}

async fn run_single_sandbox_test(sandbox_id: usize) -> SandboxTestResult {
    let name = format!("stress-{}", sandbox_id);

    // Create sandbox
    let create_start = Instant::now();
    let create_result = run_cmd(&["create", &name, "--agent", "claude"]);
    let create_time = create_start.elapsed();

    if let Err(e) = create_result {
        return SandboxTestResult {
            sandbox_id,
            name,
            create_time,
            start_time: Duration::ZERO,
            exec_time: Duration::ZERO,
            stop_time: Duration::ZERO,
            remove_time: Duration::ZERO,
            output_correct: false,
            error: Some(format!("Create failed: {}", e)),
        };
    }

    // Start sandbox
    let start_start = Instant::now();
    let start_result = run_cmd(&["start", &name]);
    let start_time = start_start.elapsed();

    if let Err(e) = start_result {
        // Cleanup: try to remove
        let _ = run_cmd(&["remove", &name]);
        return SandboxTestResult {
            sandbox_id,
            name,
            create_time,
            start_time,
            exec_time: Duration::ZERO,
            stop_time: Duration::ZERO,
            remove_time: Duration::ZERO,
            output_correct: false,
            error: Some(format!("Start failed: {}", e)),
        };
    }

    // Execute command
    let exec_start = Instant::now();
    let exec_result = run_cmd(&["exec", &name, "echo", EXPECTED_OUTPUT]);
    let exec_time = exec_start.elapsed();

    let output_correct = match &exec_result {
        Ok(output) => output.contains(EXPECTED_OUTPUT),
        Err(_) => false,
    };

    let exec_error = exec_result.err();

    // Stop sandbox
    let stop_start = Instant::now();
    let stop_result = run_cmd(&["stop", &name]);
    let stop_time = stop_start.elapsed();

    // Remove sandbox
    let remove_start = Instant::now();
    let remove_result = run_cmd(&["remove", &name]);
    let remove_time = remove_start.elapsed();

    // Combine errors
    let error = exec_error
        .or_else(|| stop_result.err())
        .or_else(|| remove_result.err());

    SandboxTestResult {
        sandbox_id,
        name,
        create_time,
        start_time,
        exec_time,
        stop_time,
        remove_time,
        output_correct,
        error,
    }
}

fn calculate_stats(results: &[SandboxTestResult], total_time: Duration) -> StressTestResults {
    let successful = results.iter().filter(|r| r.error.is_none()).count();
    let failed = results.len() - successful;

    let create_times: Vec<_> = results.iter().map(|r| r.create_time).collect();
    let start_times: Vec<_> = results.iter().map(|r| r.start_time).collect();
    let exec_times: Vec<_> = results.iter().map(|r| r.exec_time).collect();
    let stop_times: Vec<_> = results.iter().map(|r| r.stop_time).collect();
    let remove_times: Vec<_> = results.iter().map(|r| r.remove_time).collect();

    let avg_create = avg_duration(&create_times);
    let avg_start = avg_duration(&start_times);
    let avg_exec = avg_duration(&exec_times);
    let avg_stop = avg_duration(&stop_times);
    let avg_remove = avg_duration(&remove_times);
    let max_start = start_times.iter().max().copied().unwrap_or(Duration::ZERO);

    let errors: Vec<_> = results.iter().filter_map(|r| r.error.clone()).collect();

    StressTestResults {
        total_time,
        successful,
        failed,
        avg_create_time: avg_create,
        avg_start_time: avg_start,
        avg_exec_time: avg_exec,
        avg_stop_time: avg_stop,
        avg_remove_time: avg_remove,
        max_start_time: max_start,
        errors,
    }
}

fn avg_duration(durations: &[Duration]) -> Duration {
    if durations.is_empty() {
        return Duration::ZERO;
    }
    let total: Duration = durations.iter().sum();
    total / durations.len() as u32
}

fn print_results(stats: &StressTestResults) {
    println!("\nResults:");
    println!("  Total time:       {:?}", stats.total_time);
    println!(
        "  Successful:       {}/{}",
        stats.successful,
        stats.successful + stats.failed
    );
    println!("  Failed:           {}", stats.failed);
    println!("  Avg create time:  {:?}", stats.avg_create_time);
    println!("  Avg start time:   {:?}", stats.avg_start_time);
    println!("  Avg exec time:    {:?}", stats.avg_exec_time);
    println!("  Avg stop time:    {:?}", stats.avg_stop_time);
    println!("  Avg remove time:  {:?}", stats.avg_remove_time);
    println!("  Max start time:   {:?}", stats.max_start_time);

    if !stats.errors.is_empty() {
        println!("\nErrors (first 5):");
        for (i, err) in stats.errors.iter().take(5).enumerate() {
            println!("  {}: {}", i + 1, err);
        }
        if stats.errors.len() > 5 {
            println!("  ... and {} more", stats.errors.len() - 5);
        }
    }
}
