//! Benchmark test for Hyperlight backend
//!
//! Run with: cargo test --test hyperlight_benchmark --features hyperlight -- --nocapture --ignored
//!
//! **Requirements:**
//! - Linux with KVM (`/dev/kvm` accessible)
//! - Build with `--features hyperlight`

#[cfg(all(target_os = "linux", feature = "hyperlight"))]
use std::time::Instant;

#[test]
#[ignore] // Run manually with --ignored
fn benchmark_hyperlight_runtime_startup() {
    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        eprintln!("Hyperlight benchmark requires Linux with KVM and --features hyperlight");
        eprintln!("Skipping on this platform.");
        return;
    }

    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        use hyperlight_wasm::SandboxBuilder;

        const WARMUP: usize = 3;
        const ITERATIONS: usize = 20;

        println!("\n=== Hyperlight Runtime Startup Benchmark ===\n");
        println!("This measures: SandboxBuilder::new().build() + load_runtime()");
        println!("Iterations: {} (after {} warmup)", ITERATIONS, WARMUP);
        println!();

        // Warm up
        println!("Warming up...");
        for _ in 0..WARMUP {
            let proto = match SandboxBuilder::new().build() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Warmup build failed: {}", e);
                    return;
                }
            };
            match proto.load_runtime() {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Warmup load_runtime failed: {}", e);
                    return;
                }
            }
        }

        // Benchmark full lifecycle: build + load_runtime
        println!("\nBenchmarking sandbox build + runtime load...");
        let mut times_us: Vec<u128> = Vec::with_capacity(ITERATIONS);

        for i in 0..ITERATIONS {
            let start = Instant::now();

            let proto = match SandboxBuilder::new()
                .with_guest_heap_size(10_000_000)
                .with_guest_stack_size(1_000_000)
                .build()
            {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Iteration {} build failed: {}", i, e);
                    return;
                }
            };

            match proto.load_runtime() {
                Ok(_) => {
                    let elapsed = start.elapsed().as_micros();
                    times_us.push(elapsed);
                }
                Err(e) => {
                    eprintln!("Iteration {} load_runtime failed: {}", i, e);
                    return;
                }
            }
        }

        // Calculate statistics
        times_us.sort();
        let total: u128 = times_us.iter().sum();
        let avg = total as f64 / ITERATIONS as f64;
        let min = *times_us.first().unwrap() as f64;
        let max = *times_us.last().unwrap() as f64;
        let p50 = times_us[ITERATIONS / 2] as f64;
        let p95 = times_us[(ITERATIONS as f64 * 0.95) as usize] as f64;
        let p99 = times_us[(ITERATIONS as f64 * 0.99) as usize] as f64;

        println!("\n=== Results ===");
        println!("| Metric | Time |");
        println!("|--------|------|");
        println!("| Average | {:.2}µs ({:.2}ms) |", avg, avg / 1000.0);
        println!("| Min | {:.2}µs ({:.2}ms) |", min, min / 1000.0);
        println!("| Max | {:.2}µs ({:.2}ms) |", max, max / 1000.0);
        println!("| p50 | {:.2}µs ({:.2}ms) |", p50, p50 / 1000.0);
        println!("| p95 | {:.2}µs ({:.2}ms) |", p95, p95 / 1000.0);
        println!("| p99 | {:.2}µs ({:.2}ms) |", p99, p99 / 1000.0);
        println!();

        // Compare with other backends
        println!("=== Comparison (from BENCHMARK.md) ===");
        println!("| Backend | Startup |");
        println!("|---------|---------|");
        println!(
            "| Hyperlight (runtime only) | {:.2}ms (avg), {:.2}ms (p50) |",
            avg / 1000.0,
            p50 / 1000.0
        );
        println!("| Firecracker Daemon | 195ms |");
        println!("| Docker Pool | 250ms |");
        println!("| Apple Containers | 940ms |");
        println!();

        if avg / 1000.0 > 0.0 {
            let speedup_fc = 195.0 / (avg / 1000.0);
            let speedup_docker = 250.0 / (avg / 1000.0);
            println!("Speedup vs Firecracker Daemon: {:.1}x", speedup_fc);
            println!("Speedup vs Docker Pool: {:.1}x", speedup_docker);
        }

        println!();
        println!("Note: This measures only runtime startup. Module loading adds additional");
        println!("overhead depending on module size. Hyperlight requires AOT-compiled Wasm");
        println!("modules for best performance.");
    }
}

/// Test to verify Hyperlight availability on the system
#[test]
fn test_hyperlight_availability() {
    use agentkernel::hyperlight_backend::hyperlight_available;

    let available = hyperlight_available();

    println!("\n=== Hyperlight Availability ===");
    println!("Available: {}", available);

    #[cfg(target_os = "linux")]
    {
        let kvm_exists = std::path::Path::new("/dev/kvm").exists();
        println!("KVM device exists: {}", kvm_exists);

        #[cfg(feature = "hyperlight")]
        {
            println!("Hyperlight feature: enabled");
            let hypervisor_present = hyperlight_wasm::is_hypervisor_present();
            println!("Hypervisor present: {}", hypervisor_present);
        }
        #[cfg(not(feature = "hyperlight"))]
        println!("Hyperlight feature: disabled (use --features hyperlight)");
    }

    #[cfg(target_os = "macos")]
    println!("Platform: macOS (Hyperlight not supported)");

    #[cfg(target_os = "windows")]
    println!("Platform: Windows (WHP support available)");
}

