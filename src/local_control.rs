//! Secure local control-plane transport selection and Unix socket lifecycle.
//!
//! The Firecracker CLI/MCP delegation path normally uses the loopback HTTP
//! listener.  A Unix-domain socket can be selected explicitly with
//! `AGENTKERNEL_CONTROL_SOCKET` (or `agentkernel serve --control-socket`).
//! An explicitly selected socket is never replaced with a TCP request when it
//! is unavailable.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Environment variable used by CLI/MCP clients to select the local control
/// socket.  The server's `--control-socket` option should use the same value
/// for clients launched in another process.
pub const CONTROL_SOCKET_ENV: &str = "AGENTKERNEL_CONTROL_SOCKET";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Tcp { host: String, port: u16 },
    Unix { path: PathBuf },
}

impl Transport {
    pub fn select(host: &str, port: u16) -> Result<Self> {
        Self::select_with_socket(host, port, configured_socket_path()?)
    }

    pub fn select_with_socket(host: &str, port: u16, socket: Option<PathBuf>) -> Result<Self> {
        if let Some(path) = socket {
            if !path.is_absolute() {
                bail!(
                    "{CONTROL_SOCKET_ENV} must be an absolute Unix socket path (got '{}')",
                    path.display()
                );
            }
            #[cfg(not(unix))]
            bail!(
                "{CONTROL_SOCKET_ENV} is configured, but Unix-domain local control transport is not supported on this platform"
            );
            return Ok(Self::Unix { path });
        }
        Ok(Self::Tcp {
            host: host.to_string(),
            port,
        })
    }
}

/// Resolve the explicitly configured socket path.
pub fn configured_socket_path() -> Result<Option<PathBuf>> {
    let Some(raw) = std::env::var_os(CONTROL_SOCKET_ENV) else {
        return Ok(None);
    };
    let raw = raw.to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    #[cfg(not(unix))]
    {
        bail!(
            "{CONTROL_SOCKET_ENV} is configured, but Unix-domain local control transport is not supported on this platform"
        );
    }
    let path = parse_socket_path(raw)?;
    Ok(Some(path))
}

pub fn parse_socket_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        bail!(
            "{CONTROL_SOCKET_ENV} must be an absolute Unix socket path (got '{}')",
            path.display()
        );
    }
    Ok(path)
}

#[cfg(unix)]
pub fn uri_for_path(transport: &Transport, path: &str) -> Result<hyper::Uri> {
    match transport {
        Transport::Tcp { host, port } => format!("http://{host}:{port}{path}")
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid local control URI: {error}")),
        Transport::Unix { path: socket } => {
            use std::os::unix::fs::FileTypeExt;
            if let Ok(metadata) = std::fs::symlink_metadata(socket)
                && (!metadata.file_type().is_socket() || metadata.file_type().is_symlink())
            {
                bail!(
                    "refusing to connect to non-socket local control path {}",
                    socket.display()
                );
            }
            Ok(hyperlocal::Uri::new(socket, path).into())
        }
    }
}

#[cfg(not(unix))]
pub fn uri_for_path(transport: &Transport, path: &str) -> Result<hyper::Uri> {
    match transport {
        Transport::Tcp { host, port } => format!("http://{host}:{port}{path}")
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid local control URI: {error}")),
        Transport::Unix { .. } => {
            bail!("Unix-domain local control transport is not supported on this platform")
        }
    }
}

/// Metadata captured at bind time so cleanup cannot remove a path that was
/// replaced after the server stopped owning it.
#[cfg(unix)]
#[derive(Debug)]
pub struct SocketCleanup {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if !metadata.file_type().is_socket()
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
        {
            return;
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bind a private (`0600`) Unix socket, removing only a confirmed stale socket.
/// Symlinks and unrelated files are always rejected and left untouched.
#[cfg(unix)]
pub fn bind_secure_socket(path: &Path) -> Result<(tokio::net::UnixListener, SocketCleanup)> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    if !path.is_absolute() {
        bail!(
            "local control socket path must be absolute: {}",
            path.display()
        );
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("local control socket has no parent directory"))?;
    ensure_socket_parent(parent)?;

    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            bail!(
                "refusing to replace symlink at local control socket {}",
                path.display()
            );
        }
        if !file_type.is_socket() {
            bail!(
                "refusing to replace non-socket file at local control socket {}",
                path.display()
            );
        }

