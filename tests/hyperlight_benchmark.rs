//! Benchmark test for Hyperlight backend
//!
//! Run with: cargo test --test hyperlight_benchmark --features hyperlight -- --nocapture --ignored
//!
//! **Requirements:**
//! - Linux with KVM (`/dev/kvm` accessible)
//! - Build with `--features hyperlight`

use std::time::Instant;

/// Simple Wasm module that returns immediately (for measuring startup overhead)
/// This is a minimal valid Wasm module with just a _start function
#[cfg(all(target_os = "linux", feature = "hyperlight"))]
const MINIMAL_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, // magic
    0x01, 0x00, 0x00, 0x00, // version
    0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section: () -> ()
    0x03, 0x02, 0x01, 0x00, // function section: 1 function of type 0
    0x07, 0x09, 0x01, 0x05, 0x5f, 0x73, 0x74, 0x61, 0x72, 0x74, 0x00,
    0x00, // export "_start" as function 0
    0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b, // code section: function body (empty)
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

        const ITERATIONS: usize = 100;

        println!("\n=== Hyperlight Benchmark ===\n");
        println!("Testing sandbox creation and initialization...\n");

        // Warm up
        for _ in 0..5 {
            let mut sandbox = HyperlightSandbox::new("warmup");
            let _ = sandbox.init_with_wasm(MINIMAL_WASM);
        }

        // Benchmark sandbox creation (no Wasm)
        let start = Instant::now();
        for i in 0..ITERATIONS {
            let _sandbox = HyperlightSandbox::new(&format!("bench-{}", i));
        }
        let create_time = start.elapsed();
        let avg_create = create_time.as_micros() as f64 / ITERATIONS as f64;

        println!("Sandbox creation (no init):");
        println!("  Total: {:?}", create_time);
        println!(
            "  Average: {:.2}µs ({:.2}ms)",
            avg_create,
            avg_create / 1000.0
        );
        println!();

        // Benchmark sandbox creation + Wasm initialization
        let start = Instant::now();
        for i in 0..ITERATIONS {
            let mut sandbox = HyperlightSandbox::new(&format!("bench-init-{}", i));
            let _ = sandbox.init_with_wasm(MINIMAL_WASM);
        }
        let init_time = start.elapsed();
        let avg_init = init_time.as_micros() as f64 / ITERATIONS as f64;

        println!("Sandbox creation + Wasm init:");
        println!("  Total: {:?}", init_time);
        println!("  Average: {:.2}µs ({:.2}ms)", avg_init, avg_init / 1000.0);
        println!();

        // Summary
        println!("=== Summary ===");
        println!("| Metric | Time |");
        println!("|--------|------|");
        println!("| Create sandbox | {:.2}µs |", avg_create);
        println!(
            "| Create + init | {:.2}µs ({:.2}ms) |",
            avg_init,
            avg_init / 1000.0
        );
        println!();

        // Compare with other backends
        println!("=== Comparison (from BENCHMARK.md) ===");
        println!("| Backend | Startup |");
        println!("|---------|---------|");
        println!("| Hyperlight | {:.2}ms |", avg_init / 1000.0);
        println!("| Firecracker Daemon | 195ms |");
        println!("| Docker Pool | 250ms |");
        println!("| Apple Containers | 940ms |");
        println!();

        let speedup_fc = 195.0 / (avg_init / 1000.0);
        let speedup_docker = 250.0 / (avg_init / 1000.0);
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
        println!("Note: Wasm execution not yet implemented");
        println!("This test will be expanded once run_wasm() is complete.\n");

        // Create and init sandbox
        let mut sandbox = HyperlightSandbox::new("exec-bench");
        match sandbox.init_with_wasm(MINIMAL_WASM) {
            Ok(_) => println!("Sandbox initialized successfully"),
            Err(e) => println!("Sandbox init error (expected): {}", e),
        }

        // TODO: Benchmark actual Wasm execution once implemented
        // let start = Instant::now();
        // for _ in 0..1000 {
        //     sandbox.run_wasm(&[]).await.unwrap();
        // }
        // let exec_time = start.elapsed();
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
        println!("Hyperlight feature: enabled");
        #[cfg(not(feature = "hyperlight"))]
        println!("Hyperlight feature: disabled (use --features hyperlight)");
    }

    #[cfg(target_os = "macos")]
    println!("Platform: macOS (Hyperlight not supported)");

    #[cfg(target_os = "windows")]
    println!("Platform: Windows (WHP support planned)");
}
