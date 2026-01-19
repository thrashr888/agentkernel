//! Stress test: Spin up 100 microVMs, run echo, verify output, shut down.
//!
//! This test validates that agentkernel can handle rapid VM lifecycle operations
//! with minimal latency. Target: <125ms per VM boot, <10s total for 100 VMs.
//!
//! Run with: cargo test --test stress_test -- --nocapture
//!
//! Requirements:
//!   - Linux with KVM (/dev/kvm accessible)
//!   - Firecracker binary in PATH or FIRECRACKER_BIN env var
//!   - Built kernel at images/kernel/vmlinux-*
//!   - Built rootfs at images/rootfs/base.ext4

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const VM_COUNT: usize = 100;
const EXPECTED_OUTPUT: &str = "hello";
const MAX_TOTAL_TIME: Duration = Duration::from_secs(30);
const MAX_BOOT_TIME: Duration = Duration::from_millis(500); // 500ms max per VM with overhead

/// Results from a single VM test
#[derive(Debug)]
#[allow(dead_code)] // Fields used in Debug output and future implementation
struct VMTestResult {
    vm_id: usize,
    boot_time: Duration,
    exec_time: Duration,
    output_correct: bool,
    shutdown_time: Duration,
    error: Option<String>,
}

/// Aggregate results from the stress test
#[derive(Debug)]
struct StressTestResults {
    total_time: Duration,
    successful: usize,
    failed: usize,
    avg_boot_time: Duration,
    avg_exec_time: Duration,
    avg_shutdown_time: Duration,
    max_boot_time: Duration,
    errors: Vec<String>,
}

