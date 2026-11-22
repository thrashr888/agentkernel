use super::{
    bridge::{Bridge, Receipt},
    docker_engine::{DaemonStamp, DesktopEofEngine, DockerEngine},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read},
    os::fd::AsRawFd,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn denied() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

/// Local, out-of-band approved ARM64 Rex/gate/relay bytes, tied to the full
/// prepared Docker image identity. Does not fetch, install or mount any asset.
/// Caller supplies expected hashes from its trusted manifest, never from
/// a guest or user-supplied image tag. Verification does not attest image
/// contents: the provisioning owner must establish that link before attach.
pub struct PreparedGuestAssets {
    image_id: String,
}

impl PreparedGuestAssets {
    pub fn verify(
        image_id: String,
        rex: (&Path, [u8; 32]),
        gate: (&Path, [u8; 32]),
        relay: (&Path, [u8; 32]),
    ) -> io::Result<Self> {
        validate_image_id(&image_id)?;
        for (path, digest) in [rex, gate, relay] {
            verify_arm64_elf(path, digest)?;
        }
        Ok(Self { image_id })
    }
}

fn validate_image_id(image_id: &str) -> io::Result<()> {
    if image_id.strip_prefix("sha256:").is_some_and(|id| {
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(denied())
    }
}

fn verify_arm64_elf(path: &Path, expected: [u8; 32]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > 256 * 1024 * 1024 {
        return Err(denied());
    }
    let mut header = [0u8; 20];
    file.read_exact(&mut header)?;
    if header[..6] != *b"\x7fELF\x02\x01" || header[18..] != [0xb7, 0x00] {
        return Err(denied());
    }
    let mut hash = Sha256::new();
    hash.update(header);
    let mut buf = [0u8; 8192];
    let mut remaining = meta.len() - header.len() as u64;
    while remaining > 0 {
        let cap = remaining.min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..cap])?;
        hash.update(&buf[..cap]);
        remaining -= cap as u64;
    }
    if file.read(&mut [0])? != 0 || hash.finalize()[..] != expected {
        return Err(denied());
    }
    Ok(())
}

/// Caller boundary: the owner-only VMM must have verified this immutable ID,
/// exact running StartedAt, and nonce from the root-owned boot marker, plus
/// the prepared image's provenance. Never construct from a name/HTTP input.
pub struct VerifiedOwnedBoot {
    receipt: Receipt,
    image_id: String,
}

impl VerifiedOwnedBoot {
    fn from_owner_verified_receipt(
        id: String,
        started_at: String,
        nonce: [u8; 32],
        image_id: String,
    ) -> io::Result<Self> {
        validate_image_id(&image_id)?;
        Ok(Self {
            receipt: Receipt::new(id, started_at, nonce)?,
            image_id,
        })
    }

    fn key(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"agentkernel-owner-rex-v1\0");
        for s in [self.receipt.id(), self.receipt.started_at(), &self.image_id] {
            digest.update((s.len() as u64).to_be_bytes());
            digest.update(s.as_bytes());
        }
        // Each boot gets a different endpoint (96-bit label); short enough
        // for macOS sockaddr_un. Owner VMM chooses a short private root.
        hex::encode(&digest.finalize()[..12])
    }
}

/// Explicitly constructed by owner-local wiring only, for the vetted M5
/// Docker Desktop EOF path. No background discovery or registration, no
/// persistence of boot nonce. Owns in-memory listeners for already-persisted
/// registrations and intentionally offers no HTTP, SSH, or Rex settings hook.
pub struct OwnerLocalRex {
    root: PathBuf,
    cli: DockerEngine,
    image_id: String,
    active: BTreeMap<String, Bridge>,
    revoked: BTreeSet<String>,
    _registry_lock: File,
}