        // A successful connect means another server still owns the endpoint.
        // Any connection error is treated as stale, but re-check the exact
        // inode before unlinking so a replacement is never removed.
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            bail!("local control socket is already in use: {}", path.display());
        }
        let current = std::fs::symlink_metadata(path).with_context(|| {
            format!(
                "failed to recheck stale local control socket {}",
                path.display()
            )
        })?;
        if current.file_type().is_symlink()
            || !current.file_type().is_socket()
            || current.dev() != metadata.dev()
            || current.ino() != metadata.ino()
        {
            bail!(
                "local control socket changed while checking stale endpoint: {}",
                path.display()
            );
        }
        std::fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove stale local control socket {}",
                path.display()
            )
        })?;
    }

    let listener = tokio::net::UnixListener::bind(path)
        .with_context(|| format!("failed to bind local control socket {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "failed to set permissions on local control socket {}",
            path.display()
        )
    })?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect local control socket {}", path.display()))?;
    if !metadata.file_type().is_socket() {
        bail!(
            "local control path is not a socket after bind: {}",
            path.display()
        );
    }
    Ok((
        listener,
        SocketCleanup {
            path: path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
        },
    ))
}

#[cfg(unix)]
fn ensure_socket_parent(parent: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::symlink_metadata(parent) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing local control socket through symlinked directory {}",
                parent.display()
            );
        }
        if !metadata.file_type().is_dir() {
            bail!(
                "local control socket parent is not a directory: {}",
                parent.display()
            );
        }
        return Ok(());
    }
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create local control socket directory {}",
            parent.display()
        )
    })?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).with_context(
        || {
            format!(
                "failed to set permissions on local control socket directory {}",
                parent.display()
            )
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Transport, parse_socket_path};

    #[test]
    fn transport_defaults_to_tcp() {
        assert_eq!(
            Transport::select_with_socket("127.0.0.1", 18888, None).unwrap(),
            Transport::Tcp {
                host: "127.0.0.1".to_string(),
                port: 18888
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_socket_selects_unix_without_tcp_fallback() {
        let socket = std::path::PathBuf::from("/tmp/agentkernel-control.sock");
        assert_eq!(
            Transport::select_with_socket("127.0.0.1", 18888, Some(socket.clone())).unwrap(),
            Transport::Unix { path: socket },
        );
    }

    #[test]
    fn socket_path_must_be_absolute() {
        assert!(parse_socket_path("relative.sock").is_err());
    }
}

#[cfg(all(test, unix))]
mod unix_socket_tests {
    use super::bind_secure_socket;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use tempfile::TempDir;

    #[tokio::test]
    async fn socket_is_private_and_cleanup_is_owned() {
        let temp = TempDir::new().unwrap();
        let socket = temp.path().join("control.sock");
        let (listener, cleanup) = bind_secure_socket(&socket).unwrap();
        let metadata = std::fs::symlink_metadata(&socket).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        drop(listener);
        drop(cleanup);
        assert!(!socket.exists());
    }

    #[tokio::test]
    async fn stale_socket_is_removed_but_regular_file_and_symlink_are_preserved() {
        let temp = TempDir::new().unwrap();
        let stale = temp.path().join("stale.sock");
        let listener = tokio::net::UnixListener::bind(&stale).unwrap();
        drop(listener);
        let (_listener, cleanup) = bind_secure_socket(&stale).unwrap();
        drop(cleanup);

        let file = temp.path().join("regular");
        std::fs::write(&file, b"keep").unwrap();
        assert!(bind_secure_socket(&file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");

        let target = temp.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        let link = temp.path().join("link.sock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(bind_secure_socket(&link).is_err());
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }
}