#[tokio::test]
#[ignore] // Remove this once Firecracker VMM is implemented
async fn test_100_vms_parallel() {
    println!("\n=== Agentkernel Stress Test: 100 VMs ===\n");

    let start = Instant::now();
    let success_count = Arc::new(AtomicUsize::new(0));
    let fail_count = Arc::new(AtomicUsize::new(0));

    // Spawn 100 VM tasks concurrently
    let mut handles = Vec::with_capacity(VM_COUNT);

    for i in 0..VM_COUNT {
        let success = Arc::clone(&success_count);
        let fail = Arc::clone(&fail_count);

        let handle = tokio::spawn(async move {
            let result = run_single_vm_test(i).await;

            if result.error.is_none() && result.output_correct {
                success.fetch_add(1, Ordering::SeqCst);
            } else {
                fail.fetch_add(1, Ordering::SeqCst);
            }

            result
        });

        handles.push(handle);
    }

    // Wait for all VMs to complete
    let mut results = Vec::with_capacity(VM_COUNT);
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => {
                fail_count.fetch_add(1, Ordering::SeqCst);
                results.push(VMTestResult {
                    vm_id: 0,
                    boot_time: Duration::ZERO,
                    exec_time: Duration::ZERO,
                    output_correct: false,
                    shutdown_time: Duration::ZERO,
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
        "Some VMs failed: {} failures out of {}",
        stats.failed,
        VM_COUNT
    );

    assert!(
        stats.total_time < MAX_TOTAL_TIME,
        "Total time {} exceeded maximum {}",
        stats.total_time.as_secs_f64(),
        MAX_TOTAL_TIME.as_secs_f64()
    );

    assert!(
        stats.max_boot_time < MAX_BOOT_TIME,
        "Max boot time {:?} exceeded maximum {:?}",
        stats.max_boot_time,
        MAX_BOOT_TIME
    );

    println!("\n=== STRESS TEST PASSED ===\n");
}

async fn run_single_vm_test(vm_id: usize) -> VMTestResult {
    let vm_name = format!("stress-test-{}", vm_id);

    // TODO: Replace with actual Firecracker VMM calls once implemented
    // For now, this is a placeholder that simulates the expected behavior

    let boot_start = Instant::now();

    // 1. Create and boot VM
    let boot_result = create_and_boot_vm(&vm_name).await;
    let boot_time = boot_start.elapsed();

    if let Err(e) = boot_result {
        return VMTestResult {
            vm_id,
            boot_time,
            exec_time: Duration::ZERO,
            output_correct: false,
            shutdown_time: Duration::ZERO,
            error: Some(format!("Boot failed: {}", e)),
        };
    }

    // 2. Execute echo command
    let exec_start = Instant::now();
    let exec_result = exec_in_vm(&vm_name, &["echo", EXPECTED_OUTPUT]).await;
    let exec_time = exec_start.elapsed();

    let output_correct = match &exec_result {
        Ok(output) => output.trim() == EXPECTED_OUTPUT,
        Err(_) => false,
    };

    let exec_error = exec_result.err().map(|e| format!("Exec failed: {}", e));

    // 3. Shutdown VM
    let shutdown_start = Instant::now();
    let shutdown_result = shutdown_vm(&vm_name).await;
    let shutdown_time = shutdown_start.elapsed();

    let error = exec_error.or_else(|| {
        shutdown_result
            .err()
            .map(|e| format!("Shutdown failed: {}", e))
    });

    VMTestResult {
        vm_id,
        boot_time,
        exec_time,
        output_correct,
        shutdown_time,
        error,
    }
}

// Placeholder functions - to be replaced with actual Firecracker VMM implementation

async fn create_and_boot_vm(_name: &str) -> Result<(), String> {
    // TODO: Implement with Firecracker API
    // 1. Create VM configuration
    // 2. Set kernel and rootfs paths
    // 3. Configure vsock for communication
    // 4. Start VM
    // 5. Wait for guest agent to be ready

    Err("Firecracker VMM not yet implemented".to_string())
}

async fn exec_in_vm(_name: &str, _cmd: &[&str]) -> Result<String, String> {
    // TODO: Implement with vsock communication to guest agent
    // 1. Connect to VM's vsock
    // 2. Send command to guest agent
    // 3. Wait for response
    // 4. Return stdout

    Err("Firecracker VMM not yet implemented".to_string())
}

async fn shutdown_vm(_name: &str) -> Result<(), String> {
    // TODO: Implement with Firecracker API
    // 1. Send shutdown signal via vsock (graceful)
    // 2. Or send InstanceActionInfo::SendCtrlAltDel
    // 3. Wait for VM to terminate
    // 4. Clean up resources

    Err("Firecracker VMM not yet implemented".to_string())
}

fn calculate_stats(results: &[VMTestResult], total_time: Duration) -> StressTestResults {
    let successful = results.iter().filter(|r| r.error.is_none()).count();
    let failed = results.len() - successful;

    let boot_times: Vec<_> = results.iter().map(|r| r.boot_time).collect();
    let exec_times: Vec<_> = results.iter().map(|r| r.exec_time).collect();
    let shutdown_times: Vec<_> = results.iter().map(|r| r.shutdown_time).collect();

    let avg_boot = avg_duration(&boot_times);
    let avg_exec = avg_duration(&exec_times);
    let avg_shutdown = avg_duration(&shutdown_times);
    let max_boot = boot_times.iter().max().copied().unwrap_or(Duration::ZERO);

    let errors: Vec<_> = results.iter().filter_map(|r| r.error.clone()).collect();

    StressTestResults {
        total_time,
        successful,
        failed,
        avg_boot_time: avg_boot,
        avg_exec_time: avg_exec,
        avg_shutdown_time: avg_shutdown,
        max_boot_time: max_boot,
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
    println!("Results:");
    println!("  Total time:       {:?}", stats.total_time);
    println!("  Successful:       {}/{}", stats.successful, VM_COUNT);
    println!("  Failed:           {}", stats.failed);
    println!("  Avg boot time:    {:?}", stats.avg_boot_time);
    println!("  Avg exec time:    {:?}", stats.avg_exec_time);
    println!("  Avg shutdown:     {:?}", stats.avg_shutdown_time);
    println!("  Max boot time:    {:?}", stats.max_boot_time);

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
