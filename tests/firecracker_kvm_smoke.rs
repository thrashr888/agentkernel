//! Native x86_64 Firecracker lifecycle gate.
//!
//! This is deliberately ignored and additionally requires an opt-in
//! environment variable because it starts real microVMs through `/dev/kvm`.
//! Unlike the cheap backend smoke, this gate exercises persisted `VmManager`
//! transactions and the daemon-owned CLI, HTTP, and MCP control paths.

use agentkernel::backend::firecracker::FirecrackerSandbox;
use agentkernel::backend::{BackendType, Sandbox, SandboxConfig};
use agentkernel::full_state::{FORK_SECURITY_WARNING, FullStateCheckpointStore};
use agentkernel::vmm::VmManager;
use anyhow::{Context, Result, bail};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command as TokioCommand;
use tokio::time::sleep;

fn required_path(name: &str) -> PathBuf {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a prepared smoke-test asset"));
    assert!(path.is_file(), "{name} does not exist: {}", path.display());
    path
}

fn agentkernel_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_agentkernel")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/agentkernel")
        })
}

fn native_prerequisites() -> (PathBuf, PathBuf, PathBuf) {
    assert_eq!(
        std::env::consts::OS,
        "linux",
        "native KVM gate requires Linux; this is not a valid runtime result"
    );
    assert_eq!(
        std::env::consts::ARCH,
        "x86_64",
        "native KVM gate requires x86_64; this is not a valid runtime result"
    );
    assert_eq!(
        std::env::var("AGENTKERNEL_KVM_SMOKE").as_deref(),
        Ok("1"),
        "set AGENTKERNEL_KVM_SMOKE=1 to run the destructive native gate"
    );

    let kvm = Path::new("/dev/kvm");
    assert!(
        kvm.exists(),
        "/dev/kvm is unavailable; no native result recorded"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(kvm)
            .expect("inspect /dev/kvm")
            .permissions()
            .mode();
        assert!(mode & 0o400 != 0, "/dev/kvm is not readable by this user");
        assert!(mode & 0o200 != 0, "/dev/kvm is not writable by this user");
    }

    let firecracker = required_path("FIRECRACKER_BIN");
    let kernel = required_path("AGENTKERNEL_KVM_KERNEL");
    let rootfs = required_path("AGENTKERNEL_KVM_ROOTFS");
    let version = Command::new(&firecracker)
        .arg("--version")
        .output()
        .expect("run firecracker --version");
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).contains("v1.16.1"),
        "native gate requires Firecracker v1.16.1, got: {}",
        String::from_utf8_lossy(&version.stdout)
    );
    (firecracker, kernel, rootfs)
}