impl OwnerLocalRex {
    pub fn enable_vetted_docker_desktop(
        root: &Path,
        trusted_cli: &Path,
        trusted_daemon_socket: &Path,
        empty_private_docker_config: &Path,
        assets: PreparedGuestAssets,
    ) -> io::Result<Self> {
        owner_dir(root)?;
        let lock = registry_lock(root)?;
        Ok(Self {
            root: root.to_owned(),
            cli: DockerEngine::new(
                trusted_cli,
                trusted_daemon_socket,
                empty_private_docker_config,
                1000,
                1000,
            )?,
            image_id: assets.image_id,
            active: BTreeMap::new(),
            revoked: BTreeSet::new(),
            _registry_lock: lock,
        })
    }

    pub(crate) fn expected_image_id(&self) -> &str {
        &self.image_id
    }

    /// Only a persisted manager-captured launch is acceptable, never an ID,
    /// guest name, user image, or values deserialized from an HTTP request.
    /// Verify the prepared image and actual Docker launch policy before the
    /// fixed root gate operation; recheck after it since a restart can race.
    /// The returned proof is opaque; keep it for explicit registry revocation.
    pub async fn attach_captured_launch(
        &mut self,
        launch: &crate::ssh::DockerSshLaunch,
    ) -> io::Result<(PathBuf, VerifiedOwnedBoot)> {
        let (id, started, endpoint) = launch.rex_owner_identity();
        self.cli.require_saved_daemon(endpoint)?;
        // Respect in-process revocation even before any inspect/root exec.
        let scope = self.scope_for_captured_launch(launch)?;
        if self.is_revoked_key(&scope.key())? {
            return Err(denied());
        }
        let nonce = self
            .cli
            .capture_owner_boot(id, started, &self.image_id)
            .await?;
        let verified = VerifiedOwnedBoot::from_owner_verified_receipt(
            id.to_owned(),
            started.to_owned(),
            nonce,
            self.image_id.clone(),
        )?;
        let socket = self.attach(&verified)?;
        Ok((socket, verified))
    }

    /// Attach an already owner-verified boot and return its private Unix
    /// socket path. Keep this registry alive while native Rex uses the address.
    /// Reopen with the same verified boot after a process restart; never supply
    /// a stale/deleted persisted registration. A new boot gets a new address.
    fn attach(&mut self, boot: &VerifiedOwnedBoot) -> io::Result<PathBuf> {
        if boot.image_id != self.image_id {
            return Err(denied());
        }
        let key = boot.key();
        if self.is_revoked_key(&key)? {
            return Err(denied());
        }
        if let Some(active) = self.active.get(&key) {
            return if active.is_running() {
                Ok(active.socket().to_owned())
            } else {
                Err(io::ErrorKind::NotConnected.into())
            };
        }
        let dir = self.root.join(&key);
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
        owner_dir(&dir)?;
        let engine = DesktopEofEngine::for_owner_verified_boot(
            self.cli.clone(),
            boot.receipt.id().to_owned(),
            boot.image_id.clone(),
        )?;
        let bridge = Bridge::start(
            &dir,
            boot.receipt.clone(),
            Arc::new(engine),
            Duration::from_secs(5),
        )?;
        let socket = bridge.socket().to_owned();
        self.active.insert(key, bridge);
        Ok(socket)
    }

    fn scope_for_captured_launch(
        &self,
        launch: &crate::ssh::DockerSshLaunch,
    ) -> io::Result<VerifiedOwnedBoot> {
        let (id, started, _) = launch.rex_owner_identity();
        // This scope is used only for path/status/revocation. Attach cannot
        // receive it; attachment always uses the nonce captured by Docker.
        VerifiedOwnedBoot::from_owner_verified_receipt(
            id.to_owned(),
            started.to_owned(),
            [0; 32],
            self.image_id.clone(),
        )
    }

