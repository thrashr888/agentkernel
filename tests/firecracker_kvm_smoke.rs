//! Native x86_64 Firecracker smoke coverage.
//!
//! This is deliberately ignored and additionally requires an opt-in environment
//! variable because it starts a real microVM through `/dev/kvm`.

use agentkernel::backend::firecracker::FirecrackerSandbox;
use agentkernel::backend::{Sandbox, SandboxConfig};
use std::path::PathBuf;
use std::process::Command;

fn required_path(name: &str) -> PathBuf {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a prepared smoke-test asset"));
    assert!(path.is_file(), "{name} does not exist: {}", path.display());
    path
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
    let background = sandbox
        .exec(&[
            "sh",
            "-c",
            "nohup sh -c 'while :; do sleep 60; done' >/dev/null 2>&1 & echo $! >/tmp/snapshot-process.pid",
        ])
        .await
        .expect("start process whose memory state must survive");
    assert_eq!(background.exit_code, 0, "{}", background.stderr);

    let snapshot_dir = tempfile::tempdir().expect("create snapshot directory");
    let source_api = sandbox.api_socket_path().to_path_buf();
    let source_vsock = sandbox.vsock_socket_path().to_path_buf();
    let snapshot = sandbox
        .pause_to(snapshot_dir.path())
        .await
        .expect("pause microVM into full-state checkpoint");
    assert_eq!(snapshot.firecracker_version, "1.16.1");
    assert!(!sandbox.is_running());
    assert!(!source_api.exists());
    assert!(!source_vsock.exists());
    for artifact in ["memory.bin", "vmstate.bin", "rootfs.ext4"] {
        assert!(snapshot_dir.path().join(artifact).is_file(), "{artifact}");
    }

    let mut first = FirecrackerSandbox::new(&format!("{name}-fork-a"))
        .expect("create first restored Firecracker sandbox");
    let mut second = FirecrackerSandbox::new(&format!("{name}-fork-b"))
        .expect("create second restored Firecracker sandbox");
    assert_ne!(first.api_socket_path(), second.api_socket_path());
    assert_ne!(first.vsock_socket_path(), second.vsock_socket_path());
    assert_ne!(first.api_socket_path(), source_api);
    assert_ne!(second.vsock_socket_path(), source_vsock);

    let (first_restore, second_restore) = tokio::join!(
        first.restore_from(snapshot_dir.path(), &snapshot),
        second.restore_from(snapshot_dir.path(), &snapshot),
    );
    first_restore.expect("restore and resume first fork");
    second_restore.expect("restore and resume second fork");
    assert!(first.is_running());
    assert!(second.is_running());

    for fork in [&mut first, &mut second] {
        let marker = fork
            .exec(&["cat", "/tmp/snapshot-marker"])
            .await
            .expect("execute after snapshot recovery");
        assert_eq!(marker.exit_code, 0, "{}", marker.stderr);
        assert_eq!(marker.stdout, "survived-restore");
        let process = fork
            .exec(&["sh", "-c", "kill -0 $(cat /tmp/snapshot-process.pid)"])
            .await
            .expect("check restored guest process");
        assert_eq!(process.exit_code, 0, "{}", process.stderr);
    }

    first
        .write_file("/tmp/fork-a-only", b"fork-a")
        .await
        .expect("write first fork marker");
    second
        .write_file("/tmp/fork-b-only", b"fork-b")
        .await
        .expect("write second fork marker");
    let first_isolated = first
        .exec(&[
            "sh",
            "-c",
            "test -f /tmp/fork-a-only && test ! -e /tmp/fork-b-only",
        ])
        .await
        .expect("check first fork disk isolation");
    let second_isolated = second
        .exec(&[
            "sh",
            "-c",
            "test -f /tmp/fork-b-only && test ! -e /tmp/fork-a-only",
        ])
        .await
        .expect("check second fork disk isolation");
    assert_eq!(first_isolated.exit_code, 0, "{}", first_isolated.stderr);
    assert_eq!(second_isolated.exit_code, 0, "{}", second_isolated.stderr);

    first.stop().await.expect("stop first restored microVM");
    second.stop().await.expect("stop second restored microVM");
}