/// Kill the API server even if an assertion aborts the test. The server owns
/// the Firecracker children, so dropping this guard is part of safe cleanup.
struct ServerGuard {
    child: Child,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Port for the test's private API server. `AGENTKERNEL_KVM_SMOKE_PORT`
/// pins one; otherwise the kernel picks a free loopback port. Either way the
/// port must be free right now, so the CLI and HTTP calls below can never
/// reach an unrelated agentkernel server that already listens on the host.
fn smoke_server_port() -> Result<u16> {
    let requested = match std::env::var("AGENTKERNEL_KVM_SMOKE_PORT") {
        Ok(value) => {
            let port: u16 = value
                .parse()
                .with_context(|| format!("AGENTKERNEL_KVM_SMOKE_PORT is not a port: {value}"))?;
            if port == 0 {
                bail!("AGENTKERNEL_KVM_SMOKE_PORT must be a fixed non-zero port");
            }
            port
        }
        Err(_) => 0,
    };
    let probe = std::net::TcpListener::bind(("127.0.0.1", requested)).with_context(|| {
        format!(
            "KVM smoke API port {requested} is already in use; set AGENTKERNEL_KVM_SMOKE_PORT to a free port"
        )
    })?;
    Ok(probe.local_addr()?.port())
}

async fn start_server(port: u16) -> Result<ServerGuard> {
    let mut child = Command::new(agentkernel_binary())
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start agentkernel API server for native lifecycle routing")?;
    match wait_for_owned_health(&mut child, port, 100).await {
        Ok(()) => Ok(ServerGuard { child }),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

/// Who holds the listener(s) that would answer `127.0.0.1:port`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum ListenerOwnership {
    /// Nothing listens yet: normal while the server is still starting.
    Absent,
    /// Every listener that can accept `127.0.0.1:port` belongs to the child.
    Child,
    /// At least one such listener belongs to another process.
    Foreign,
}

/// Wait until our own server child answers health on `port`, failing closed.
///
/// `smoke_server_port()` has to release its probe before the server binds,
/// and `serve` does scheduler and worker setup before it binds. In that gap
/// another process could take the port and answer `/status`. So before any
/// request is sent, the listener behind `127.0.0.1:port` must be a socket in
/// the child's own fd table, and that is checked again after health
/// succeeds. A foreign listener aborts at once; only "nothing listens yet" is
/// waited out.
///
/// This guards against an accidental port collision. It does not defend
/// against a hostile local process that replaces the child's PID or socket.
async fn wait_for_owned_health(child: &mut Child, port: u16, attempts: usize) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .context("build loopback HTTP client")?;
    let url = format!("http://127.0.0.1:{port}/status");
    for _ in 0..attempts {
        ensure_child_running(child)?;
        match listener_ownership(child.id(), port)? {
            ListenerOwnership::Absent => {
                sleep(Duration::from_millis(100)).await;
                continue;
            }
            ListenerOwnership::Foreign => {
                bail!(
                    "another process is listening on 127.0.0.1:{port}; refusing to send lifecycle requests to it"
                );
            }
            ListenerOwnership::Child => {}
        }
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            // Readiness boundary: the child must still be alive and still
            // the only owner of the listener that answered.
            ensure_child_running(child)?;
            if listener_ownership(child.id(), port)? != ListenerOwnership::Child {
                bail!("listener on 127.0.0.1:{port} changed owner during the health check");
            }
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }
    bail!("agentkernel API server did not become healthy at {url}")
}

fn ensure_child_running(child: &mut Child) -> Result<()> {
    if let Some(status) = child.try_wait().context("inspect API server")? {
        bail!("agentkernel API server exited before becoming healthy: {status}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn listener_ownership(pid: u32, port: u16) -> Result<ListenerOwnership> {
    let tcp = fs::read_to_string("/proc/net/tcp").context("read /proc/net/tcp")?;
    // tcp6 is absent when IPv6 is disabled; then no IPv6 listener can exist.
    let tcp6 = fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    let listeners = loopback_v4_listener_inodes(&tcp, &tcp6, port);
    if listeners.is_empty() {
        return Ok(ListenerOwnership::Absent);
    }
    let owned = socket_inodes_of(pid)?;
    Ok(if listeners.iter().all(|inode| owned.contains(inode)) {
        ListenerOwnership::Child
    } else {
        ListenerOwnership::Foreign
    })
}

#[cfg(not(target_os = "linux"))]
fn listener_ownership(_pid: u32, _port: u16) -> Result<ListenerOwnership> {
    bail!("listener ownership checks need Linux /proc; the native KVM gate is Linux-only")
}

/// Socket inodes of LISTEN entries that can accept a connection to
/// `127.0.0.1:port`. On IPv4 that is a bind to 127.0.0.1 or 0.0.0.0. On IPv6
/// it is `::`, which may be dual-stack (procfs does not expose IPV6_V6ONLY, so
/// it counts), or a v4-mapped `::ffff:127.0.0.1` / `::ffff:0.0.0.0`. Other
/// addresses such as `::1` cannot receive IPv4 traffic and are ignored.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn loopback_v4_listener_inodes(tcp: &str, tcp6: &str, port: u16) -> Vec<u64> {
    const LISTEN: &str = "0A";
    let port_hex = format!("{port:04X}");
    let mut inodes = Vec::new();
    for (table, accepts) in [
        (tcp, &["0100007F", "00000000"][..]),
        (
            tcp6,
            &[
                "00000000000000000000000000000000",
                "0000000000000000FFFF00000100007F",
                "0000000000000000FFFF000000000000",
            ][..],
        ),
    ] {
        for line in table.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != LISTEN {
                continue;
            }
            let Some((address, local_port)) = fields[1].split_once(':') else {
                continue;
            };
            if !local_port.eq_ignore_ascii_case(&port_hex)
                || !accepts.iter().any(|a| a.eq_ignore_ascii_case(address))
            {
                continue;
            }
            if let Ok(inode) = fields[9].parse::<u64>() {
                inodes.push(inode);
            }
        }
    }
    inodes
}

#[cfg(target_os = "linux")]
fn socket_inodes_of(pid: u32) -> Result<std::collections::HashSet<u64>> {
    let mut inodes = std::collections::HashSet::new();
    let dir = format!("/proc/{pid}/fd");
    for entry in fs::read_dir(&dir).with_context(|| format!("read {dir}"))? {
        // An fd can close between listing and readlink; skip it.
        let Ok(target) = fs::read_link(entry?.path()) else {
            continue;
        };
        if let Some(inode) = target
            .to_str()
            .and_then(|t| t.strip_prefix("socket:["))
            .and_then(|t| t.strip_suffix(']'))
            .and_then(|t| t.parse().ok())
        {
            inodes.insert(inode);
        }
    }
    Ok(inodes)
}

async fn run_cli(args: &[&str]) -> Result<String> {
    let output = TokioCommand::new(agentkernel_binary())
        .args(args)
        .output()
        .await
        .with_context(|| format!("run agentkernel {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!(
            "agentkernel {} failed ({}):\nstdout: {}\nstderr: {}",
            args.join(" "),
            output.status,
            stdout,
            stderr
        );
    }
    Ok(format!("{stdout}{stderr}"))
}

async fn run_mcp_lifecycle(source: &str, child: &str) -> Result<String> {
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#
            .to_string(),
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#.to_string(),
        format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"permission_grant","arguments":{{"kind":"sandbox_create","granted":true,"scope":"session","sandbox":"{child}"}}}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"sandbox_fork","arguments":{{"name":"{source}","as_name":"{child}"}}}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"sandbox_resume","arguments":{{"name":"{source}"}}}}}}"#
        ),
    ]
    .join("\n")
        + "\n";

    let mut process = TokioCommand::new(agentkernel_binary())
        .arg("mcp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("start MCP lifecycle client")?;
    process
        .stdin
        .take()
        .context("open MCP stdin")?
        .write_all(input.as_bytes())
        .await
        .context("send MCP lifecycle calls")?;
    let output = process
        .wait_with_output()
        .await
        .context("wait for MCP lifecycle client")?;
    if !output.status.success() {
        bail!("MCP lifecycle client failed: {}", output.status);
    }
    let response = String::from_utf8(output.stdout).context("MCP output is not UTF-8")?;
    for line in response.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("invalid MCP JSON-RPC response: {line}"))?;
        assert!(
            value.get("error").is_none(),
            "MCP lifecycle call returned an error: {value}"
        );
    }
    assert!(
        response.contains("forked from"),
        "MCP fork response missing: {response}"
    );
    assert!(
        response.contains("resumed"),
        "MCP resume response missing: {response}"
    );
    Ok(response)
}