    fn is_revoked_key(&self, key: &str) -> io::Result<bool> {
        if self.revoked.contains(key) {
            return Ok(true);
        }
        let dir = self.root.join(key);
        match fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            other => {
                other?;
            }
        }
        owner_dir(&dir)?;
        match fs::symlink_metadata(dir.join("revoked")) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn has_unrevoked_registration(
        &self,
        launch: &crate::ssh::DockerSshLaunch,
    ) -> io::Result<bool> {
        let key = self.scope_for_captured_launch(launch)?.key();
        if self.is_revoked_key(&key)? {
            return Ok(self.active.contains_key(&key));
        }
        match fs::symlink_metadata(self.root.join(&key)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Revoke if our registry ever attached this exact launch. Generic Docker
    /// boots without a Rex registration do not acquire tombstones or do I/O
    /// against Docker. No boot nonce/capture/recovery required for shutdown.
    pub(crate) async fn revoke_registered_launch(
        &mut self,
        launch: &crate::ssh::DockerSshLaunch,
    ) -> io::Result<()> {
        let scope = self.scope_for_captured_launch(launch)?;
        let key = scope.key();
        if !self.active.contains_key(&key) {
            match fs::symlink_metadata(self.root.join(&key)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                other => {
                    other?;
                }
            }
        }
        self.revoke(&scope).await
    }

    /// Persist a nonce-free tombstone before closing the private endpoint.
    /// Even if persistence fails, close the live handle and report failure so
    /// the VM lifecycle does not clear ownership evidence or proceed blindly.
    pub async fn revoke(&mut self, boot: &VerifiedOwnedBoot) -> io::Result<()> {
        if boot.image_id != self.image_id {
            return Err(denied());
        }
        let key = boot.key();
        self.revoked.insert(key.clone());
        let saved = persist_tombstone(&self.root, &key);
        let closed = match self.active.remove(&key) {
            Some(bridge) => bridge.revoke().await,
            None => Ok(()),
        };
        saved.and(closed)
    }
}

fn registry_lock(root: &Path) -> io::Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join("registry.lock"))?;
    let meta = lock.metadata()?;
    // SAFETY: geteuid has no pointer or input requirements.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o7777 != 0o600
        || meta.nlink() != 1
    {
        return Err(denied());
    }
    // SAFETY: lock retains ownership of this descriptor.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(lock)
}

fn persist_tombstone(root: &Path, key: &str) -> io::Result<()> {
    let dir = root.join(key);
    owner_dir(&dir)?;
    let path = dir.join("revoked");
    let file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?,
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    // SAFETY: geteuid has no pointer or input requirements.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o7777 != 0o600
        || meta.nlink() != 1
    {
        return Err(denied());
    }
    file.sync_all()?;
    File::open(&dir)?.sync_all()?;
    File::open(root)?.sync_all()
}

