//! Benchmark test: Measure sandbox lifecycle performance at scale.
//!
//! This test runs many sandbox create/start/exec/stop/remove cycles to measure
//! performance characteristics and identify bottlenecks.
//!
//! Run with: cargo test --test benchmark_test -- --nocapture --ignored
//!
//! Configurable via environment:
//!   BENCH_SANDBOXES=100       - Number of sandboxes per iteration (default: 10)
//!   BENCH_ITERATIONS=100      - Number of iterations (default: 10)
//!   BENCH_MAX_CONCURRENT=50   - Max concurrent sandbox operations (default: 50)
//!
//! Example large run:
//!   BENCH_SANDBOXES=100 BENCH_ITERATIONS=10 BENCH_MAX_CONCURRENT=100 cargo test --test benchmark_test -- --nocapture --ignored
//!
//! Requirements:
//!   - agentkernel binary built (cargo build --release)
//!   - Setup complete (agentkernel setup -y)

use serde::Serialize;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

fn get_config() -> (usize, usize, usize) {
    let sandboxes = std::env::var("BENCH_SANDBOXES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let iterations = std::env::var("BENCH_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    // Limit concurrent sandbox operations to prevent Docker daemon/thread pool exhaustion
    let max_concurrent = std::env::var("BENCH_MAX_CONCURRENT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    (sandboxes, iterations, max_concurrent)
}

/// Results from a single sandbox lifecycle
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
struct LifecycleResult {
    iteration: usize,
    sandbox_id: usize,
    create_time: Duration,
    start_time: Duration,
    exec_time: Duration,
    stop_time: Duration,
    remove_time: Duration,
    total_time: Duration,
    success: bool,
}

/// Aggregate benchmark statistics
#[derive(Debug, Serialize)]
struct BenchmarkStats {
    total_cycles: usize,
    successful_cycles: usize,
    failed_cycles: usize,
    total_wall_time: Duration,

    // Per-operation stats
    avg_create: Duration,
    avg_start: Duration,
    avg_exec: Duration,
    avg_stop: Duration,
    avg_remove: Duration,
    avg_total: Duration,

    // Percentiles
    p50_total: Duration,
    p95_total: Duration,
    p99_total: Duration,

    // Throughput
    sandboxes_per_second: f64,
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
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[tokio::test]
#[ignore] // Run manually: cargo test --test benchmark_test -- --nocapture --ignored
async fn benchmark_sandbox_lifecycle() {
    let (sandboxes, iterations, max_concurrent) = get_config();

    println!(
        "\n=== Agentkernel Benchmark: {}x{} ({} total, max {} concurrent) ===\n",
        sandboxes,
        iterations,
        sandboxes * iterations,
        max_concurrent
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

    // Clean up any leftover bench-* sandboxes from previous runs
    println!("Cleaning up leftover sandboxes...");

    // 1. Remove Docker containers (names use "agentkernel-" prefix)
    let container_names: Vec<String> = (0..iterations)
        .flat_map(|iter| (0..sandboxes).map(move |id| format!("agentkernel-bench-{}-{}", iter, id)))
        .collect();
    for chunk in container_names.chunks(100) {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f"])
            .args(chunk)
            .output();
    }

    // 2. Remove sandbox state files from ~/.local/share/agentkernel/sandboxes/
    let sandboxes_dir = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".local/share/agentkernel/sandboxes"))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/agentkernel/sandboxes"));
    for iter in 0..iterations {
        for id in 0..sandboxes {
            let state_file = sandboxes_dir.join(format!("bench-{}-{}.json", iter, id));
            let _ = std::fs::remove_file(state_file);
        }
    }
    println!("Done.\n");

    let mut all_results: Vec<LifecycleResult> = Vec::new();
    let wall_start = Instant::now();

    // Create semaphore to limit concurrent sandbox operations
    let semaphore = Arc::new(Semaphore::new(max_concurrent));

    for iteration in 0..iterations {
        println!(
            "Iteration {}/{}: Running {} sandboxes in parallel...",
            iteration + 1,
            iterations,
            sandboxes
        );

        let iter_start = Instant::now();
        let results = run_parallel_sandboxes(iteration, sandboxes, Arc::clone(&semaphore)).await;
        let iter_time = iter_start.elapsed();

        let successful = results.iter().filter(|r| r.success).count();
        println!(
            "  Completed: {}/{} successful in {:?}",
            successful, sandboxes, iter_time
        );

        all_results.extend(results);
    }

    let total_wall_time = wall_start.elapsed();

    // Calculate and print statistics
    let stats = calculate_benchmark_stats(&all_results, total_wall_time);
    print_benchmark_results(&stats);

    // Save results to files
    save_benchmark_results(&stats, &all_results);

    // Assertions
    let success_rate = stats.successful_cycles as f64 / stats.total_cycles as f64;
    assert!(
        success_rate >= 0.95,
        "Success rate {:.1}% is below 95%",
        success_rate * 100.0
    );

    println!("\n=== BENCHMARK COMPLETE ===\n");
}

async fn run_parallel_sandboxes(
    iteration: usize,
    count: usize,
    semaphore: Arc<Semaphore>,
) -> Vec<LifecycleResult> {
    let completed_count = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(count);

    for sandbox_id in 0..count {
        let sem = Arc::clone(&semaphore);
        let completed = Arc::clone(&completed_count);

        let handle = tokio::spawn(async move {
            // Acquire semaphore permit before starting sandbox operations
            let _permit = sem.acquire().await.unwrap();

            let result = run_single_lifecycle(iteration, sandbox_id).await;

            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            // Print progress every 10 sandboxes
            if done % 10 == 0 {
                eprintln!("    Progress: {}/{}", done, count);
            }

            result
            // Permit automatically released when _permit is dropped
        });
        handles.push(handle);
    }

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => {
                results.push(LifecycleResult {
                    iteration,
                    sandbox_id: 0,
                    create_time: Duration::ZERO,
                    start_time: Duration::ZERO,
                    exec_time: Duration::ZERO,
                    stop_time: Duration::ZERO,
                    remove_time: Duration::ZERO,
                    total_time: Duration::ZERO,
                    success: false,
                });
                eprintln!("Task panic: {}", e);
            }
        }
    }

    results
}