fn set_artifact_writable(path: &Path, writable: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("inspect checkpoint artifact")
            .permissions();
        permissions.set_mode(if writable { 0o600 } else { 0o400 });
        fs::set_permissions(path, permissions).expect("set checkpoint artifact permissions");
    }
}

async fn manager_lifecycle_and_corrupt_recovery() -> Result<()> {
    let mut manager = VmManager::with_backend(Some(BackendType::Firecracker))?;
    manager
        .create("manager-source", "alpine:3.24", 1, 256)
        .await?;
    manager.start("manager-source").await?;

    let network = manager
        .exec_cmd(
            "manager-source",
            &[
                "sh".to_string(),
                "-c".to_string(),
                "ip -o link show lo && ping -c 1 -W 1 127.0.0.1".to_string(),
            ],
        )
        .await?;
    assert!(
        network.contains("LOOPBACK"),
        "guest network check: {network}"
    );
    manager
        .write_file("manager-source", "/tmp/native-marker", b"manager-state")
        .await?;
    manager
        .exec_cmd(
            "manager-source",
            &[
                "sh".to_string(),
                "-c".to_string(),
                "nohup sh -c 'while :; do sleep 60; done' >/dev/null 2>&1 & echo $! >/tmp/native-process.pid"
                    .to_string(),
            ],
        )
        .await?;

    let data_dir = manager.get_data_dir().to_path_buf();
    let checkpoint = manager.pause("manager-source").await?;
    let checkpoint_id = checkpoint.id.clone();
    let checkpoint_dir = data_dir.join("full-state-checkpoints").join(&checkpoint_id);
    assert!(!manager.is_running("manager-source"));
    assert_eq!(checkpoint.format_version, 2);
    assert_eq!(checkpoint.source_sandbox, "manager-source");
    assert!(!checkpoint.source_sandbox_uuid.is_empty());
    assert!(checkpoint_dir.join("manifest.json").is_file());
    assert!(checkpoint_dir.join("memory.bin").is_file());
    assert!(checkpoint_dir.join("vmstate.bin").is_file());
    assert!(checkpoint_dir.join("rootfs.ext4").is_file());
    assert!(
        FullStateCheckpointStore::new(&data_dir)?
            .staging_entries()?
            .is_empty()
    );

    manager
        .fork_sandbox("manager-source", "manager-fork-a")
        .await?;
    manager
        .fork_sandbox("manager-source", "manager-fork-b")
        .await?;
    assert!(!manager.is_running("manager-source"));
    let marker = manager
        .exec_cmd(
            "manager-fork-a",
            &["cat".to_string(), "/tmp/native-marker".to_string()],
        )
        .await?;
    assert_eq!(marker, "manager-state");
    let process = manager
        .exec_cmd(
            "manager-fork-a",
            &[
                "sh".to_string(),
                "-c".to_string(),
                "kill -0 $(cat /tmp/native-process.pid)".to_string(),
            ],
        )
        .await?;
    assert!(process.is_empty(), "restored process check: {process}");

    let random_a = manager
        .exec_cmd(
            "manager-fork-a",
            &[
                "od".to_string(),
                "-An".to_string(),
                "-tx1".to_string(),
                "-N16".to_string(),
                "/dev/urandom".to_string(),
            ],
        )
        .await?;
    let random_b = manager
        .exec_cmd(
            "manager-fork-b",
            &[
                "od".to_string(),
                "-An".to_string(),
                "-tx1".to_string(),
                "-N16".to_string(),
                "/dev/urandom".to_string(),
            ],
        )
        .await?;
    assert!(!random_a.trim().is_empty());
    assert!(!random_b.trim().is_empty());
    assert_ne!(random_a, random_b, "fork RNG streams did not diverge");

    manager
        .write_file("manager-fork-a", "/tmp/fork-a-only", b"a")
        .await?;
    manager
        .write_file("manager-fork-b", "/tmp/fork-b-only", b"b")
        .await?;
    assert!(
        manager
            .exec_cmd(
                "manager-fork-a",
                &[
                    "sh".to_string(),
                    "-c".to_string(),
                    "test -f /tmp/fork-a-only && test ! -e /tmp/fork-b-only".to_string(),
                ],
            )
            .await?
            .is_empty()
    );
    assert!(
        manager
            .exec_cmd(
                "manager-fork-b",
                &[
                    "sh".to_string(),
                    "-c".to_string(),
                    "test -f /tmp/fork-b-only && test ! -e /tmp/fork-a-only".to_string(),
                ],
            )
            .await?
            .is_empty()
    );

    // Keep fork references alive while checking deferred checkpoint cleanup.
    manager.resume("manager-source").await?;
    assert!(manager.is_running("manager-source"));
    assert_eq!(
        manager
            .exec_cmd(
                "manager-source",
                &["cat".to_string(), "/tmp/native-marker".to_string()],
            )
            .await?,
        "manager-state"
    );
    assert!(
        manager
            .get_state("manager-source")
            .expect("manager state")
            .full_state_cleanup_pending
            .contains(&checkpoint_id)
    );
    assert!(
        checkpoint_dir.is_dir(),
        "live forks must retain the checkpoint"
    );
    manager.remove("manager-fork-a").await?;
    manager.remove("manager-fork-b").await?;
    manager.remove("manager-source").await?;
    assert!(
        !checkpoint_dir.exists(),
        "checkpoint tombstone was not cleaned"
    );
    assert!(
        !data_dir
            .join("sandboxes")
            .join("manager-source.json")
            .exists()
    );

    // A digest failure must be rejected before a restore VMM is started. The
    // paused state and checkpoint remain retryable until explicit removal.
    manager
        .create("corrupt-source", "alpine:3.24", 1, 256)
        .await?;
    manager.start("corrupt-source").await?;
    let corrupt_checkpoint = manager.pause("corrupt-source").await?;
    let corrupt_dir = data_dir
        .join("full-state-checkpoints")
        .join(&corrupt_checkpoint.id);
    let memory = corrupt_dir.join("memory.bin");
    set_artifact_writable(&memory, true);
    let mut file = OpenOptions::new().write(true).open(&memory)?;
    use std::io::{Seek, SeekFrom, Write};
    file.seek(SeekFrom::Start(0))?;
    file.write_all(b"x")?;
    file.sync_all()?;
    drop(file);
    set_artifact_writable(&memory, false);
    let error = manager.resume("corrupt-source").await.unwrap_err();
    assert!(
        error.to_string().contains("digest"),
        "unexpected corrupt recovery error: {error:#}"
    );
    assert!(!manager.is_running("corrupt-source"));
    assert!(
        manager
            .get_state("corrupt-source")
            .and_then(|state| state.paused_at.as_ref())
            .is_some()
    );
    assert!(
        corrupt_dir.exists(),
        "corrupt checkpoint was not retained for recovery"
    );
    manager.remove("corrupt-source").await?;
    assert!(
        !corrupt_dir.exists(),
        "corrupt checkpoint tombstone was not removed"
    );
    Ok(())
}

