//! Benchmark test for the generic SandboxPool

use std::time::Instant;

/// Test sandbox pool warm-up and performance
#[tokio::test]
#[ignore] // Requires container runtime
async fn benchmark_sandbox_pool() {
    use agentkernel::backend::{SandboxConfig, detect_best_backend};
    use agentkernel::sandbox_pool::SandboxPool;

    // Detect best available backend
    let Some(backend) = detect_best_backend() else {
        eprintln!("No backend available, skipping benchmark");
        return;
    };

    println!("\n=== SandboxPool Benchmark ({}) ===", backend);

    // Measure pool startup time
    let start = Instant::now();
    let config = SandboxConfig::with_image("alpine:3.20");
    let pool = SandboxPool::with_config(backend, config, 3, 10).unwrap();
    pool.start().await.unwrap();
    let startup_time = start.elapsed();
    println!("Pool startup (3 warm): {:?}", startup_time);

    // Measure acquire time (should be fast - from warm pool)
    let start = Instant::now();
    let mut sandbox = pool.acquire().await.unwrap();
    let acquire_time = start.elapsed();
    println!("Acquire from pool: {:?}", acquire_time);

    // Measure command run time
    let start = Instant::now();
    let result = sandbox.exec(&["echo", "hello"]).await.unwrap();
    let run_time = start.elapsed();
    assert!(result.is_success());
    assert!(result.stdout.contains("hello"));
    println!("Command run: {:?}", run_time);

    // Release back to pool
    let start = Instant::now();
    pool.release(sandbox).await;
    let release_time = start.elapsed();
    println!("Release to pool: {:?}", release_time);

    // Measure second acquire (should be very fast - reused sandbox)
    let start = Instant::now();
    let mut sandbox2 = pool.acquire().await.unwrap();
    let reacquire_time = start.elapsed();
    println!("Re-acquire: {:?}", reacquire_time);

    // Run again to verify reuse works
    let result = sandbox2.exec(&["echo", "world"]).await.unwrap();
    assert!(result.is_success());
    assert!(result.stdout.contains("world"));

    pool.release(sandbox2).await;

    // Stop and cleanup
    let start = Instant::now();
    pool.stop().await.unwrap();
    let stop_time = start.elapsed();
    println!("Pool stop: {:?}", stop_time);

    // Show pool stats
    let stats = pool.stats().await;
    println!("\nFinal stats: {}", stats);

    // Performance assertions
    assert!(
        acquire_time.as_millis() < 100,
        "Acquire should be fast from warm pool"
    );
    assert!(
        reacquire_time.as_millis() < 100,
        "Re-acquire should be fast"
    );
    println!("\n=== Benchmark Complete ===\n");
}
