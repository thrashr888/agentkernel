//! Durable ownership and process identity for local Firecracker runtimes.
//!
//! A Child handle is useful while the manager that spawned it is alive, but
//! it is not a durable ownership proof. This module keeps the proof in a
//! private journal next to the runtime sockets and holds an advisory lock for
//! the lifetime of the manager's handle. Reattachment is only allowed when
//! the PID still has the same boot ID/start time/executable and both sockets
//! are the same current-user-owned Unix sockets recorded at startup.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "linux"))]
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const JOURNAL_FILE: &str = "runtime.json";
const LOCK_FILE: &str = ".runtime.lock";
const JOURNAL_VERSION: u32 = 1;
const JOURNAL_MODE: u32 = 0o600;
const RUNTIME_MODE: u32 = 0o700;

/// A process identity stronger than a PID, which can be reused after exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_start_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
}

impl ProcessIdentity {
    /// Capture the strongest process identity available for pid.
    pub fn capture(pid: u32) -> Result<Self> {
        if pid == 0 {
            bail!("process identity cannot use PID 0");
        }
        #[cfg(target_os = "linux")]
        {
            let proc_dir = PathBuf::from(format!("/proc/{pid}"));
            if !proc_dir.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("process {pid} does not exist"),
                )
                .into());
            }
            let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            let pid_start_time = match read_linux_pid_start_time(pid) {
                Ok(value) => Some(value),
                Err(error) if process_is_gone(&error) => return Err(error),
                Err(error) if !proc_dir.exists() => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("process {pid} exited while its identity was captured"),
                    )
                    .into());
                }
                Err(_) => None,
            };
            let executable = fs::read_link(format!("/proc/{pid}/exe"))
                .ok()
                .and_then(|path| fs::canonicalize(path).ok())
                .map(|path| path.to_string_lossy().into_owned());
            if pid_start_time.is_none() {
                bail!("no PID-specific process identity is available for PID {pid}");
            }
            return Ok(Self {
                pid,
                boot_id,
                pid_start_time,
                executable,
            });
        }
        #[cfg(not(target_os = "linux"))]
        {
            // macOS does not expose Linux's boot_id/proc start ticks through
            // a portable std API. proc_pidpath is not available in the libc
            // crate on every supported SDK, so use ps's absolute command path
            // as the strongest broadly available identity and fail closed if
            // it cannot be obtained.
            let executable = if pid == std::process::id() {
                std::env::current_exe().ok()
            } else {
                Command::new("ps")
                    .args(["-p", &pid.to_string(), "-o", "comm="])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .and_then(|output| {
                        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                        (!value.is_empty()).then(|| PathBuf::from(value))
                    })
            }
            .and_then(|path| fs::canonicalize(path).ok())
            .map(|path| path.to_string_lossy().into_owned());
            if executable.is_none() {
                bail!("no strong process identity is available for PID {pid}");
            }
            Ok(Self {
                pid,
                boot_id: None,
                pid_start_time: None,
                executable,
            })
        }
    }

    /// Return whether the observed process is provably the same process.
    pub fn matches(&self, observed: &Self) -> bool {
        if self.pid != observed.pid {
            return false;
        }
        let mut compared = 0;
        if let Some(expected) = self.boot_id.as_deref() {
            compared += 1;
            if observed.boot_id.as_deref() != Some(expected) {
                return false;
            }
        }
        if let Some(expected) = self.pid_start_time {
            compared += 1;
            if observed.pid_start_time != Some(expected) {
                return false;
            }
        }
        if let Some(expected) = self.executable.as_deref() {
            compared += 1;
            if observed.executable.as_deref() != Some(expected) {
                return false;
            }
        }
        compared > 0
    }

    pub fn is_alive(&self) -> Result<bool> {
        match Self::capture(self.pid) {
            Ok(observed) if self.matches(&observed) => Ok(true),
            Ok(_) => bail!(
                "PID {} no longer matches its recorded process identity",
                self.pid
            ),
            Err(error) if process_is_gone(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[cfg(target_os = "linux")]
fn read_linux_pid_start_time(pid: u32) -> Result<u64> {
    let value = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // The comm field may contain spaces and ')' characters. The final ')' is
    // the delimiter before the state field.
    let (_, rest) = value
        .rsplit_once(')')
        .ok_or_else(|| anyhow::anyhow!("malformed /proc/{pid}/stat"))?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.first() == Some(&"Z") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("process {pid} is a zombie"),
        )
        .into());
    }
    // rest[0] is field 3 (state), so field 22 is index 19.
    fields
        .get(19)
        .ok_or_else(|| anyhow::anyhow!("/proc/{pid}/stat has no start time"))?
        .parse()
        .context("invalid process start time")
}