/// Keep the original backend-level signal alongside the broader manager gate:
/// it isolates Firecracker API/vsock/snapshot behavior when a manager failure
/// needs to be distinguished from a backend failure.
async fn direct_backend_lifecycle(kernel: PathBuf, rootfs: PathBuf) -> Result<()> {
    let name = format!("kvm-direct-{}", std::process::id());
    let mut source = FirecrackerSandbox::new(&name)?
        .with_kernel(kernel)
        .with_rootfs(rootfs);
    source.start(&SandboxConfig::default()).await?;
    let exec = source
        .exec(&["sh", "-c", "printf agentkernel-vsock"])
        .await?;
    assert_eq!(exec.exit_code, 0, "{}", exec.stderr);
    assert_eq!(exec.stdout, "agentkernel-vsock");
    let network = source
        .exec(&["sh", "-c", "ip -o link show lo && ping -c 1 -W 1 127.0.0.1"])
        .await?;
    assert_eq!(network.exit_code, 0, "{}", network.stderr);
    assert!(network.stdout.contains("LOOPBACK"));
    source
        .write_file("/tmp/direct-marker", b"direct-state")
        .await?;

    let checkpoint_dir = tempfile::tempdir()?;
    let snapshot = source.pause_to(checkpoint_dir.path()).await?;
    assert_eq!(snapshot.firecracker_version, "1.16.1");
    assert!(!source.is_running());
    for artifact in ["memory.bin", "vmstate.bin", "rootfs.ext4"] {
        assert!(checkpoint_dir.path().join(artifact).is_file(), "{artifact}");
    }

    let mut fork = FirecrackerSandbox::new(&format!("{name}-fork"))?;
    assert_ne!(source.api_socket_path(), fork.api_socket_path());
    fork.restore_from(checkpoint_dir.path(), &snapshot).await?;
    let marker = fork.exec(&["cat", "/tmp/direct-marker"]).await?;
    assert_eq!(marker.exit_code, 0, "{}", marker.stderr);
    assert_eq!(marker.stdout, "direct-state");
    fork.stop().await?;
    Ok(())
}

