//! Native x86_64 Firecracker smoke coverage.
//!
//! This is deliberately ignored and additionally requires an opt-in environment
//! variable because it starts a real microVM through `/dev/kvm`.

use agentkernel::backend::firecracker::FirecrackerSandbox;
use agentkernel::backend::{Sandbox, SandboxConfig};
use agentkernel::firecracker_client::{
    FirecrackerClient, SnapshotCreateParams, SnapshotLoadParams, VsockOverride,
};
use agentkernel::vsock::VsockClient;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn required_path(name: &str) -> PathBuf {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a prepared smoke-test asset"));
    assert!(path.is_file(), "{name} does not exist: {}", path.display());
    path
}

async fn wait_for_path(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {}", path.display());
}

async fn wait_for_vsock(path: &Path) -> VsockClient {
    let client = VsockClient::for_firecracker(path.to_path_buf());
    for _ in 0..100 {
        if client.ping().await.unwrap_or(false) {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for restored vsock {}", path.display());
}

#[tokio::test]
#[ignore = "requires an explicitly approved native x86_64 KVM runner"]
async fn firecracker_lifecycle_exec_network_vsock_snapshot_recovery() {
    assert_eq!(std::env::consts::OS, "linux");
    assert_eq!(std::env::consts::ARCH, "x86_64");
    assert_eq!(std::env::var("AGENTKERNEL_KVM_SMOKE").as_deref(), Ok("1"));

    let firecracker = required_path("FIRECRACKER_BIN");
    let kernel = required_path("AGENTKERNEL_KVM_KERNEL");
    let rootfs = required_path("AGENTKERNEL_KVM_ROOTFS");
    let version = Command::new(&firecracker)
        .arg("--version")
        .output()
        .expect("run firecracker --version");
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("v1.16.1"));

    let name = format!("kvm-smoke-{}", std::process::id());
    let mut sandbox = FirecrackerSandbox::new(&name)
        .expect("create Firecracker sandbox")
        .with_kernel(kernel)
        .with_rootfs(rootfs);
    sandbox
        .start(&SandboxConfig::default())
        .await
        .expect("start Firecracker microVM");
    assert!(sandbox.is_running());

    let exec = sandbox
        .exec(&["sh", "-c", "printf agentkernel-vsock"])
        .await
        .expect("execute over vsock");
    assert_eq!(exec.exit_code, 0, "{}", exec.stderr);
    assert_eq!(exec.stdout, "agentkernel-vsock");

    let network = sandbox
        .exec(&["sh", "-c", "ip -o link show lo && ping -c 1 -W 1 127.0.0.1"])
        .await
        .expect("inspect guest network stack over vsock");
    assert_eq!(network.exit_code, 0, "{}", network.stderr);
    assert!(network.stdout.contains("LOOPBACK"));

    sandbox
        .write_file("/tmp/snapshot-marker", b"survived-restore")
        .await
        .expect("write snapshot marker over vsock");

    let snapshot_dir = tempfile::tempdir().expect("create snapshot directory");
    let mem_path = snapshot_dir.path().join("microvm.mem");
    let state_path = snapshot_dir.path().join("microvm.state");
    let original_socket = PathBuf::from(format!("/tmp/agentkernel-{name}.sock"));
    let original_rootfs = PathBuf::from(format!("/tmp/agentkernel-{name}-rootfs.ext4"));
    let rootfs_backup = snapshot_dir.path().join("rootfs.ext4");
    let client = FirecrackerClient::new(&original_socket);
    client.pause().await.expect("pause microVM for snapshot");
    client
        .create_snapshot(&SnapshotCreateParams {
            mem_file_path: mem_path.to_string_lossy().into_owned(),
            snapshot_path: state_path.to_string_lossy().into_owned(),
            snapshot_type: "Full".to_string(),
        })
        .await
        .expect("create full Firecracker snapshot");
    std::fs::copy(&original_rootfs, &rootfs_backup).expect("preserve snapshot rootfs");
    sandbox.stop().await.expect("stop original microVM");
    std::fs::copy(&rootfs_backup, &original_rootfs).expect("restore snapshot rootfs path");

    let restored_socket = snapshot_dir.path().join("restored-api.sock");
    let restored_vsock = snapshot_dir.path().join("restored-vsock.sock");
    let mut restored = Command::new(&firecracker)
        .arg("--api-sock")
        .arg(&restored_socket)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Firecracker for snapshot recovery");
    wait_for_path(&restored_socket).await;

    let restored_client = FirecrackerClient::new(&restored_socket);
    restored_client
        .load_snapshot(&SnapshotLoadParams {
            mem_file_path: mem_path.to_string_lossy().into_owned(),
            snapshot_path: state_path.to_string_lossy().into_owned(),
            resume_vm: true,
            vsock_override: VsockOverride {
                uds_path: restored_vsock.to_string_lossy().into_owned(),
            },
        })
        .await
        .expect("load and resume Firecracker snapshot");

    let restored_vsock_client = wait_for_vsock(&restored_vsock).await;
    let marker = restored_vsock_client
        .run_command(&["cat".to_string(), "/tmp/snapshot-marker".to_string()])
        .await
        .expect("execute after snapshot recovery");
    assert_eq!(marker.exit_code, 0, "{}", marker.stderr);
    assert_eq!(marker.stdout, "survived-restore");

    let _ = restored_client.send_ctrl_alt_del().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = restored.kill();
    let _ = restored.wait();
    let _ = std::fs::remove_file(original_rootfs);
}
