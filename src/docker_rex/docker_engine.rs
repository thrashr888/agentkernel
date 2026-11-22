//! Fixed Docker CLI transport for the relay. The owner must select the
//! executable and a previously trusted local daemon with protected ancestors.
//! A Unix URL does not authenticate a daemon. Pins detect ordinary replacement
//! but are not an adversarial TOCTOU guarantee on a writable path.
//!
//! The guest must include GNU timeout and the fixed gate. This candidate bounds
//! orphan lifetime, but cannot guarantee that revocation synchronously kills
//! the remote exec: Docker CLI termination is only local. Do not call this a
//! production revocable adapter unless the owner wiring opts into the tested
//! Docker Desktop detach-to-EOF path with the pinned cooperative guest gate.

use std::{
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::bridge::{Engine, Exec};

const FORMAT: &str =
    "{{.Id}}\n{{.State.Running}}\n{{.State.Paused}}\n{{.State.StartedAt}}\n{{.Image}}";
const POLICY_FORMAT: &str = "{{json .Config.User}}\n{{json .HostConfig}}\n{{json .Mounts}}";
const GATE: &str = "/usr/libexec/rex-eof-guest";
const GUEST_TIMEOUT: &str = "/usr/bin/timeout";
const GUEST_LIMIT: &str = "120s";
const KILL_GRACE: &str = "2s";
const LOCAL_BOOT_PHASE: Duration = Duration::from_secs(5);

fn denied() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

fn full_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Pin {
    dev: u64,
    ino: u64,
}

impl Pin {
    fn from(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DaemonStamp {
    socket: PathBuf,
    dev: u64,
    ino: u64,
}

/// The executable and socket are provided by trusted app wiring, never by
/// Rex, guest names, inherited Docker settings, or a registration's display
/// labels. Supply a dedicated empty local config dir with mode 0700.
#[derive(Clone)]
pub struct DockerEngine {
    cli: PathBuf,
    cli_pin: Pin,
    socket: PathBuf,
    daemon_pin: Pin,
    config: PathBuf,
    config_pin: Pin,
    run_as: String,
}

/// Drops the *local* CLI process and IO. The fixed in-guest timeout separately
/// bounds the remote relay process after loss of its client.
pub struct CliLease(Child);

impl Drop for CliLease {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

impl DockerEngine {
    pub fn new(
        cli: &Path,
        socket: &Path,
        empty_config: &Path,
        guest_uid: u32,
        guest_gid: u32,
    ) -> io::Result<Self> {
        if guest_uid == 0 || guest_gid == 0 {
            return Err(denied());
        }
        let cli = cli.canonicalize()?;
        let cli_meta = fs::symlink_metadata(&cli)?;
        if !cli_meta.is_file() || cli_meta.mode() & 0o111 == 0 || cli_meta.mode() & 0o022 != 0 {
            return Err(denied());
        }
        let socket = socket.canonicalize()?;
        let daemon_meta = fs::symlink_metadata(&socket)?;
        if !daemon_meta.file_type().is_socket() || socket.to_str().is_none() {
            return Err(denied());
        }
        let config = empty_config.canonicalize()?;
        let config_meta = fs::symlink_metadata(&config)?;
        if !config_meta.is_dir()
            // SAFETY: geteuid has no argument or pointer requirements.
            || config_meta.uid() != unsafe { libc::geteuid() }
            || config_meta.mode() & 0o7777 != 0o700
            || config.read_dir()?.next().is_some()
        {
            return Err(denied());
        }
        Ok(Self {
            cli,
            cli_pin: Pin::from(&cli_meta),
            socket,
            daemon_pin: Pin::from(&daemon_meta),
            config,
            config_pin: Pin::from(&config_meta),
            run_as: format!("{guest_uid}:{guest_gid}"),
        })
    }

    fn command(&self) -> io::Result<Command> {
        let cli = fs::symlink_metadata(&self.cli)?;
        let daemon = fs::symlink_metadata(&self.socket)?;
        let config = fs::symlink_metadata(&self.config)?;
        if Pin::from(&cli) != self.cli_pin
            || !cli.is_file()
            || cli.mode() & 0o022 != 0
            || Pin::from(&daemon) != self.daemon_pin
            || !daemon.file_type().is_socket()
            || Pin::from(&config) != self.config_pin
            || !config.is_dir()
            || config.mode() & 0o7777 != 0o700
            || self.config.read_dir()?.next().is_some()
        {
            return Err(denied());
        }
        let mut cli = Command::new(&self.cli);
        cli.env_clear()
            .current_dir(&self.config)
            .arg("--config")
            .arg(&self.config)
            .arg("--host")
            .arg(format!("unix://{}", self.socket.display()))
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok(cli)
    }
}

impl DockerEngine {
    #[cfg(test)]
    pub async fn running_started_at(&self, id: &str) -> io::Result<Option<String>> {
        self.inspect_started_at(id, None).await
    }

    pub(super) async fn inspect_started_at(
        &self,
        id: &str,
        expected_image: Option<&str>,
    ) -> io::Result<Option<String>> {
        if !full_id(id) {
            return Err(denied());
        }
        let mut child = self
            .command()?
            .args(["inspect", "--type", "container", "--format", FORMAT, id])
            .spawn()?;
        use tokio::io::AsyncReadExt;
        let stdout = child.stdout.take().ok_or_else(denied)?;
        let output = tokio::time::timeout(LOCAL_BOOT_PHASE, async {
            let mut output = Vec::new();
            stdout.take(513).read_to_end(&mut output).await?;
            if output.len() > 512 || !child.wait().await?.success() {
                return Err(denied());
            }
            Ok(output)
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        let output = std::str::from_utf8(&output).map_err(|_| denied())?;
        let mut lines = output.strip_suffix('\n').ok_or_else(denied)?.split('\n');
        let result = match (
            lines.next(),
            lines.next(),
            lines.next(),
            lines.next(),
            lines.next(),
            lines.next(),
        ) {
            (Some(found), Some("true"), Some("false"), Some(value), Some(image), None)
                if found == id
                    && expected_image.is_none_or(|expected| expected == image)
                    && !value.is_empty()
                    && !value.bytes().any(|b| b.is_ascii_control()) =>
            {
                Some(value.to_owned())
            }
            _ => None,
        };
        Ok(result)
    }

    pub(super) fn require_saved_daemon(&self, endpoint: &str) -> io::Result<()> {
        let path = Path::new(endpoint.strip_prefix("unix://").ok_or_else(denied)?);
        if !path.is_absolute() || path.canonicalize()? != self.socket {
            return Err(denied());
        }
        // Also recheck the selected CLI/daemon/config pins before proceeding.
        self.command()?;
        Ok(())
    }

    pub(super) async fn capture_owner_boot(
        &self,
        id: &str,
        started_at: &str,
        image: &str,
    ) -> io::Result<[u8; 32]> {
        let verify = || async {
            if self.inspect_started_at(id, Some(image)).await?.as_deref() != Some(started_at) {
                Err(denied())
            } else {
                self.verify_prepared_guest_policy(id).await
            }
        };
        verify().await?;
        let mut child = self
            .command()?
            .args(["exec", "--user", "0:0", id, GATE, "--owner-boot"])
            .spawn()?;
        use tokio::io::AsyncReadExt;
        let stdout = child.stdout.take().ok_or_else(denied)?;
        let nonce = tokio::time::timeout(LOCAL_BOOT_PHASE, async {
            let mut stdout = stdout.take(33);
            let mut nonce = [0u8; 32];
            stdout.read_exact(&mut nonce).await?;
            if stdout.read(&mut [0]).await? != 0 || !child.wait().await?.success() {
                return Err(denied());
            }
            Ok(nonce)
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        verify().await?;
        Ok(nonce)
    }

    async fn verify_prepared_guest_policy(&self, id: &str) -> io::Result<()> {
        // Exec as root is safe only for the vetted immutable guest. SSH
        // capture alone does not prove this: inspect actual launch policy.
        let mut child = self
            .command()?
            .args([
                "inspect",
                "--type",
                "container",
                "--format",
                POLICY_FORMAT,
                id,
            ])
            .spawn()?;
        use tokio::io::AsyncReadExt;
        let stdout = child.stdout.take().ok_or_else(denied)?;
        let raw = tokio::time::timeout(LOCAL_BOOT_PHASE, async {
            let mut raw = Vec::new();
            stdout.take(16_385).read_to_end(&mut raw).await?;
            if raw.len() > 16_384 || !child.wait().await?.success() {
                return Err(denied());
            }
            Ok(raw)
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        verify_launch_policy(&raw)
    }

    #[cfg(test)]
    pub async fn open_fixed(
        &self,
        id: &str,
    ) -> io::Result<Exec<ChildStdout, ChildStdin, CliLease>> {
        if !full_id(id) {
            return Err(denied());
        }
        self.spawn_gate(id, true)
    }

    fn spawn_gate(
        &self,
        id: &str,
        qa_timeout: bool,
    ) -> io::Result<Exec<ChildStdout, ChildStdin, CliLease>> {
        if !full_id(id) {
            return Err(denied());
        }
        let mut cmd = self.command()?;
        cmd.args(["exec", "--interactive", "--user", &self.run_as, id]);
        if qa_timeout {
            cmd.args([
                GUEST_TIMEOUT,
                "--signal=TERM",
                "--kill-after",
                KILL_GRACE,
                GUEST_LIMIT,
            ]);
        }
        let mut lease = CliLease(cmd.arg(GATE).stdin(Stdio::piped()).spawn()?);
        let stdout = lease.0.stdout.take().ok_or_else(denied)?;
        let stdin = lease.0.stdin.take().ok_or_else(denied)?;
        Ok(Exec {
            stdout,
            stdin,
            lease,
        })
    }
}

fn verify_launch_policy(raw: &[u8]) -> io::Result<()> {
    use serde_json::Value;
    let text = std::str::from_utf8(raw).map_err(|_| denied())?;
    let lines: Vec<_> = text
        .strip_suffix('\n')
        .ok_or_else(denied)?
        .split('\n')
        .collect();
    let [user, host, mounts] = lines.as_slice() else {
        return Err(denied());
    };
    if *user != "\"1000:1000\"" {
        return Err(denied());
    }
    let host: Value = serde_json::from_str(host).map_err(|_| denied())?;
    let mounts: Value = serde_json::from_str(mounts).map_err(|_| denied())?;
    let no_extra = |v: &Value| v.is_null() || v.as_array().is_some_and(Vec::is_empty);
    if host["Privileged"] != false
        || host["ReadonlyRootfs"] != true
        || host["CapDrop"] != serde_json::json!(["ALL"])
        || !matches!(host["SecurityOpt"].as_array().map(Vec::as_slice), Some([entry]) if entry == "no-new-privileges" || entry == "no-new-privileges=true")
        || [
            "CapAdd",
            "Binds",
            "Mounts",
            "Devices",
            "DeviceRequests",
            "VolumesFrom",
            "GroupAdd",
        ]
        .iter()
        .any(|key| !no_extra(&host[key]))
        || ["PidMode", "UsernsMode", "UTSMode"]
            .iter()
            .any(|key| host[*key] != "")
    {
        return Err(denied());
    }

    let allowed = [
        ("/run/agentkernel-rex-boot", "0", "0", "0755"),
        ("/run/agentkernel-rex", "1000", "1000", "0700"),
        ("/tmp", "1000", "1000", "0700"),
    ];
    let Some(tmpfs) = host["Tmpfs"].as_object() else {
        return Err(denied());
    };
    if tmpfs.len() < 2 || tmpfs.len() > 3 {
        return Err(denied());
    }
    for (path, uid, gid, mode) in allowed {
        let Some(opts) = tmpfs.get(path).and_then(Value::as_str) else {
            if path == "/tmp" && !tmpfs.contains_key(path) {
                continue;
            }
            return Err(denied());
        };
        let expected = [
            "rw".to_owned(),
            "noexec".to_owned(),
            "nosuid".to_owned(),
            "nodev".to_owned(),
            format!("uid={uid}"),
            format!("gid={gid}"),
            format!("mode={mode}"),
        ];
        let parts: std::collections::BTreeSet<_> = opts.split(',').collect();
        if parts != expected.iter().map(String::as_str).collect()
            || opts.split(',').count() != expected.len()
        {
            return Err(denied());
        }
    }
    if let Some(items) = mounts.as_array() {
        for mount in items {
            if mount["Type"] != "tmpfs"
                || !mount["Destination"]
                    .as_str()
                    .is_some_and(|dest| tmpfs.contains_key(dest))
            {
                return Err(denied());
            }
        }
    } else if !mounts.is_null() {
        return Err(denied());
    }
    Ok(())
}

/// Owner-wired, explicitly tested M5 Docker Desktop path. Construction is not
/// daemon attestation: caller selects a trusted CLI/socket, and guarantees the
/// immutable owned image includes the exact cooperative EOF gate and fixed relay.
/// Do not use this wrapper on untested daemon paths or arbitrary guest images.
pub struct DesktopEofEngine {
    cli: DockerEngine,
    owned_id: String,
    image_id: String,
}

impl DesktopEofEngine {
    pub fn for_owner_verified_boot(
        cli: DockerEngine,
        owned_id: String,
        image_id: String,
    ) -> io::Result<Self> {
        if !full_id(&owned_id) || !image_id.strip_prefix("sha256:").is_some_and(full_id) {
            return Err(denied());
        }
        Ok(Self {
            cli,
            owned_id,
            image_id,
        })
    }
}

impl Engine for DesktopEofEngine {
    type Reader = ChildStdout;
    type Writer = ChildStdin;
    type Lease = CliLease;

    async fn running_started_at(&self, id: &str) -> io::Result<Option<String>> {
        if id != self.owned_id {
            return Err(denied());
        }
        self.cli.inspect_started_at(id, Some(&self.image_id)).await
    }

    async fn open_fixed(&self, id: &str) -> io::Result<Exec<ChildStdout, ChildStdin, CliLease>> {
        // There is no await after spawn and before ownership transfers, so
        // startup cannot detach between those operations. Drop kills local
        // attach, which the verified Desktop daemon propagates as guest EOF.
        if id != self.owned_id {
            return Err(denied());
        }
        self.cli.spawn_gate(id, false)
    }
}

// Existing CLI contract fixtures stay grouped separately from the dedicated
// prepared-launch methods below.
#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    static SEQ: AtomicUsize = AtomicUsize::new(0);
    const STAMP: &str = "2026-11-22T01:02:03.123456789Z";
    const POLICY: &str = concat!(
        concat!(r#""1000:1000""#, "\n"),
        r#"{"Privileged":false,"ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges"],"PidMode":"","UsernsMode":"","UTSMode":"","Tmpfs":{"/run/agentkernel-rex-boot":"rw,noexec,nosuid,nodev,uid=0,gid=0,mode=0755","/run/agentkernel-rex":"rw,noexec,nosuid,nodev,uid=1000,gid=1000,mode=0700"}}"#,
        "\n[]\n",
    );

    struct Fixture {
        dir: PathBuf,
        engine: DockerEngine,
        sock: std::os::unix::net::UnixListener,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = PathBuf::from(format!(
                "/tmp/ake-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
            let cli = dir.join("fake");
            let config = dir.join("config");
            let socket = dir.join("daemon.sock");
            fs::DirBuilder::new().mode(0o700).create(&config).unwrap();
            let sock = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            let script = format!(
                r#"#!/bin/sh
set -eu
test -z "${{DOCKER_HOST-}}${{DOCKER_CONTEXT-}}${{DOCKER_CONFIG-}}${{SSH_AUTH_SOCK-}}"
test "$#" -ge 5
test "$1" = "--config"
test "$2" = "{config}"
test "$3" = "--host"
test "$4" = "unix://{socket}"
shift 4
case "$1" in
    inspect)
        test "$#" -eq 6
        test "$2 $3 $4" = "--type container --format"
        if test "$5" = '{POLICY_FORMAT}'; then
            test "$6" = '{id}'
            printf '%s' '{POLICY}'
            exit 0
        fi
        test "$5" = '{{{{.Id}}}}
{{{{.State.Running}}}}
{{{{.State.Paused}}}}
{{{{.State.StartedAt}}}}
{{{{.Image}}}}'
        case "$6" in
            a*) printf '%s\ntrue\nfalse\n%s\n%s\n' "$6" '{STAMP}' '{image}' ;;
            b*) printf '%s\ntrue\ntrue\n%s\n%s\n' "$6" '{STAMP}' '{image}' ;;
            c*) printf '%s\ntrue\nfalse\n%s\n%s\n' '{id}' '{STAMP}' '{image}' ;;
            *) exit 2 ;;
        esac
        ;;
    exec)
        if test "$2" = "--user"; then
            test "$#" -eq 6
            test "$3 $4 $5 $6" = "0:0 {id} /usr/libexec/rex-eof-guest --owner-boot"
            printf '12345678901234567890123456789012'
            exit 0
        fi
        test "$2 $3 $4" = "--interactive --user 1000:1001"
        test "$5" = '{id}'
        if test "$#" -eq 11; then
            test "$6 $7 $8 $9" = "/usr/bin/timeout --signal=TERM --kill-after 2s"
            shift 9
            test "$1 $2" = "120s /usr/libexec/rex-eof-guest"
        else
            test "$#" -eq 6
            test "$6" = "/usr/libexec/rex-eof-guest"
            /bin/dd bs=32 count=1 of=/dev/null 2>/dev/null
            printf '\001'
        fi
        printf '%s\n' 'never stdout' >&2
        printf '%s\n' "$$"
        IFS= read -r input
        printf 'relay:%s\n' "$input"
        exec /bin/cat
        ;;
    *) exit 2 ;;
esac
"#,
                config = config.canonicalize().unwrap().display(),
                socket = socket.canonicalize().unwrap().display(),
                id = "a".repeat(64),
                image = format!("sha256:{}", "d".repeat(64)),
            );
            fs::write(&cli, script).unwrap();
            fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(DockerEngine::new(&cli, &socket, &config, 0, 1001).is_err());
            let engine = DockerEngine::new(&cli, &socket, &config, 1000, 1001).unwrap();
            Self { dir, engine, sock }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn inspect_returns_exact_nanoseconds_and_rejects_wrong_id_and_paused() {
        let f = Fixture::new();
        assert_eq!(
            f.engine
                .running_started_at(&"a".repeat(64))
                .await
                .unwrap()
                .as_deref(),
            Some(STAMP)
        );
        assert_eq!(
            f.engine.running_started_at(&"b".repeat(64)).await.unwrap(),
            None
        );
        assert_eq!(
            f.engine.running_started_at(&"c".repeat(64)).await.unwrap(),
            None
        );
        assert!(f.engine.running_started_at("some-name").await.is_err());
    }

    #[test]
    fn prepared_guest_refuses_writable_privileged_root_or_overlaid_binary() {
        assert!(verify_launch_policy(POLICY.as_bytes()).is_ok());
        for (from, to) in [
            ("1000:1000", "0:0"),
            (r#""Privileged":false"#, r#""Privileged":true"#),
            (r#""ReadonlyRootfs":true"#, r#""ReadonlyRootfs":false"#),
            (
                "\n[]\n",
                concat!("\n", r#"[{"Type":"bind","Destination":"/usr"}]"#, "\n"),
            ),
            ("uid=0,gid=0,mode=0755", "uid=1000,gid=1000,mode=0755"),
        ] {
            assert!(verify_launch_policy(POLICY.replace(from, to).as_bytes()).is_err());
        }
    }

    #[tokio::test]
    async fn owner_nonce_uses_saved_local_daemon_and_only_exact_prepared_id() {
        let f = Fixture::new();
        let endpoint = format!("unix://{}", f.engine.socket.display());
        assert!(f.engine.require_saved_daemon(&endpoint).is_ok());
        assert!(
            f.engine
                .require_saved_daemon("tcp://localhost:2375")
                .is_err()
        );
        let expected_image = format!("sha256:{}", "d".repeat(64));
        assert_eq!(
            f.engine
                .capture_owner_boot(&"a".repeat(64), STAMP, &expected_image)
                .await
                .unwrap(),
            *b"12345678901234567890123456789012"
        );
        assert!(
            f.engine
                .capture_owner_boot(&"a".repeat(64), "wrong-boot", &expected_image)
                .await
                .is_err()
        );
        assert!(
            f.engine
                .capture_owner_boot("guest-name", STAMP, &expected_image)
                .await
                .is_err()
        );
        assert!(
            f.engine
                .capture_owner_boot(&"a".repeat(64), STAMP, &"e".repeat(64))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn fixed_exec_passes_no_stderr_and_lease_kills_local_child() {
        let f = Fixture::new();
        assert!(f.engine.open_fixed("same-name").await.is_err());
        let Exec {
            stdout,
            mut stdin,
            lease,
        } = f.engine.open_fixed(&"a".repeat(64)).await.unwrap();
        let mut stdout = tokio::io::BufReader::new(stdout);
        use tokio::io::AsyncBufReadExt;
        let mut first = String::new();
        tokio::time::timeout(Duration::from_secs(2), stdout.read_line(&mut first))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.trim().parse::<u32>().unwrap(), lease.0.id().unwrap());
        stdin.write_all(b"fixture\n").await.unwrap();
        let mut got = [0; 14];
        tokio::time::timeout(Duration::from_secs(2), stdout.read_exact(&mut got))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&got, b"relay:fixture\n");
        drop(lease);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), stdout.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(stdin.write_all(b"after revoke\n").await.is_err());
    }

    #[tokio::test]
    async fn desktop_eof_engine_plugged_into_real_owner_socket_forwards_and_revokes() {
        use super::super::bridge::{Bridge, Receipt};
        use std::sync::Arc;

        let f = Fixture::new();
        let id = "a".repeat(64);
        let receipt = Receipt::new(id, STAMP.to_owned(), [7; 32]).unwrap();
        let base = DockerEngine::new(
            &f.engine.cli,
            &f.engine.socket,
            &f.engine.config,
            1000,
            1001,
        )
        .unwrap();
        let engine = Arc::new(
            DesktopEofEngine::for_owner_verified_boot(
                base,
                "a".repeat(64),
                format!("sha256:{}", "d".repeat(64)),
            )
            .unwrap(),
        );
        assert!(engine.open_fixed(&"b".repeat(64)).await.is_err());
        assert_eq!(
            engine
                .cli
                .inspect_started_at(&"a".repeat(64), Some(&format!("sha256:{}", "e".repeat(64))))
                .await
                .unwrap(),
            None
        );
        let wait = Duration::from_secs(2);
        let dir = f.dir.canonicalize().unwrap();
        let bridge = Bridge::start(&dir, receipt, engine, wait).unwrap();
        let socket = bridge.socket().to_owned();
        let client = tokio::net::UnixStream::connect(&socket).await.unwrap();
        use tokio::io::AsyncBufReadExt;
        let mut client = tokio::io::BufReader::new(client);
        let mut pid = String::new();
        tokio::time::timeout(wait, client.read_line(&mut pid))
            .await
            .unwrap()
            .unwrap();
        assert!(pid.trim().parse::<u32>().is_ok()); // ACK must not leak.
        client.get_mut().write_all(b"attached\n").await.unwrap();
        let mut reply = String::new();
        tokio::time::timeout(wait, client.read_line(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply, "relay:attached\n");
        bridge.revoke().await.unwrap();
        assert_eq!(
            tokio::time::timeout(wait, client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(tokio::net::UnixStream::connect(socket).await.is_err());
    }

    #[tokio::test]
    async fn daemon_socket_replacement_and_docker_config_population_fail_closed() {
        let f = Fixture::new();
        let socket = f.dir.join("daemon.sock");
        fs::rename(&socket, f.dir.join("old.sock")).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(f.engine.running_started_at(&"a".repeat(64)).await.is_err());
        assert!(f.engine.open_fixed(&"a".repeat(64)).await.is_err());
        drop(replacement);
        fs::remove_file(&socket).unwrap();
        fs::rename(f.dir.join("old.sock"), &socket).unwrap();
        let _keep_alive = &f.sock;
        fs::write(f.dir.join("config/config.json"), b"{}").unwrap();
        assert!(f.engine.open_fixed(&"a".repeat(64)).await.is_err());
    }
}

const PREPARED_LABEL: &str = "com.agentkernel.rex.prepared";
const PREPARED_BOOT: &str = r#"
d=/run/agentkernel-rex
umask 077
mkdir "$d/cfg" "$d/state" "$d/data" "$d/cache" "$d/temp"
/usr/local/bin/rex --server "unix://$d/rex.sock" server start --restore=false --socket-mode 0600
test -S "$d/rex.sock"
exec /bin/sleep infinity
"#;

impl DockerEngine {
    async fn prepared_output(&self, args: &[&str]) -> io::Result<String> {
        let child = self.command()?.args(args).spawn()?;
        let out = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        if !out.status.success() || out.stdout.len() > 4096 {
            return Err(denied());
        }
        let text = std::str::from_utf8(&out.stdout).map_err(|_| denied())?;
        Ok(text.trim_end_matches('\n').to_owned())
    }

    pub(super) async fn create_prepared(&self, image: &str, owner: &str) -> io::Result<String> {
        const IMAGE_FORMAT: &str =
            "{{.Id}}|{{.Os}}|{{.Architecture}}|{{json (index .Config \"Volumes\")}}";
        if self
            .prepared_output(&["image", "inspect", "--format", IMAGE_FORMAT, image])
            .await?
            != format!("{image}|linux|arm64|null")
        {
            return Err(denied());
        }
        let label = format!("{PREPARED_LABEL}={owner}");
        let id = self
            .prepared_output(&[
                "create",
                "--pull=never",
                "--platform=linux/arm64",
                "--network=none",
                "--user=1000:1000",
                "--read-only",
                "--cap-drop=ALL",
                "--security-opt=no-new-privileges",
                "--restart=no",
                "--no-healthcheck",
                "--pids-limit=64",
                "--memory=512m",
                "--label",
                &label,
                "--tmpfs",
                "/run/agentkernel-rex-boot:rw,noexec,nosuid,nodev,uid=0,gid=0,mode=0755",
                "--tmpfs",
                "/run/agentkernel-rex:rw,noexec,nosuid,nodev,uid=1000,gid=1000,mode=0700",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,nodev,uid=1000,gid=1000,mode=0700",
                "--entrypoint",
                "/usr/bin/env",
                image,
                "-i",
                "HOME=/run/agentkernel-rex",
                "PATH=/usr/bin:/bin",
                "SHELL=/bin/sh",
                "XDG_CONFIG_HOME=/run/agentkernel-rex/cfg",
                "XDG_STATE_HOME=/run/agentkernel-rex/state",
                "XDG_DATA_HOME=/run/agentkernel-rex/data",
                "XDG_CACHE_HOME=/run/agentkernel-rex/cache",
                "TMPDIR=/run/agentkernel-rex/temp",
                "/bin/sh",
                "-ec",
                PREPARED_BOOT,
            ])
            .await?;
        if !full_id(&id) {
            return Err(denied());
        }
        Ok(id)
    }

    async fn verify_prepared_owner(&self, id: &str, image: &str, owner: &str) -> io::Result<()> {
        if !full_id(id) {
            return Err(denied());
        }
        const FORMAT: &str = "{{.Id}}|{{.Image}}|{{index .Config.Labels \"com.agentkernel.rex.prepared\"}}|{{.HostConfig.NetworkMode}}|{{json .HostConfig.PortBindings}}|{{.HostConfig.PublishAllPorts}}";
        let out = self
            .prepared_output(&["inspect", "--type", "container", "--format", FORMAT, id])
            .await?;
        if out != format!("{id}|{image}|{owner}|none|{{}}|false")
            && out != format!("{id}|{image}|{owner}|none|null|false")
        {
            return Err(denied());
        }
        Ok(())
    }

    pub(super) async fn start_prepared(
        &self,
        id: &str,
        image: &str,
        owner: &str,
    ) -> io::Result<(String, [u8; 32])> {
        self.verify_prepared_owner(id, image, owner).await?;
        self.prepared_output(&["start", id]).await?;
        let started = self
            .inspect_started_at(id, Some(image))
            .await?
            .ok_or_else(denied)?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .prepared_output(&[
                        "exec",
                        "--user",
                        "1000:1000",
                        id,
                        "/usr/bin/test",
                        "-S",
                        "/run/agentkernel-rex/rex.sock",
                    ])
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?;
        self.verify_prepared_owner(id, image, owner).await?;
        let nonce = self.capture_owner_boot(id, &started, image).await?;
        Ok((started, nonce))
    }

    pub(super) fn daemon_stamp(&self) -> DaemonStamp {
        DaemonStamp {
            socket: self.socket.clone(),
            dev: self.daemon_pin.dev,
            ino: self.daemon_pin.ino,
        }
    }

    pub(super) async fn reopen_prepared(
        &self,
        id: &str,
        image: &str,
        owner: &str,
        started: &str,
        daemon: &DaemonStamp,
    ) -> io::Result<[u8; 32]> {
        if &self.daemon_stamp() != daemon {
            return Err(denied());
        }
        self.verify_prepared_owner(id, image, owner).await?;
        if self.inspect_started_at(id, Some(image)).await?.as_deref() != Some(started) {
            return Err(denied());
        }
        // The existing tested owner gate initializes if absent. For reopen,
        // refuse a missing marker before calling it. Workload cannot remove
        // the marker (root-owned tmpfs verified by capture_owner_boot).
        self.prepared_output(&[
            "exec",
            "--user",
            "1000:1000",
            id,
            "/usr/bin/test",
            "-f",
            "/run/agentkernel-rex-boot/boot",
        ])
        .await?;
        let nonce = self.capture_owner_boot(id, started, image).await?;
        self.verify_prepared_owner(id, image, owner).await?;
        if self.inspect_started_at(id, Some(image)).await?.as_deref() != Some(started) {
            return Err(denied());
        }
        Ok(nonce)
    }

    pub(super) async fn remove_prepared(
        &self,
        id: &str,
        image: &str,
        owner: &str,
    ) -> io::Result<()> {
        self.verify_prepared_owner(id, image, owner).await?;
        self.prepared_output(&["rm", "--force", id]).await?;
        Ok(())
    }
}
