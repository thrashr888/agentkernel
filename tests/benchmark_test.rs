//! Benchmark test: Measure sandbox lifecycle performance at scale.
//!
//! This test runs many sandbox create/start/exec/stop/remove cycles to measure
//! performance characteristics and identify bottlenecks.
//!
//! Run with: cargo test --test benchmark_test -- --nocapture --ignored
//!
//! Configurable via environment:
//!   BENCH_SANDBOXES=100  - Number of sandboxes per iteration (default: 10)
//!   BENCH_ITERATIONS=100 - Number of iterations (default: 10)
//!
//! Requirements:
//!   - agentkernel binary built (cargo build --release)
//!   - Setup complete (agentkernel setup -y)

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

fn get_config() -> (usize, usize) {
    let sandboxes = std::env::var("BENCH_SANDBOXES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let iterations = std::env::var("BENCH_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    (sandboxes, iterations)
}

/// Results from a single sandbox lifecycle
#[derive(Debug, Clone)]
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
#[derive(Debug)]
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
    let (sandboxes, iterations) = get_config();

    println!(
        "\n=== Agentkernel Benchmark: {}x{} ({} total cycles) ===\n",
        sandboxes,
        iterations,
        sandboxes * iterations
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

    let mut all_results: Vec<LifecycleResult> = Vec::new();
    let wall_start = Instant::now();

    for iteration in 0..iterations {
        println!(
            "Iteration {}/{}: Running {} sandboxes in parallel...",
            iteration + 1,
            iterations,
            sandboxes
        );

        let iter_start = Instant::now();
        let results = run_parallel_sandboxes(iteration, sandboxes).await;
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

    // Assertions
    let success_rate = stats.successful_cycles as f64 / stats.total_cycles as f64;
    assert!(
        success_rate >= 0.95,
        "Success rate {:.1}% is below 95%",
        success_rate * 100.0
    );

    println!("\n=== BENCHMARK COMPLETE ===\n");
}

async fn run_parallel_sandboxes(iteration: usize, count: usize) -> Vec<LifecycleResult> {
    let success_count = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(count);

    for sandbox_id in 0..count {
        let success = Arc::clone(&success_count);
        let handle =
            tokio::spawn(async move { run_single_lifecycle(iteration, sandbox_id, success).await });
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

async fn run_single_lifecycle(
    iteration: usize,
    sandbox_id: usize,
    _success_count: Arc<AtomicUsize>,
) -> LifecycleResult {
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