fn process_is_gone(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|error| error.kind() == std::io::ErrorKind::NotFound)
}

/// Metadata for one Firecracker Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketMetadata {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inode: Option<u64>,
    pub uid: u32,
    pub mode: u32,
}

impl SocketMetadata {
    /// Build metadata for an endpoint that has not been created yet. The
    /// journal records the expected path immediately after spawn; callers must
    /// call `validate` after Firecracker creates the socket before attaching.
    pub fn expected(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_string_lossy().into_owned(),
            device: None,
            inode: None,
            uid: current_uid(),
            mode: 0o600,
        }
    }

    pub fn capture(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect runtime socket {}", path.display()))?;
        if metadata.file_type().is_symlink() || !is_socket(&metadata) {
            bail!("runtime endpoint is not a Unix socket: {}", path.display());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let uid = unsafe { libc::geteuid() } as u32;
            if metadata.uid() != uid {
                bail!(
                    "runtime socket is not owned by the current user: {}",
                    path.display()
                );
            }
            Ok(Self {
                path: path.to_string_lossy().into_owned(),
                device: Some(metadata.dev()),
                inode: Some(metadata.ino()),
                uid: metadata.uid(),
                mode: metadata.permissions().mode() & 0o777,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            bail!("Firecracker runtime sockets require a Unix host")
        }
    }

    pub fn validate(&self) -> Result<()> {
        let observed = Self::capture(&self.path)?;
        if observed.uid != self.uid || observed.mode != self.mode {
            bail!("runtime socket ownership or mode changed: {}", self.path);
        }
        if self.device.is_some() && observed.device != self.device {
            bail!("runtime socket device changed: {}", self.path);
        }
        if self.inode.is_some() && observed.inode != self.inode {
            bail!("runtime socket metadata changed: {}", self.path);
        }
        Ok(())
    }

    /// Validate an endpoint when present, while allowing the expected socket
    /// to be absent during startup or explicit cleanup phases. A journal that
    /// already records device/inode identity must never silently accept a
    /// missing endpoint.
    pub fn validate_if_present(&self) -> Result<()> {
        match fs::symlink_metadata(&self.path) {
            Ok(_) => self.validate(),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && self.device.is_none()
                    && self.inode.is_none() =>
            {
                Ok(())
            }
            Err(error) => Err(error)
                .with_context(|| format!("failed to inspect runtime socket {}", self.path)),
        }
    }

    /// Secure an endpoint that is still only an expected startup path.
    ///
    /// Firecracker creates its Unix sockets, so the first journal record cannot
    /// include device/inode identity. During that narrow `Starting` window we
    /// may tighten a current-user-owned socket to 0600 before capturing it.
    /// Once device/inode metadata has been captured, callers must use
    /// [`Self::validate`] and any mode change is an identity failure.
    pub fn secure_expected_if_present(&self) -> Result<()> {
        if self.device.is_some() || self.inode.is_some() {
            return self.validate_if_present();
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !is_socket(&metadata) {
                    bail!("runtime endpoint is not a Unix socket: {}", self.path);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::{MetadataExt, PermissionsExt};
                    let uid = unsafe { libc::geteuid() } as u32;
                    if metadata.uid() != uid {
                        bail!(
                            "runtime socket is not owned by the current user: {}",
                            self.path
                        );
                    }
                    let mut permissions = metadata.permissions();
                    permissions.set_mode(0o600);
                    fs::set_permissions(&self.path, permissions).with_context(|| {
                        format!("failed to secure runtime socket {}", self.path)
                    })?;
                }
                self.validate()
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && self.device.is_none()
                    && self.inode.is_none() =>
            {
                Ok(())
            }
            Err(error) => Err(error)
                .with_context(|| format!("failed to inspect runtime socket {}", self.path)),
        }
    }
}

fn is_socket(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        metadata.file_type().is_socket()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

/// Lifecycle phase recorded before each externally visible runtime action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase {
    Starting,
    Running,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Orphaned,
}

/// Who currently owns the durable handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHandleState {
    Owned,
    Attached,
    Orphaned,
}

