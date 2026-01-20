//! Pool benchmark: Compare container pool performance vs direct creation.
//!
//! This test demonstrates the speedup from using a warm container pool
//! instead of creating/destroying containers per operation.
//!
//! Run with: cargo test --test pool_benchmark -- --nocapture --ignored

use std::process::Command;
use std::time::{Duration, Instant};

const ITERATIONS: usize = 20;
const POOL_SIZE: usize = 5;
const IMAGE: &str = "alpine:3.20";

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
        .map_err(|e| format!("Failed to run: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn docker_run(name: &str, cmd: &str) -> Result<Duration, String> {
    let start = Instant::now();

    // Create container
    let container_name = format!("agentkernel-pool-bench-{}", name);
    let output = Command::new("docker")
        .args([
            "run",
            "-d",
            "--rm",
            "--name",
            &container_name,
            "--entrypoint",
            "sh",
            IMAGE,
            "-c",
            "while true; do sleep 3600; done",
        ])
        .output()
        .map_err(|e| format!("Failed to start: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    // Run command
    let output = Command::new("docker")
        .args(["exec", &container_name, "echo", "hello"])
        .output()
        .map_err(|e| format!("Failed to run command: {}", e))?;

    if !output.status.success() {
        let _ = Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    // Stop container
    let _ = Command::new("docker")
        .args(["rm", "-f", &container_name])
        .output();

    Ok(start.elapsed())
}

fn docker_exec_only(container_name: &str) -> Result<Duration, String> {
    let start = Instant::now();

    let output = Command::new("docker")
        .args(["exec", container_name, "echo", "hello"])
        .output()
        .map_err(|e| format!("Failed to run command: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(start.elapsed())
}

#[tokio::test]
#[ignore] // Run manually: cargo test --test pool_benchmark -- --nocapture --ignored
async fn benchmark_pool_concept() {
    println!("\n=== Container Pool Concept Benchmark ===\n");
    println!("This test compares:");
    println!("  - POOLED: Pre-started container, just run command");
    println!("  - DIRECT: Create → Start → Command → Stop → Remove\n");

    // Create pool of warm containers
    println!("Creating {} warm containers...", POOL_SIZE);
    let mut pool_containers = Vec::new();

    for i in 0..POOL_SIZE {
        let name = format!("agentkernel-pool-warm-{}", i);
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &name,
                "--entrypoint",
                "sh",
                IMAGE,
                "-c",
                "while true; do sleep 3600; done",
            ])
            .output()
            .expect("Failed to create pool container");

        if output.status.success() {
            pool_containers.push(name);
        }
    }

    println!("Pool ready: {} containers\n", pool_containers.len());

    // Benchmark pooled (just command execution)
    println!("Benchmarking POOLED ({} iterations)...", ITERATIONS);
    let mut pooled_times = Vec::new();

    for i in 0..ITERATIONS {
        let container = &pool_containers[i % pool_containers.len()];
        match docker_exec_only(container) {
            Ok(elapsed) => pooled_times.push(elapsed),
            Err(e) => eprintln!("  Error: {}", e),
        }

        if (i + 1) % 5 == 0 {
            println!("  {}/{}", i + 1, ITERATIONS);
        }
    }

    // Benchmark direct (full lifecycle)
    println!("\nBenchmarking DIRECT ({} iterations)...", ITERATIONS);
    let mut direct_times = Vec::new();

    for i in 0..ITERATIONS {
        match docker_run(&format!("direct-{}", i), "echo hello") {
            Ok(elapsed) => direct_times.push(elapsed),
            Err(e) => eprintln!("  Error: {}", e),
        }

        if (i + 1) % 5 == 0 {
            println!("  {}/{}", i + 1, ITERATIONS);
        }
    }

    // Cleanup pool
    println!("\nCleaning up pool containers...");
    for name in &pool_containers {
        let _ = Command::new("docker").args(["rm", "-f", name]).output();
    }

    // Calculate stats
    let pooled_total: Duration = pooled_times.iter().sum();
    let pooled_avg = pooled_total / pooled_times.len() as u32;
    let pooled_min = pooled_times.iter().min().unwrap_or(&Duration::ZERO);
    let pooled_max = pooled_times.iter().max().unwrap_or(&Duration::ZERO);

    let direct_total: Duration = direct_times.iter().sum();
    let direct_avg = direct_total / direct_times.len() as u32;
    let direct_min = direct_times.iter().min().unwrap_or(&Duration::ZERO);
    let direct_max = direct_times.iter().max().unwrap_or(&Duration::ZERO);

    let speedup = direct_avg.as_micros() as f64 / pooled_avg.as_micros().max(1) as f64;

    // Print results
    println!("\n==========================================");
    println!("           BENCHMARK RESULTS");
    println!("==========================================\n");

    println!("| Metric        | Pooled     | Direct     | Speedup |");
    println!("|---------------|------------|------------|---------|");
    println!(
        "| Avg latency   | {:>8}ms | {:>8}ms | {:>5.1}x  |",
        pooled_avg.as_millis(),
        direct_avg.as_millis(),
        speedup
    );
    println!(
        "| Min latency   | {:>8}ms | {:>8}ms |         |",
        pooled_min.as_millis(),
        direct_min.as_millis()
    );
    println!(
        "| Max latency   | {:>8}ms | {:>8}ms |         |",
        pooled_max.as_millis(),
        direct_max.as_millis()
    );
    println!(
        "| Total time    | {:>8}ms | {:>8}ms | {:>5.1}x  |",
        pooled_total.as_millis(),
        direct_total.as_millis(),
        speedup
    );

    println!(
        "\n=== Savings per command: ~{}ms ===",
        direct_avg.as_millis() as i64 - pooled_avg.as_millis() as i64
    );

    println!("\n=== POOL BENCHMARK COMPLETE ===\n");

    assert!(
        speedup > 2.0,
        "Expected at least 2x speedup, got {:.1}x",
        speedup
    );
}