fn owner_dir(dir: &Path) -> io::Result<()> {
    if !dir.is_absolute() || dir.canonicalize()? != dir {
        return Err(denied());
    }
    let meta = fs::symlink_metadata(dir)?;
    // SAFETY: no pointer or input requirements.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o7777 != 0o700 {
        return Err(denied());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_stable_for_the_exact_boot_but_not_another_generation() {
        let boot = |stamp: &str| {
            VerifiedOwnedBoot::from_owner_verified_receipt(
                "a".repeat(64),
                stamp.to_owned(),
                [7; 32],
                "sha256:".to_owned() + &"b".repeat(64),
            )
            .unwrap()
        };
        assert_eq!(boot("boot-a").key(), boot("boot-a").key());
        assert_ne!(boot("boot-a").key(), boot("boot-b").key());
        assert!(validate_image_id("ubuntu:latest").is_err());
    }
    #[test]
    fn prepared_assets_require_exact_bytes_and_arm64_header() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("guest");
        let mut blob = vec![0; 32];
        blob[..6].copy_from_slice(b"\x7fELF\x02\x01");
        blob[18] = 0xb7;
        fs::write(&file, &blob).unwrap();
        assert!(verify_arm64_elf(&file, Sha256::digest(&blob).into()).is_ok());
        assert!(verify_arm64_elf(&file, [0; 32]).is_err());
        blob[18] = 0x3e;
        fs::write(&file, &blob).unwrap();
        assert!(verify_arm64_elf(&file, Sha256::digest(&blob).into()).is_err());
    }
    #[test]
    fn normal_launch_manifest_discovery_is_opt_in_and_does_not_write() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().canonicalize().unwrap();
        let dir = config.join("agentkernel-desktop");
        let default = dir.join("owner-rex.json");
        assert!(
            OwnerLocalRex::discover_owner_manifest(None, None)
                .unwrap()
                .is_none()
        );
        assert!(
            OwnerLocalRex::discover_owner_manifest(None, Some(&config))
                .unwrap()
                .is_none()
        );
        assert!(!dir.exists());
        fs::create_dir(&dir).unwrap();
        fs::write(&default, "{broken JSON").unwrap();
        fs::set_permissions(&default, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(OwnerLocalRex::discover_owner_manifest(None, Some(&config)).is_err());
        let missing_override = config.join("missing-explicit.json");
        assert_eq!(
            OwnerLocalRex::discover_owner_manifest(
                Some(missing_override.as_os_str()),
                Some(&config)
            )
            .err()
            .unwrap()
            .kind(),
            io::ErrorKind::NotFound,
        );
        let alias = config.join("alias");
        symlink(&config, &alias).unwrap();
        assert!(OwnerLocalRex::discover_owner_manifest(None, Some(&alias)).is_err());
        fs::remove_file(&default).unwrap();
        symlink(&missing_override, &default).unwrap();
        assert!(OwnerLocalRex::discover_owner_manifest(None, Some(&config)).is_err());
    }

    #[test]
    fn normal_launch_valid_manifest_activates_only_separate_registry() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        let temp = tempfile::Builder::new()
            .prefix("rr-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let base = temp.path().canonicalize().unwrap();
        let endpoint = base.join("r");
        let docker_config = base.join("config");
        for dir in [&endpoint, &docker_config] {
            DirBuilder::new().mode(0o700).create(dir).unwrap();
        }
        let cli = base.join("docker");
        fs::write(&cli, "#!/bin/sh\nexit 99\n").unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = base.join("daemon.sock");
        let _daemon = UnixListener::bind(&socket).unwrap();
        let asset = base.join("arm64");
        let mut blob = vec![0; 32];
        blob[..6].copy_from_slice(b"\x7fELF\x02\x01");
        blob[18] = 0xb7;
        fs::write(&asset, &blob).unwrap();
        let pin = serde_json::json!({"path":asset,"sha256":hex::encode(Sha256::digest(&blob))});
        let config = base.join("Application Support");
        let dir = config.join("agentkernel-desktop");
        fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.json");
        fs::write(&settings, "untouched").unwrap();
        let manifest = dir.join("owner-rex.json");
        fs::write(
            &manifest,
            serde_json::json!({
                "approved_prepared_m5_docker":true, "endpoint_root":endpoint,
                "docker_cli":cli, "docker_socket":socket, "empty_docker_config":docker_config,
                "image_id":format!("sha256:{}", "b".repeat(64)), "rex":pin, "gate":pin, "relay":pin
            })
            .to_string(),
        )
        .unwrap();
        fs::set_permissions(&manifest, fs::Permissions::from_mode(0o600)).unwrap();
        let registry = OwnerLocalRex::discover_owner_manifest(None, Some(&config))
            .unwrap()
            .unwrap();
        assert_eq!(
            registry.expected_image_id(),
            format!("sha256:{}", "b".repeat(64))
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), "untouched");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
    }

    #[test]
    fn owner_manifest_refuses_public_and_symlink_input() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest");
        fs::write(&manifest, "{}").unwrap();
        fs::set_permissions(&manifest, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            OwnerLocalRex::from_owner_manifest(&manifest)
                .err()
                .unwrap()
                .kind()
                == io::ErrorKind::PermissionDenied
        );
        let alias = dir.path().join("alias");
        symlink(&manifest, &alias).unwrap();
        assert!(OwnerLocalRex::from_owner_manifest(&alias).is_err());
        assert!(OwnerLocalRex::from_owner_manifest(Path::new("relative")).is_err());
    }

    #[tokio::test]
    async fn prepared_reopen_restores_only_the_recorded_boot_after_manager_restart() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        let temp = tempfile::Builder::new()
            .prefix("rr-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let root = temp.path().join("r");
        let config = temp.path().join("c");
        for path in [&root, &config] {
            DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        let cli = temp.path().join("cli");
        let mode = temp.path().join("mode");
        let socket = temp.path().join("d");
        let _daemon = UnixListener::bind(&socket).unwrap();
        let image = format!("sha256:{}", "b".repeat(64));
        let id = "a".repeat(64);
        let marker = uuid::Uuid::new_v4().to_string();
        let start = "2026-11-22T01:02:03.123456789Z";
        let policy = concat!(
            r#""1000:1000""#,
            "\n",
            r#"{"Privileged":false,"ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges"],"PidMode":"","UsernsMode":"","UTSMode":"","Tmpfs":{"/run/agentkernel-rex-boot":"rw,noexec,nosuid,nodev,uid=0,gid=0,mode=0755","/run/agentkernel-rex":"rw,noexec,nosuid,nodev,uid=1000,gid=1000,mode=0700"}}"#,
            "\n[]\n",
        );
        fs::write(&mode, "good").unwrap();
        fs::write(&cli, format!(r#"#!/bin/sh
set -eu
shift 4
mode=$(/bin/cat '{mode}')
case "$1" in
 inspect)
   test "$6" = '{id}'
   case "$5" in
     *com.agentkernel.rex.prepared*)
        test "$mode" != wrong-label
        printf '%s\n' '{id}|{image}|{marker}|none|{{}}|false'
        ;;
     *HostConfig*)
        test "$mode" != bad-policy
        printf '%s' '{policy}'
        ;;
     *StartedAt*)
        test "$mode" != stopped
        if test "$mode" = changed; then stamp=changed; else stamp='{start}'; fi
        printf '%s\ntrue\nfalse\n%s\n%s\n' '{id}' "$stamp" '{image}'
        ;;
     *) exit 7;;
   esac
   ;;
 exec)
   test "$4" = '{id}'
   case "$5" in
     /usr/bin/test) test "$6 $7" = '-f /run/agentkernel-rex-boot/boot'; test "$mode" != missing-marker;;
     /usr/libexec/rex-eof-guest)
       test "$3 $6" = '0:0 --owner-boot'
       if test "$mode" = wrong-nonce; then printf '22345678901234567890123456789012';
       else printf '12345678901234567890123456789012'; fi
       ;;
     *) exit 8;;
   esac
   ;;
 *) exit 9;;