/// Durable runtime record. This is independent of SandboxState so it can be
/// written before each Firecracker request and survive state publication
/// interruption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeJournal {
    pub version: u32,
    pub sandbox: String,
    pub runtime_id: String,
    pub phase: RuntimePhase,
    pub handle_state: RuntimeHandleState,
    pub process: ProcessIdentity,
    pub api_socket: SocketMetadata,
    pub vsock_socket: SocketMetadata,
    pub updated_at_unix_ms: u128,
}

impl RuntimeJournal {
    fn validate_shape_only(&self) -> Result<()> {
        if self.version != JOURNAL_VERSION {
            bail!(
                "unsupported Firecracker runtime journal version {}",
                self.version
            );
        }
        if self.sandbox.is_empty() || self.runtime_id.is_empty() || self.process.pid == 0 {
            bail!("Firecracker runtime journal has invalid identity");
        }
        Ok(())
    }
}

/// A held ownership lock and the journal it protects.
pub struct RuntimeSupervisor {
    runtime_dir: PathBuf,
    journal_path: PathBuf,
    _lock: File,
    journal: Option<RuntimeJournal>,
}

impl std::fmt::Debug for RuntimeSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSupervisor")
            .field("runtime_dir", &self.runtime_dir)
            .field("journal_path", &self.journal_path)
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

impl RuntimeSupervisor {
    pub fn create(runtime_dir: impl AsRef<Path>) -> Result<Self> {
        ensure_private_directory(runtime_dir.as_ref())?;
        Self::open_locked(runtime_dir.as_ref(), true)
    }

    pub fn open(runtime_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_locked(runtime_dir.as_ref(), false)
    }