async fn run_single_lifecycle(iteration: usize, sandbox_id: usize) -> LifecycleResult {
    let name = format!("bench-{}-{}", iteration, sandbox_id);
    let total_start = Instant::now();
    let mut success = true;

    // Create
    let create_start = Instant::now();
    if run_cmd(&["create", &name, "--agent", "claude"]).is_err() {
        return LifecycleResult {
            iteration,
            sandbox_id,
            create_time: create_start.elapsed(),
            start_time: Duration::ZERO,
            exec_time: Duration::ZERO,
            stop_time: Duration::ZERO,
            remove_time: Duration::ZERO,
            total_time: total_start.elapsed(),
            success: false,
        };
    }
    let create_time = create_start.elapsed();

    // Start
    let start_start = Instant::now();
    if run_cmd(&["start", &name]).is_err() {
        let _ = run_cmd(&["remove", &name]);
        return LifecycleResult {
            iteration,
            sandbox_id,
            create_time,
            start_time: start_start.elapsed(),
            exec_time: Duration::ZERO,
            stop_time: Duration::ZERO,
            remove_time: Duration::ZERO,
            total_time: total_start.elapsed(),
            success: false,
        };
    }
    let start_time = start_start.elapsed();

    // Exec
    let exec_start = Instant::now();
    if run_cmd(&["exec", &name, "echo", "benchmark"]).is_err() {
        success = false;
    }
    let exec_time = exec_start.elapsed();

    // Stop
    let stop_start = Instant::now();
    if run_cmd(&["stop", &name]).is_err() {
        success = false;
    }
    let stop_time = stop_start.elapsed();

    // Remove
    let remove_start = Instant::now();
    if run_cmd(&["remove", &name]).is_err() {
        success = false;
    }
    let remove_time = remove_start.elapsed();

    LifecycleResult {
        iteration,
        sandbox_id,
        create_time,
        start_time,
        exec_time,
        stop_time,
        remove_time,
        total_time: total_start.elapsed(),
        success,
    }
}