/// Benchmark Hyperlight pool performance
///
/// Run with: cargo test --test hyperlight_benchmark --features hyperlight -- --nocapture --ignored
#[test]
#[ignore] // Run manually with --ignored
fn benchmark_hyperlight_pool() {
    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        eprintln!("Hyperlight pool benchmark requires Linux with KVM and --features hyperlight");
        eprintln!("Skipping on this platform.");
        return;
    }

    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        use agentkernel::hyperlight_backend::{HyperlightPool, HyperlightPoolConfig};

        const WARMUP: usize = 2;
        const ITERATIONS: usize = 10;

        println!("\n=== Hyperlight Pool Benchmark ===\n");

        // Create pool with custom config
        let config = HyperlightPoolConfig {
            min_warm: 3,
            max_warm: 15, // Allow enough for benchmark iterations
            ..Default::default()
        };

        let pool = match HyperlightPool::new(config) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create pool: {}", e);
                return;
            }
        };

        // Measure warm-up time
        println!("Warming up pool (3 runtimes)...");
        let warm_start = Instant::now();
        if let Err(e) = pool.warm_up() {
            eprintln!("Failed to warm up pool: {}", e);
            return;
        }
        let warm_time = warm_start.elapsed();
        println!(
            "Pool warm-up: {:.2}ms (3 runtimes)",
            warm_time.as_secs_f64() * 1000.0
        );

        let stats = pool.stats();
        println!("Pool stats: {} warm runtimes\n", stats.warm_count);

        // Warmup iterations (discard)
        println!("Warmup iterations...");
        for _ in 0..WARMUP {
            match pool.acquire() {
                Ok(_runtime) => {
                    // Runtime is consumed when we load a module
                    // For this benchmark, we're just measuring acquire time
                }
                Err(e) => {
                    eprintln!("Warmup acquire failed: {}", e);
                }
            }
        }

        // Re-warm the pool
        let _ = pool.warm_up();

        // Benchmark acquire times (warm path)
        // First, pre-warm the pool with enough runtimes for all iterations
        println!("\nPre-warming pool with {} runtimes...", ITERATIONS);
        if let Err(e) = pool.warm_to(ITERATIONS) {
            eprintln!("Failed to pre-warm pool: {}", e);
            return;
        }
        let stats = pool.stats();
        println!("Pool now has {} warm runtimes", stats.warm_count);

        println!("\nBenchmarking warm acquire (pre-warmed pool)...");
        let mut warm_times_us: Vec<u128> = Vec::with_capacity(ITERATIONS);

        for i in 0..ITERATIONS {
            let start = Instant::now();
            match pool.acquire() {
                Ok(_runtime) => {
                    let elapsed = start.elapsed().as_micros();
                    warm_times_us.push(elapsed);
                }
                Err(e) => {
                    eprintln!("Iteration {} acquire failed: {}", i, e);
                }
            }
        }

        // Calculate statistics for warm acquire
        if !warm_times_us.is_empty() {
            warm_times_us.sort();
            let total: u128 = warm_times_us.iter().sum();
            let avg = total as f64 / warm_times_us.len() as f64;
            let min = *warm_times_us.first().unwrap() as f64;
            let max = *warm_times_us.last().unwrap() as f64;
            let p50_idx = warm_times_us.len() / 2;
            let p50 = warm_times_us[p50_idx] as f64;

            println!("\n=== Warm Acquire Results ===");
            println!("| Metric | Time |");
            println!("|--------|------|");
            println!("| Average | {:.2}µs ({:.3}ms) |", avg, avg / 1000.0);
            println!("| Min | {:.2}µs ({:.3}ms) |", min, min / 1000.0);
            println!("| Max | {:.2}µs ({:.3}ms) |", max, max / 1000.0);
            println!("| p50 | {:.2}µs ({:.3}ms) |", p50, p50 / 1000.0);
        }

        // Benchmark cold acquire (empty pool)
        println!("\nBenchmarking cold acquire (empty pool)...");
        pool.clear();

        let mut cold_times_us: Vec<u128> = Vec::with_capacity(5);
        for i in 0..5 {
            pool.clear(); // Ensure pool is empty

            let start = Instant::now();
            match pool.acquire() {
                Ok(_runtime) => {
                    let elapsed = start.elapsed().as_micros();
                    cold_times_us.push(elapsed);
                }
                Err(e) => {
                    eprintln!("Cold iteration {} acquire failed: {}", i, e);
                }
            }
        }

        if !cold_times_us.is_empty() {
            cold_times_us.sort();
            let total: u128 = cold_times_us.iter().sum();
            let avg = total as f64 / cold_times_us.len() as f64;
            let min = *cold_times_us.first().unwrap() as f64;
            let max = *cold_times_us.last().unwrap() as f64;

            println!("\n=== Cold Acquire Results (no warm runtimes) ===");
            println!("| Metric | Time |");
            println!("|--------|------|");
            println!("| Average | {:.2}µs ({:.2}ms) |", avg, avg / 1000.0);
            println!("| Min | {:.2}µs ({:.2}ms) |", min, min / 1000.0);
            println!("| Max | {:.2}µs ({:.2}ms) |", max, max / 1000.0);
        }

        println!("\n=== Summary ===");
        println!("Pool pre-warming eliminates the ~68ms runtime startup cost.");
        println!("Warm acquire from pool should be <1ms (mostly lock acquisition).");
        println!("Cold acquire falls back to full runtime startup (~68ms).");

        // Clean up
        pool.shutdown();
    }
}