esac
"#,mode=mode.display())).unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let open = || {
            OwnerLocalRex::enable_vetted_docker_desktop(
                &root,
                &cli,
                &socket,
                &config,
                PreparedGuestAssets {
                    image_id: image.clone(),
                },
            )
            .unwrap()
        };
        let mut owner = open();
        let nonce = *b"12345678901234567890123456789012";
        let boot = VerifiedOwnedBoot::from_owner_verified_receipt(
            id.clone(),
            start.into(),
            nonce,
            image.clone(),
        )
        .unwrap();
        let record = PreparedRecord {
            id: id.clone(),
            image: image.clone(),
            owner: marker.clone(),
            started_at: Some(start.into()),
            daemon: Some(owner.cli.daemon_stamp()),
            nonce_sha256: Some(hex::encode(Sha256::digest(nonce))),
        };
        assert!(owner.list_prepared_sandboxes().unwrap().is_empty());
        let journal = owner.prepared_record_path("demo").unwrap();
        owner.publish_prepared_record(&journal, &record).unwrap();
        let path = owner.attach(&boot).unwrap();
        assert_eq!(
            owner.list_prepared_sandboxes().unwrap(),
            vec![("demo".into(), "open")]
        );
        drop(owner);
        tokio::time::timeout(Duration::from_secs(1), async {
            while path.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut owner = open();
        assert_eq!(
            owner.list_prepared_sandboxes().unwrap(),
            vec![("demo".into(), "reopenable")]
        );
        for bad in [
            "wrong-label",
            "bad-policy",
            "stopped",
            "changed",
            "missing-marker",
            "wrong-nonce",
        ] {
            fs::write(&mode, bad).unwrap();
            assert!(
                owner.reopen_prepared_sandbox("demo").await.is_err(),
                "{bad}"
            );
            assert!(!path.exists(), "{bad}");
        }
        fs::write(&mode, "good").unwrap();
        assert_eq!(owner.reopen_prepared_sandbox("demo").await.unwrap(), path);
        owner.revoke(&boot).await.unwrap();
        assert!(!path.exists());
        assert_eq!(
            owner.list_prepared_sandboxes().unwrap(),
            vec![("demo".into(), "unavailable")]
        );
        assert!(owner.reopen_prepared_sandbox("demo").await.is_err());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn registry_is_single_owner_and_exact_tombstone_refuses_reopen() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        let temp = tempfile::Builder::new()
            .prefix("rr-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let root = temp.path().join("r");
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let cli = temp.path().join("cli");
        fs::write(&cli, "#!/bin/sh\nexit 99\n").unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let config = temp.path().join("c");
        DirBuilder::new().mode(0o700).create(&config).unwrap();
        let socket = temp.path().join("d");
        let _daemon = UnixListener::bind(&socket).unwrap();
        let image = "sha256:".to_owned() + &"b".repeat(64);
        let open = || {
            OwnerLocalRex::enable_vetted_docker_desktop(
                &root,
                &cli,
                &socket,
                &config,
                PreparedGuestAssets {
                    image_id: image.clone(),
                },
            )
        };
        let mut owner = open().unwrap();
        assert!(open().is_err());
        let boot = VerifiedOwnedBoot::from_owner_verified_receipt(
            "a".repeat(64),
            "boot".into(),
            [3; 32],
            image.clone(),
        )
        .unwrap();
        let path = owner.attach(&boot).unwrap();
        assert!(path.exists());
        let launch = serde_json::from_value(serde_json::json!({
            "id": "a".repeat(64), "started_at": "boot", "port": 2345,
            "endpoint": format!("unix://{}", socket.display()),
        }))
        .unwrap();
        assert!(owner.has_unrevoked_registration(&launch).unwrap());
        owner.revoke_registered_launch(&launch).await.unwrap();
        assert!(!path.exists());
        drop(owner);
        let mut reopened = open().unwrap();
        assert!(reopened.is_revoked_key(&boot.key()).unwrap());
        assert!(reopened.attach_captured_launch(&launch).await.is_err());
        assert!(!path.exists());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedRecord {
    id: String,
    image: String,
    owner: String,
    started_at: Option<String>,
    #[serde(default)]
    daemon: Option<DaemonStamp>,
    #[serde(default)]
    nonce_sha256: Option<String>,
}

impl OwnerLocalRex {
    fn prepared_record_path(&self, name: &str) -> io::Result<PathBuf> {
        crate::validation::validate_sandbox_name(name).map_err(|_| denied())?;
        Ok(self.root.join(format!("prepared-{name}.json")))
    }

    fn publish_prepared_record(&self, path: &Path, record: &PreparedRecord) -> io::Result<()> {
        use std::io::Write;
        let mut staging = tempfile::NamedTempFile::new_in(&self.root)?;
        staging.write_all(&serde_json::to_vec(record).map_err(io::Error::other)?)?;
        staging.as_file().sync_all()?;
        staging.persist(path).map_err(|e| e.error)?;
        File::open(&self.root)?.sync_all()
    }

    /// Dedicated Rex-only sandbox; no selected user image, SSH, mounted
    /// workspace, port, network egress, or credential injection.
    pub(crate) async fn create_prepared_sandbox(&mut self, name: &str) -> io::Result<PathBuf> {
        let path = self.prepared_record_path(name)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        let owner = uuid::Uuid::new_v4().to_string();
        let id = self.cli.create_prepared(&self.image_id, &owner).await?;
        let mut record = PreparedRecord {
            id,
            image: self.image_id.clone(),
            owner,
            started_at: None,
            daemon: Some(self.cli.daemon_stamp()),
            nonce_sha256: None,
        };
        let result = async {
            // First publish exact ID/label, then start. Retain this record if
            // cleanup cannot prove ownership, never recover by Docker name.
            self.publish_prepared_record(&path, &record)?;
            let (started, nonce) = self
                .cli
                .start_prepared(&record.id, &record.image, &record.owner)
                .await?;
            record.started_at = Some(started.clone());
            record.nonce_sha256 = Some(hex::encode(Sha256::digest(nonce)));
            self.publish_prepared_record(&path, &record)?;
            let boot = VerifiedOwnedBoot::from_owner_verified_receipt(
                record.id.clone(),
                started,
                nonce,
                record.image.clone(),
            )?;
            self.attach(&boot)
        }
        .await;
        if result.is_err()
            && self
                .cli
                .remove_prepared(&record.id, &record.image, &record.owner)
                .await
                .is_ok()
        {
            let _ = fs::remove_file(&path);
            let _ = File::open(&self.root).and_then(|d| d.sync_all());
        }
        result
    }

    fn read_prepared_record(&self, path: &Path) -> io::Result<PreparedRecord> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no pointer or input requirements.
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o7777 != 0o600
            || meta.nlink() != 1
            || meta.len() > 8192
        {
            return Err(denied());
        }
        let mut raw = Vec::new();
        file.take(8193).read_to_end(&mut raw)?;
        if raw.len() > 8192 {
            return Err(denied());
        }
        serde_json::from_slice(&raw).map_err(io::Error::other)
    }

    fn reopen_scope(&self, record: &PreparedRecord) -> io::Result<VerifiedOwnedBoot> {
        let Some(start) = record.started_at.as_deref() else {
            return Err(denied());
        };
        if record.image != self.image_id
            || uuid::Uuid::parse_str(&record.owner).is_err()
            || record.daemon.as_ref() != Some(&self.cli.daemon_stamp())
            || !record.nonce_sha256.as_ref().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
            })
        {
            return Err(denied());
        }
        let scope = VerifiedOwnedBoot::from_owner_verified_receipt(
            record.id.clone(),
            start.to_owned(),
            [0; 32],
            record.image.clone(),
        )?;
        if self.is_revoked_key(&scope.key())? {
            return Err(denied());
        }
        Ok(scope)
    }

    pub(crate) fn list_prepared_sandboxes(&self) -> io::Result<Vec<(String, &'static str)>> {
        let mut boxes = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name
                .to_str()
                .and_then(|name| name.strip_prefix("prepared-"))
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if crate::validation::validate_sandbox_name(name).is_err() {
                continue;
            }
            let status = match self
                .read_prepared_record(&entry.path())
                .and_then(|r| self.reopen_scope(&r))
            {
                Ok(scope)
                    if self
                        .active
                        .get(&scope.key())
                        .is_some_and(Bridge::is_running) =>
                {
                    "open"
                }
                Ok(_) => "reopenable",
                Err(_) => "unavailable",
            };
            boxes.push((name.to_owned(), status));
        }
        boxes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Ok(boxes)
    }

    /// Never calls create/start/list or resolves a Docker name. The boot used
    /// by the original Create must still own the exact nonce and launcher.
    pub(crate) async fn reopen_prepared_sandbox(&mut self, name: &str) -> io::Result<PathBuf> {
        let record = self.read_prepared_record(&self.prepared_record_path(name)?)?;
        let scope = self.reopen_scope(&record)?;
        let nonce = self
            .cli
            .reopen_prepared(
                scope.receipt.id(),
                &record.image,
                &record.owner,
                scope.receipt.started_at(),
                record.daemon.as_ref().ok_or_else(denied)?,
            )
            .await
            .map_err(|_| denied())?;
        if record.nonce_sha256.as_deref() != Some(&hex::encode(Sha256::digest(nonce))) {
            return Err(denied());
        }
        let verified = VerifiedOwnedBoot::from_owner_verified_receipt(
            record.id,
            scope.receipt.started_at().to_string(),
            nonce,
            record.image,
        )?;
        let key = verified.key();
        if self
            .active
            .get(&key)
            .is_some_and(|bridge| !bridge.is_running())
        {
            self.active.remove(&key);
        }
        self.attach(&verified)
    }

    /// Remove only the exact owner-labelled prepared container. A marker
    /// and fsync precede closing the endpoint and deleting that container.
    pub(crate) async fn remove_prepared_sandbox(&mut self, name: &str) -> io::Result<()> {
        let path = self.prepared_record_path(name)?;
        let record = self.read_prepared_record(&path)?;
        if record.image != self.image_id || uuid::Uuid::parse_str(&record.owner).is_err() {
            return Err(denied());
        }
        if let Some(started) = record.started_at {
            let boot = VerifiedOwnedBoot::from_owner_verified_receipt(
                record.id.clone(),
                started,
                [0; 32],
                record.image.clone(),
            )?;
            let key = boot.key();
            if self.active.contains_key(&key) || self.root.join(&key).exists() {
                self.revoke(&boot).await?;
            }
        }
        self.cli
            .remove_prepared(&record.id, &record.image, &record.owner)
            .await?;
        fs::remove_file(path)?;
        File::open(&self.root)?.sync_all()
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetPin {
    path: PathBuf,
    sha256: String,
}

impl AssetPin {
    fn pair(&self) -> io::Result<(&Path, [u8; 32])> {
        let mut sha = [0u8; 32];
        hex::decode_to_slice(&self.sha256, &mut sha).map_err(|_| denied())?;
        if !self.path.is_absolute() {
            return Err(denied());
        }
        Ok((&self.path, sha))
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerManifest {
    approved_prepared_m5_docker: bool,
    endpoint_root: PathBuf,
    docker_cli: PathBuf,
    docker_socket: PathBuf,
    empty_docker_config: PathBuf,
    image_id: String,
    rex: AssetPin,
    gate: AssetPin,
    relay: AssetPin,
}

impl OwnerLocalRex {
    /// Normal app startup discovers only this separate platform config file.
    /// An explicit override is authoritative (including an invalid or missing
    /// override) so a test configuration cannot activate normal owner state.
    pub(crate) fn discover_owner_manifest(
        explicit: Option<&std::ffi::OsStr>,
        config_dir: Option<&Path>,
    ) -> io::Result<Option<Self>> {
        if let Some(path) = explicit {
            return Self::from_owner_manifest(Path::new(path)).map(Some);
        }
        let Some(dir) = config_dir else {
            return Ok(None);
        };
        if !dir.is_absolute() {
            return Err(denied());
        }
        let path = dir.join("agentkernel-desktop").join("owner-rex.json");
        // First distinguish an absent default from a present manifest whose
        // contents/pinned assets are broken. Never create the directory/file.
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            other => {
                other?;
            }
        }
        // The normal platform path must not select an alias from a symlink
        // ancestor. The final open also uses O_NOFOLLOW.
        if path.canonicalize()? != path {
            return Err(denied());
        }
        Self::from_owner_manifest(&path).map(Some)
    }

    /// Optional opt-in provided only by the owner who launches the local
    /// control server. The dedicated manifest must be owner-only and is never
    /// created by this program or accepted over HTTP.
    pub(crate) fn from_owner_manifest(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(denied());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no pointer or input requirements.
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o7777 != 0o600
            || meta.nlink() != 1
            || meta.len() > 16_384
        {
            return Err(denied());
        }
        let mut raw = Vec::new();
        (&mut file).take(16_385).read_to_end(&mut raw)?;
        if raw.len() > 16_384 {
            return Err(denied());
        }
        let m: OwnerManifest = serde_json::from_slice(&raw).map_err(io::Error::other)?;
        let display = m.endpoint_root.to_str().ok_or_else(denied)?;
        if !m.approved_prepared_m5_docker
            || display.len() + 1 + 24 + "/rex.sock".len() > 103
            || !display
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
            || !m.docker_cli.is_absolute()
            || !m.docker_socket.is_absolute()
            || !m.empty_docker_config.is_absolute()
        {
            return Err(denied());
        }
        let assets = PreparedGuestAssets::verify(
            m.image_id,
            m.rex.pair()?,
            m.gate.pair()?,
            m.relay.pair()?,
        )?;
        Self::enable_vetted_docker_desktop(
            &m.endpoint_root,
            &m.docker_cli,
            &m.docker_socket,
            &m.empty_docker_config,
            assets,
        )
    }
}