async fn routed_lifecycle() -> Result<()> {
    let port = smoke_server_port()?;
    eprintln!("[kvm-smoke] private API server on 127.0.0.1:{port}");
    unsafe { std::env::set_var("AGENTKERNEL_PORT", port.to_string()) };
    let _server = start_server(port).await?;

    let mut publisher = VmManager::with_backend(Some(BackendType::Firecracker))?;
    publisher
        .create("routed-source", "alpine:3.24", 1, 256)
        .await?;
    drop(publisher);

    let start = run_cli(&["sandbox", "start", "routed-source"]).await?;
    assert!(start.contains("Starting Firecracker sandbox via API server"));
    let exec = run_cli(&["exec", "routed-source", "sh", "-c", "printf routed-state"]).await?;
    assert!(exec.contains("routed-state"));
    run_cli(&["sandbox", "pause", "routed-source"]).await?;

    let client = reqwest::Client::builder().no_proxy().build()?;
    let http_fork = client
        .post(format!(
            "http://127.0.0.1:{port}/sandboxes/routed-source/fork"
        ))
        .json(&serde_json::json!({"as_name": "http-fork"}))
        .send()
        .await?;
    let http_status = http_fork.status();
    let http_body = http_fork.text().await?;
    assert_eq!(
        http_status,
        reqwest::StatusCode::CREATED,
        "HTTP fork: {http_body}"
    );

    let mcp_response = run_mcp_lifecycle("routed-source", "mcp-fork").await?;
    assert!(mcp_response.contains(FORK_SECURITY_WARNING));

    // MCP resumed the source; exercise the direct HTTP pause endpoint, then
    // the CLI resume path. Each mutation still has one authoritative owner.
    let http_pause = client
        .post(format!(
            "http://127.0.0.1:{port}/sandboxes/routed-source/pause"
        ))
        .send()
        .await?;
    let http_pause_status = http_pause.status();
    let http_pause_body = http_pause.text().await?;
    assert_eq!(
        http_pause_status,
        reqwest::StatusCode::OK,
        "HTTP pause: {http_pause_body}"
    );
    run_cli(&["sandbox", "resume", "routed-source"]).await?;
    let resumed = run_cli(&["exec", "routed-source", "sh", "-c", "printf routed-resumed"]).await?;
    assert!(resumed.contains("routed-resumed"));

    let state = VmManager::with_backend(Some(BackendType::Firecracker))?;
    assert_eq!(
        state
            .get_state("routed-source")
            .map(|s| s.paused_at.is_none()),
        Some(true)
    );
    assert_eq!(
        state
            .get_state("http-fork")
            .map(|s| s.forked_from.as_deref()),
        Some(Some("routed-source"))
    );
    assert_eq!(
        state
            .get_state("mcp-fork")
            .map(|s| s.forked_from.as_deref()),
        Some(Some("routed-source"))
    );

    // Cleanup uses the same server-owned removal route and leaves audit
    // tombstones while deleting durable sandbox/checkpoint state.
    run_cli(&["sandbox", "remove", "http-fork"]).await?;
    run_cli(&["sandbox", "remove", "mcp-fork"]).await?;
    run_cli(&["sandbox", "remove", "routed-source"]).await?;
    let audit = client
        .get(format!("http://127.0.0.1:{port}/audit"))
        .send()
        .await?
        .text()
        .await?;
    assert!(
        audit.contains("routed-source"),
        "audit tombstone missing: {audit}"
    );
    let final_state = VmManager::with_backend(Some(BackendType::Firecracker))?;
    assert!(!final_state.exists("routed-source"));
    assert!(!final_state.exists("http-fork"));
    assert!(!final_state.exists("mcp-fork"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly approved native x86_64 KVM runner"]
async fn firecracker_native_manager_routes_recovery_and_clone_safety() {
    let (firecracker, kernel, rootfs) = native_prerequisites();
    let home = tempfile::tempdir().expect("create isolated HOME");
    // CLI readiness checks inspect the installation tree, not the backend's
    // explicit KVM asset overrides. Stage the same verified assets privately.
    let images = home.path().join(".local/share/agentkernel/images");
    let bin = home.path().join(".local/share/agentkernel/bin");
    fs::create_dir_all(&bin).expect("create private binary directory");
    fs::copy(&firecracker, bin.join("firecracker")).expect("stage private Firecracker");
    fs::create_dir_all(images.join("kernel")).expect("create private kernel directory");
    fs::create_dir_all(images.join("rootfs")).expect("create private rootfs directory");
    fs::copy(&kernel, images.join("kernel/vmlinux-6.18.45-agentkernel"))
        .expect("stage private verified kernel");
    fs::copy(&rootfs, images.join("rootfs/base.ext4")).expect("stage private verified rootfs");
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("AGENTKERNEL_KVM_SMOKE", "1");
    }

    direct_backend_lifecycle(kernel, rootfs)
        .await
        .expect("direct Firecracker backend lifecycle gate");
    manager_lifecycle_and_corrupt_recovery()
        .await
        .expect("VmManager native lifecycle gate");
    routed_lifecycle()
        .await
        .expect("CLI/HTTP/MCP native routing gate");
}

// Regression coverage for the listener-ownership check. These run in normal
// `cargo test` on Linux; they need no KVM.

#[test]
fn loopback_listener_inodes_cover_v4_and_dual_stack_only() {
    let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1
   1: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 112 1
   2: 0200007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 113 1
   3: 0100007F:1F91 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 114 1
   4: 0100007F:1F90 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 115 1
";
    let tcp6 = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 211 1
   1: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 212 1
   2: 0000000000000000FFFF00000100007F:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 213 1
";
    let mut found = loopback_v4_listener_inodes(tcp, tcp6, 0x1F90);
    found.sort_unstable();
    // 127.0.0.1, 0.0.0.0, :: (maybe dual-stack) and ::ffff:127.0.0.1 count.
    // 127.0.0.2, another port, a non-LISTEN row and ::1 do not.
    assert_eq!(found, vec![111, 112, 211, 213]);
    assert!(loopback_v4_listener_inodes(tcp, tcp6, 0x2000).is_empty());
}

#[cfg(target_os = "linux")]
const OWNED_HEALTH_HELPER_PORT: &str = "AGENTKERNEL_SMOKE_OWNED_HEALTH_HELPER_PORT";

/// Minimal HTTP responder that answers every request with 200 OK.
#[cfg(target_os = "linux")]
fn serve_ok(listener: std::net::TcpListener, hits: std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { return };
        hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut buffer = [0u8; 1024];
        let _ = stream.read(&mut buffer);
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
    }
}

/// Re-executed by `owned_listener_is_accepted_after_startup_absence` as the
/// "server" child. Without the env var it is a no-op.
#[cfg(target_os = "linux")]
#[test]
fn owned_health_helper() {
    let Ok(port) = std::env::var(OWNED_HEALTH_HELPER_PORT) else {
        return;
    };
    // Start up "slowly" so the parent sees a normal not-yet-bound phase.
    std::thread::sleep(Duration::from_millis(300));
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
    serve_ok(listener, Default::default());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn foreign_healthy_listener_fails_closed_before_any_request() {
    let foreign = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = foreign.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_hits = hits.clone();
    std::thread::spawn(move || serve_ok(foreign, server_hits));

    // The intended server is alive but never binds the port.
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let result = wait_for_owned_health(&mut child, port, 20).await;
    let _ = child.kill();
    let _ = child.wait();

    let error = result.expect_err("a foreign healthy listener must be rejected");
    assert!(
        error.to_string().contains("another process is listening"),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no request may reach a listener the child does not own"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn owned_listener_is_accepted_after_startup_absence() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "owned_health_helper", "--nocapture"])
        .env(OWNED_HEALTH_HELPER_PORT, port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let result = wait_for_owned_health(&mut child, port, 100).await;
    let _ = child.kill();
    let _ = child.wait();
    result.expect("the child's own listener must be accepted once it binds");
}
