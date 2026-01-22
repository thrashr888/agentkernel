//! Benchmark test for Hyperlight backend
//!
//! Run with: cargo test --test hyperlight_benchmark --features hyperlight -- --nocapture --ignored
//!
//! **Requirements:**
//! - Linux with KVM (`/dev/kvm` accessible)
//! - Build with `--features hyperlight`

use std::time::Instant;

/// A minimal valid Wasm module that exports a simple function
/// This is the simplest possible Wasm module that Hyperlight can load:
/// - Type section: defines function type () -> i32
/// - Function section: declares one function of that type
/// - Export section: exports the function as "add"
/// - Code section: function body that returns 42
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
const MINIMAL_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, // magic: \0asm
    0x01, 0x00, 0x00, 0x00, // version: 1
    // Type section (id=1)
    0x01, 0x05, // section id=1, size=5
    0x01,       // 1 type
    0x60,       // func type
    0x00,       // 0 params
    0x01, 0x7f, // 1 result: i32
    // Function section (id=3)
    0x03, 0x02, // section id=3, size=2
    0x01,       // 1 function
    0x00,       // type index 0
    // Export section (id=7)
    0x07, 0x07, // section id=7, size=7
    0x01,       // 1 export
    0x03,       // name length: 3
    b'a', b'd', b'd', // name: "add"
    0x00,       // export kind: function
    0x00,       // function index 0
    // Code section (id=10)
    0x0a, 0x06, // section id=10, size=6
    0x01,       // 1 function body
    0x04,       // body size: 4
    0x00,       // 0 locals
    0x41, 0x2a, // i32.const 42
    0x0b,       // end
];

#[test]
#[ignore] // Run manually with --ignored
fn benchmark_hyperlight_startup() {
    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        eprintln!("Hyperlight benchmark requires Linux with KVM and --features hyperlight");
        eprintln!("Skipping on this platform.");
        return;
    }

    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        use agentkernel::hyperlight_backend::HyperlightSandbox;

        const WARMUP: usize = 3;
        const ITERATIONS: usize = 20;

        println!("\n=== Hyperlight Benchmark ===\n");
        println!("Iterations: {} (after {} warmup)", ITERATIONS, WARMUP);
        println!();

        // Warm up
        println!("Warming up...");
        for _ in 0..WARMUP {
            let mut sandbox = HyperlightSandbox::new("warmup");
            match sandbox.init_with_wasm(MINIMAL_WASM) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Warmup failed: {}", e);
                    return;
                }
            }
        }

        // Benchmark full lifecycle: create + init
        println!("\nBenchmarking sandbox creation + Wasm init...");
        let mut times_us: Vec<u128> = Vec::with_capacity(ITERATIONS);

        for i in 0..ITERATIONS {
            let start = Instant::now();
            let mut sandbox = HyperlightSandbox::new(&format!("bench-{}", i));
            match sandbox.init_with_wasm(MINIMAL_WASM) {
                Ok(_) => {
                    let elapsed = start.elapsed().as_micros();
                    times_us.push(elapsed);
                }
                Err(e) => {
                    eprintln!("Iteration {} failed: {}", i, e);
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
            "| Hyperlight | {:.2}ms (avg), {:.2}ms (p50) |",
            avg / 1000.0,
            p50 / 1000.0
        );
        println!("| Firecracker Daemon | 195ms |");
        println!("| Docker Pool | 250ms |");
        println!("| Apple Containers | 940ms |");
        println!();

        let speedup_fc = 195.0 / (avg / 1000.0);
        let speedup_docker = 250.0 / (avg / 1000.0);
        println!("Speedup vs Firecracker Daemon: {:.1}x", speedup_fc);
        println!("Speedup vs Docker Pool: {:.1}x", speedup_docker);
    }
}

#[test]
#[ignore]
fn benchmark_hyperlight_execution() {
    #[cfg(not(all(target_os = "linux", feature = "hyperlight")))]
    {
        eprintln!("Hyperlight benchmark requires Linux with KVM and --features hyperlight");
        return;
    }

    #[cfg(all(target_os = "linux", feature = "hyperlight"))]
    {
        use agentkernel::hyperlight_backend::HyperlightSandbox;

        println!("\n=== Hyperlight Execution Benchmark ===\n");

        // Create and init sandbox
        let mut sandbox = HyperlightSandbox::new("exec-bench");
        match sandbox.init_with_wasm(MINIMAL_WASM) {
            Ok(_) => println!("Sandbox initialized successfully"),
            Err(e) => {
                println!("Sandbox init error: {}", e);
                return;
            }
        }

        // Benchmark function calls
        const ITERATIONS: usize = 100;
        println!("\nBenchmarking {} function calls...", ITERATIONS);

        let start = Instant::now();
        for _ in 0..ITERATIONS {
            match sandbox.call_function::<i32>("add") {
                Ok(result) => {
                    assert_eq!(result, 42);
                }
                Err(e) => {
                    println!("Function call failed: {}", e);
                    return;
                }
            }
        }
        let total = start.elapsed();
        let avg_us = total.as_micros() as f64 / ITERATIONS as f64;

        println!("\n=== Execution Results ===");
        println!("| Metric | Time |");
        println!("|--------|------|");
        println!("| Total ({} calls) | {:?} |", ITERATIONS, total);
        println!("| Average per call | {:.2}µs ({:.3}ms) |", avg_us, avg_us / 1000.0);
        println!();

        // Compare with Docker exec
        println!("=== Comparison ===");
        println!("| Backend | Exec latency |");
        println!("|---------|--------------|");
        println!("| Hyperlight | {:.3}ms |", avg_us / 1000.0);
        println!("| Firecracker vsock | 19ms |");
        println!("| Docker exec | 83ms |");
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