fn calculate_benchmark_stats(
    results: &[LifecycleResult],
    total_wall_time: Duration,
) -> BenchmarkStats {
    let total_cycles = results.len();
    let successful_cycles = results.iter().filter(|r| r.success).count();
    let failed_cycles = total_cycles - successful_cycles;

    // Average times (only from successful cycles)
    let successful_results: Vec<_> = results.iter().filter(|r| r.success).collect();

    let avg_create = avg_duration(successful_results.iter().map(|r| r.create_time).collect());
    let avg_start = avg_duration(successful_results.iter().map(|r| r.start_time).collect());
    let avg_exec = avg_duration(successful_results.iter().map(|r| r.exec_time).collect());
    let avg_stop = avg_duration(successful_results.iter().map(|r| r.stop_time).collect());
    let avg_remove = avg_duration(successful_results.iter().map(|r| r.remove_time).collect());
    let avg_total = avg_duration(successful_results.iter().map(|r| r.total_time).collect());

    // Percentiles
    let mut total_times: Vec<_> = successful_results.iter().map(|r| r.total_time).collect();
    total_times.sort();

    let p50_total = percentile(&total_times, 50);
    let p95_total = percentile(&total_times, 95);
    let p99_total = percentile(&total_times, 99);

    // Throughput
    let sandboxes_per_second = if total_wall_time.as_secs_f64() > 0.0 {
        successful_cycles as f64 / total_wall_time.as_secs_f64()
    } else {
        0.0
    };

    BenchmarkStats {
        total_cycles,
        successful_cycles,
        failed_cycles,
        total_wall_time,
        avg_create,
        avg_start,
        avg_exec,
        avg_stop,
        avg_remove,
        avg_total,
        p50_total,
        p95_total,
        p99_total,
        sandboxes_per_second,
    }
}

fn avg_duration(durations: Vec<Duration>) -> Duration {
    if durations.is_empty() {
        return Duration::ZERO;
    }
    let total: Duration = durations.iter().sum();
    total / durations.len() as u32
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = (p * sorted.len() / 100).min(sorted.len() - 1);
    sorted[idx]
}

fn print_benchmark_results(stats: &BenchmarkStats) {
    println!("\n==========================================");
    println!("           BENCHMARK RESULTS");
    println!("==========================================\n");

    println!("Overview:");
    println!("  Total cycles:       {}", stats.total_cycles);
    println!("  Successful:         {}", stats.successful_cycles);
    println!("  Failed:             {}", stats.failed_cycles);
    println!("  Total wall time:    {:?}", stats.total_wall_time);
    println!(
        "  Throughput:         {:.2} sandboxes/sec",
        stats.sandboxes_per_second
    );

    println!("\nAverage Operation Times:");
    println!("  Create:             {:?}", stats.avg_create);
    println!("  Start:              {:?}", stats.avg_start);
    println!("  Exec:               {:?}", stats.avg_exec);
    println!("  Stop:               {:?}", stats.avg_stop);
    println!("  Remove:             {:?}", stats.avg_remove);
    println!("  Total lifecycle:    {:?}", stats.avg_total);

    println!("\nLatency Percentiles (total lifecycle):");
    println!("  p50:                {:?}", stats.p50_total);
    println!("  p95:                {:?}", stats.p95_total);
    println!("  p99:                {:?}", stats.p99_total);

    println!("\n==========================================\n");
}

fn save_benchmark_results(stats: &BenchmarkStats, results: &[LifecycleResult]) {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let results_dir = std::path::Path::new("benchmark-results");

    // Ensure directory exists
    let _ = fs::create_dir_all(results_dir);

    // Save summary stats as JSON
    let stats_file = results_dir.join(format!("benchmark_{}.json", timestamp));
    if let Ok(json) = serde_json::to_string_pretty(stats) {
        if let Err(e) = fs::write(&stats_file, json) {
            eprintln!("Failed to save benchmark stats: {}", e);
        } else {
            println!("Saved benchmark stats to: {}", stats_file.display());
        }
    }

    // Save detailed results as JSON
    let details_file = results_dir.join(format!("benchmark_{}_details.json", timestamp));
    if let Ok(json) = serde_json::to_string_pretty(results) {
        if let Err(e) = fs::write(&details_file, json) {
            eprintln!("Failed to save detailed results: {}", e);
        } else {
            println!("Saved detailed results to: {}", details_file.display());
        }
    }
}