    fn open_locked(runtime_dir: &Path, create: bool) -> Result<Self> {
        if create {
            ensure_private_directory(runtime_dir)?;
        } else {
            verify_private_directory(runtime_dir)?;
        }
        let lock_path = runtime_dir.join(LOCK_FILE);
        let lock = open_private_file(&lock_path, true)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    bail!(
                        "Firecracker runtime ownership lock is held: {}",
                        runtime_dir.display()
                    );
                }
                return Err(error).context("failed to acquire Firecracker runtime ownership lock");
            }
        }
        let journal_path = runtime_dir.join(JOURNAL_FILE);
        let journal = read_journal(&journal_path)?;
        Ok(Self {
            runtime_dir: runtime_dir.to_path_buf(),
            journal_path,
            _lock: lock,
            journal,
        })
    }

    pub fn journal(&self) -> Option<&RuntimeJournal> {
        self.journal.as_ref()
    }

    pub fn record_process(
        &mut self,
        sandbox: &str,
        runtime_id: &str,
        process: ProcessIdentity,
        api_socket: SocketMetadata,
        vsock_socket: SocketMetadata,
        phase: RuntimePhase,
    ) -> Result<RuntimeJournal> {
        let journal = RuntimeJournal {
            version: JOURNAL_VERSION,
            sandbox: sandbox.to_owned(),
            runtime_id: runtime_id.to_owned(),
            phase,
            handle_state: RuntimeHandleState::Owned,
            process,
            api_socket,
            vsock_socket,
            updated_at_unix_ms: now_unix_ms(),
        };
        self.write_journal(journal.clone())?;
        Ok(journal)
    }

    pub fn update_phase(&mut self, phase: RuntimePhase) -> Result<()> {
        let Some(mut journal) = self.journal.clone() else {
            bail!("Firecracker runtime process has not been journaled");
        };
        journal.phase = phase;
        journal.updated_at_unix_ms = now_unix_ms();
        self.write_journal(journal)
    }

    pub fn update_handle_state(&mut self, handle_state: RuntimeHandleState) -> Result<()> {
        let Some(mut journal) = self.journal.clone() else {
            bail!("Firecracker runtime process has not been journaled");
        };
        journal.handle_state = handle_state;
        journal.updated_at_unix_ms = now_unix_ms();
        self.write_journal(journal)
    }

    /// Replace expected socket paths with metadata captured after
    /// Firecracker has created the endpoints.
    pub fn refresh_sockets(&mut self) -> Result<()> {
        let Some(mut journal) = self.journal.clone() else {
            bail!("Firecracker runtime process has not been journaled");
        };
        journal.api_socket = SocketMetadata::capture(&journal.api_socket.path)?;
        journal.vsock_socket = SocketMetadata::capture(&journal.vsock_socket.path)?;
        journal.updated_at_unix_ms = now_unix_ms();
        self.write_journal(journal)
    }

    fn write_journal(&mut self, journal: RuntimeJournal) -> Result<()> {
        journal.validate_shape_only()?;
        atomic_write_json(&self.journal_path, &journal)?;
        self.journal = Some(journal);
        Ok(())
    }

    /// Verify process and socket identity before attaching to an existing VM.
    pub fn verify_attachable(&self) -> Result<&RuntimeJournal> {
        let journal = self
            .journal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Firecracker runtime journal is missing"))?;
        journal.validate_shape_only()?;
        match ProcessIdentity::capture(journal.process.pid) {
            Ok(observed) if !journal.process.matches(&observed) => {
                bail!("Firecracker PID {} identity mismatch", journal.process.pid);
            }
            Ok(_) => {}
            Err(error)
                if process_is_gone(&error)
                    && matches!(
                        journal.phase,
                        RuntimePhase::Stopping | RuntimePhase::Orphaned
                    ) =>
            {
                // An already-exited runtime in an explicit cleanup phase is
                // safe to reopen so the caller can release its journal and
                // private directory; no signal will be sent.
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect Firecracker PID {}", journal.process.pid)
                });
            }
        }
        if journal.phase == RuntimePhase::Starting {
            journal.api_socket.secure_expected_if_present()?;
            journal.vsock_socket.secure_expected_if_present()?;
        } else if matches!(
            journal.phase,
            RuntimePhase::Stopping | RuntimePhase::Orphaned
        ) {
            journal.api_socket.validate_if_present()?;
            journal.vsock_socket.validate_if_present()?;
        } else {
            journal.api_socket.validate()?;
            journal.vsock_socket.validate()?;
        }
        Ok(journal)
    }

    /// Verify only process identity for an owned Child before kill/wait. This
    /// remains usable while Firecracker is shutting sockets down.
    pub fn verify_process_identity(&self) -> Result<()> {
        let journal = self
            .journal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Firecracker runtime journal is missing"))?;
        let observed = ProcessIdentity::capture(journal.process.pid).with_context(|| {
            format!("failed to inspect Firecracker PID {}", journal.process.pid)
        })?;
        if !journal.process.matches(&observed) {
            bail!("Firecracker PID {} identity mismatch", journal.process.pid);
        }
        Ok(())
    }

    /// Mark a journal orphaned after an identity check failed. No signal is
    /// sent and no file is deleted.
    pub fn mark_orphaned(&mut self) -> Result<()> {
        if let Some(mut journal) = self.journal.clone() {
            journal.phase = RuntimePhase::Orphaned;
            journal.handle_state = RuntimeHandleState::Orphaned;
            journal.updated_at_unix_ms = now_unix_ms();
            self.write_journal(journal)?;
        }
        Ok(())
    }

    /// Signal only after proving that the PID is still the original process.
    /// Kill/wait ambiguity leaves the journal orphaned and returns an error.
    pub fn terminate(&mut self, timeout: Duration) -> Result<()> {
        let journal = self
            .journal
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Firecracker runtime journal is missing"))?;
        match ProcessIdentity::capture(journal.process.pid) {
            Ok(observed) if journal.process.matches(&observed) => {}
            Ok(_) => {
                self.mark_orphaned()?;
                bail!(
                    "refusing to terminate a reused Firecracker PID {}",
                    journal.process.pid
                )
            }
            Err(error) if process_is_gone(&error) => {
                let _ = self.update_phase(RuntimePhase::Paused);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        #[cfg(unix)]
        if unsafe { libc::kill(journal.process.pid as libc::pid_t, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            self.mark_orphaned().ok();
            return Err(error).context("failed to terminate Firecracker process");
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match ProcessIdentity::capture(journal.process.pid) {
                Ok(current) if journal.process.matches(&current) => {
                    if std::time::Instant::now() >= deadline {
                        self.mark_orphaned().ok();
                        bail!(
                            "timed out waiting for Firecracker PID {} to exit",
                            journal.process.pid
                        );
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(_) => {
                    self.mark_orphaned().ok();
                    bail!(
                        "Firecracker PID {} changed identity while stopping",
                        journal.process.pid
                    );
                }
                Err(error) if process_is_gone(&error) => {
                    let _ = self.update_phase(RuntimePhase::Paused);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Remove the journal once the manager has durably published stopped or
    /// paused state.
    pub fn release(&mut self) -> Result<()> {
        match fs::symlink_metadata(&self.journal_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!("refusing to remove symlinked Firecracker runtime journal");
                }
                fs::remove_file(&self.journal_path)?;
                sync_directory(&self.runtime_dir)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("failed to inspect runtime journal for release");
            }
        }
        self.journal = None;
        Ok(())
    }

    /// Release the journal and then remove the empty runtime directory after
    /// the ownership lock has been closed. This ordering avoids leaving a
    /// lock file behind or unlinking a directory while it is still owned.
    pub fn release_and_remove(mut self) -> Result<()> {
        let runtime_dir = self.runtime_dir.clone();
        self.release()?;
        drop(self);
        let lock_path = runtime_dir.join(LOCK_FILE);
        if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
            if metadata.file_type().is_symlink() {
                bail!("refusing to remove symlinked Firecracker runtime lock");
            }
            fs::remove_file(lock_path)?;
        }
        fs::remove_dir(&runtime_dir).with_context(|| {
            format!(
                "failed to remove runtime directory {}",
                runtime_dir.display()
            )
        })
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() as u32 }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn read_journal(path: &Path) -> Result<Option<RuntimeJournal>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "Firecracker runtime journal is not a regular file: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o777 != JOURNAL_MODE
        {
            bail!("Firecracker runtime journal must be current-user-owned with mode 0600");
        }
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let journal: RuntimeJournal = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid Firecracker runtime journal {}", path.display()))?;
    journal.validate_shape_only()?;
    Ok(Some(journal))
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to overwrite symlinked Firecracker runtime journal");
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).context("failed to inspect runtime journal for atomic write");
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("runtime journal has no parent directory"))?;
    let temporary = parent.join(format!(".{JOURNAL_FILE}.tmp-{}", Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let write_result = (|| -> Result<()> {
        let mut file = open_private_file(&temporary, false)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn open_private_file(path: &Path, existing: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(JOURNAL_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    if existing {
        options.create(true);
    } else {
        options.create_new(true);
    }
    let file = options
        .open(path)
        .with_context(|| format!("failed to open private runtime file {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!(
            "runtime ownership/journal path is not a regular file: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o777 != JOURNAL_MODE
        {
            bail!(
                "runtime ownership/journal path is not private: {}",
                path.display()
            );
        }
    }
    Ok(file)
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    let created = match fs::create_dir(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create runtime directory {}", path.display()));
        }
    };
    if created {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(RUNTIME_MODE))?;
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "runtime directory is not a real directory: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o777 != RUNTIME_MODE
        {
            bail!(
                "runtime directory must be current-user-owned with mode 0700: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn verify_private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("runtime directory is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "runtime directory is not a real directory: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } as u32
            || metadata.permissions().mode() & 0o777 != RUNTIME_MODE
        {
            bail!(
                "runtime directory must be current-user-owned with mode 0700: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .with_context(|| format!("failed to sync runtime directory {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn process_identity_rejects_pid_reuse_or_missing_strong_fields() {
        let identity = ProcessIdentity::capture(std::process::id()).unwrap();
        assert!(identity.matches(&identity));
        let mut changed = identity.clone();
        if let Some(value) = changed.pid_start_time.as_mut() {
            *value += 1;
        } else {
            changed.executable = Some("/definitely/not-the-runtime".to_string());
        }
        assert!(!identity.matches(&changed));
        let mut no_proof = identity.clone();
        no_proof.boot_id = None;
        no_proof.pid_start_time = None;
        no_proof.executable = None;
        assert!(!identity.matches(&no_proof));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn nonexistent_pid_is_not_claimed_alive_from_boot_identity() {
        let error = ProcessIdentity::capture(99_999_999).unwrap_err();
        assert!(process_is_gone(&error));
    }

    #[test]
    fn lifecycle_phase_update_survives_state_publication_gap() {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let mut supervisor = RuntimeSupervisor::create(&runtime).unwrap();
        let process = ProcessIdentity::capture(std::process::id()).unwrap();
        let api = SocketMetadata::expected(runtime.join("api.sock"));
        let vsock = SocketMetadata::expected(runtime.join("vsock.sock"));
        supervisor
            .record_process(
                "sandbox",
                "0123456789abcdef0123456789abcdef",
                process,
                api,
                vsock,
                RuntimePhase::Starting,
            )
            .unwrap();
        supervisor.update_phase(RuntimePhase::Pausing).unwrap();
        drop(supervisor);

        let reopened = RuntimeSupervisor::open(&runtime).unwrap();
        assert_eq!(
            reopened.journal().map(|journal| journal.phase),
            Some(RuntimePhase::Pausing)
        );
    }

    #[test]
    fn starting_journal_allows_socket_creation_gap() {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let mut supervisor = RuntimeSupervisor::create(&runtime).unwrap();
        supervisor
            .record_process(
                "sandbox",
                "fedcba9876543210fedcba9876543210",
                ProcessIdentity::capture(std::process::id()).unwrap(),
                SocketMetadata::expected(runtime.join("api.sock")),
                SocketMetadata::expected(runtime.join("vsock.sock")),
                RuntimePhase::Starting,
            )
            .unwrap();
        assert!(supervisor.verify_attachable().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn starting_expected_sockets_are_tightened_before_capture() {
        use std::os::unix::net::UnixListener;

        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let api_path = runtime.join("api.sock");
        let vsock_path = runtime.join("vsock.sock");
        let _api = UnixListener::bind(&api_path).unwrap();
        let _vsock = UnixListener::bind(&vsock_path).unwrap();
        for path in [&api_path, &vsock_path] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o666);
            fs::set_permissions(path, permissions).unwrap();
        }

        let mut supervisor = RuntimeSupervisor::create(&runtime).unwrap();
        supervisor
            .record_process(
                "sandbox",
                "fedcba9876543210fedcba9876543210",
                ProcessIdentity::capture(std::process::id()).unwrap(),
                SocketMetadata::expected(&api_path),
                SocketMetadata::expected(&vsock_path),
                RuntimePhase::Starting,
            )
            .unwrap();
        supervisor.verify_attachable().unwrap();

        for path in [&api_path, &vsock_path] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn journal_writes_are_atomic_and_reopenable() {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let supervisor = RuntimeSupervisor::create(&runtime).unwrap();
        let process = ProcessIdentity::capture(std::process::id()).unwrap();
        let api = SocketMetadata {
            path: "/tmp/api.sock".to_string(),
            device: None,
            inode: None,
            uid: unsafe { libc::geteuid() } as u32,
            mode: 0o600,
        };
        let vsock = SocketMetadata {
            path: "/tmp/vsock.sock".to_string(),
            ..api.clone()
        };
        let journal = RuntimeJournal {
            version: JOURNAL_VERSION,
            sandbox: "sandbox".to_string(),
            runtime_id: "runtime".to_string(),
            phase: RuntimePhase::Running,
            handle_state: RuntimeHandleState::Owned,
            process,
            api_socket: api,
            vsock_socket: vsock,
            updated_at_unix_ms: now_unix_ms(),
        };
        atomic_write_json(&supervisor.journal_path, &journal).unwrap();
        drop(supervisor);
        let reopened = RuntimeSupervisor::open(&runtime).unwrap();
        assert_eq!(reopened.journal(), Some(&journal));
        assert_eq!(
            fs::metadata(runtime.join(JOURNAL_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn lock_is_exclusive_until_owner_drops() {
        let temp = TempDir::new().unwrap();
        let runtime = temp.path().join("runtime");
        let owner = RuntimeSupervisor::create(&runtime).unwrap();
        let error = RuntimeSupervisor::open(&runtime).unwrap_err();
        assert!(error.to_string().contains("ownership lock"));
        drop(owner);
        RuntimeSupervisor::open(&runtime).unwrap();
    }
}
