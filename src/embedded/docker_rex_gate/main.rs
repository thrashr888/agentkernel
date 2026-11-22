//! Docker-only guest helper. No Apple mode changes.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem::ManuallyDrop,
    os::fd::FromRawFd,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    process::Command,
};

mod supervisor;

const DIR: &str = "/run/agentkernel-rex-boot";
const MARKER: &str = "/run/agentkernel-rex-boot/boot";
const RELAY: &str = "/usr/libexec/agentkernel-rex-relay";
const NONCE_LEN: usize = 32;

fn invalid() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

fn check_root_node(path: &str, dir: bool, mode: u32) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    let kind_ok = if dir { m.is_dir() } else { m.is_file() };
    if !kind_ok || m.uid() != 0 || m.mode() & 0o7777 != mode {
        return Err(invalid());
    }
    Ok(())
}

// Entrypoint must invoke exactly once per boot before readiness, as root,
// with a fresh mode-0755, uid-0 tmpfs mounted at DIR. Never reuse the mount.
fn init() -> io::Result<()> {
    check_root_node(DIR, true, 0o755)?;
    let mut nonce = [0; NONCE_LEN];
    File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o444)
        .open(MARKER)?;
    file.write_all(&nonce)?;
    file.sync_all()?;
    check_root_node(MARKER, false, 0o444)
}

/// Owner-only exact boot nonce capture, invoked by pinned Docker full ID.
fn owner_boot() -> io::Result<()> {
    // SAFETY: geteuid has no arguments or pointer requirements.
    if unsafe { libc::geteuid() } != 0 {
        return Err(invalid());
    }
    check_root_node(DIR, true, 0o755)?;
    match fs::symlink_metadata(MARKER) {
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => init()?,
        Err(error) => return Err(error),
    }
    check_root_node(MARKER, false, 0o444)?;
    let mut marker = [0u8; NONCE_LEN];
    let mut file = File::open(MARKER)?;
    file.read_exact(&mut marker)?;
    if file.read(&mut [0])? != 0 {
        return Err(invalid());
    }
    let mut output = io::stdout().lock();
    output.write_all(&marker)?;
    output.flush()
}

fn gated<T>(
    marker: Option<&[u8]>,
    input: &mut impl Read,
    start: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some(marker) = marker.filter(|m| m.len() == NONCE_LEN) else {
        return Err(invalid());
    };
    let mut expected = [0; NONCE_LEN];
    input.read_exact(&mut expected).map_err(|_| invalid())?;
    if expected != marker {
        return Err(invalid());
    }
    start()
}

fn relay() -> io::Result<()> {
    check_root_node(DIR, true, 0o755)?;
    check_root_node(MARKER, false, 0o444)?;
    let mut marker = Vec::with_capacity(NONCE_LEN + 1);
    File::open(MARKER)?
        .take((NONCE_LEN + 1) as u64)
        .read_to_end(&mut marker)?;
    // Read directly from fd 0; Stdin's internal buffering could read ahead
    // and discard Rex input across exec. The next byte belongs to Rex.
    // SAFETY: process descriptor 0 is inherited; ManuallyDrop does not take
    // ownership of it, so it remains open for the fixed helper after exec.
    let mut input = ManuallyDrop::new(unsafe { File::from_raw_fd(0) });
    gated(Some(&marker), &mut *input, || Ok(()))?;
    {
        // Host must consume this fixed one-byte acknowledgement before
        // forwarding output to Rex. It proves the nonce check, not exec success.
        let mut output = io::stdout().lock();
        output.write_all(&[1])?;
        output.flush()?;
        drop(output);
        supervisor::supervise(&mut *input, &mut Command::new(RELAY))
    }
}

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let result = match args.as_slice() {
        [mode] if mode == "--init" => init(),
        [mode] if mode == "--owner-boot" => owner_boot(),
        [] => relay(),
        _ => Err(invalid()),
    };
    if result.is_err() {
        // Never write stderr/stdout to avoid leaking guest data or poisoning Rex.
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_wrong_stale_or_truncated_never_start_rex() {
        let current = [7; NONCE_LEN];
        let stale = [6; NONCE_LEN];
        let wrong = [0; NONCE_LEN];
        let cases: [(Option<&[u8]>, &[u8]); 5] = [
            (None, &current),
            (Some(&current), &wrong),
            (Some(&current), &stale),
            (Some(&current), &current[..5]),
            (Some(&current[..3]), &current),
        ];
        for (marker, mut input) in cases {
            let mut bytes = Vec::<u8>::new();
            assert!(gated(marker, &mut input, || {
                bytes.extend(b"Rex");
                Ok(())
            })
            .is_err());
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn valid_consumes_exact_header_and_opens_rex() {
        let current = [7; NONCE_LEN];
        let mut wire = current.to_vec();
        wire.extend(b"unread Rex bytes");
        let mut input = wire.as_slice();
        let mut bytes = Vec::<u8>::new();
        assert!(gated(Some(&current), &mut input, || {
            bytes.extend(b"Rex");
            Ok(())
        })
        .is_ok());
        assert_eq!(bytes, b"Rex");
        assert_eq!(input, b"unread Rex bytes");
    }
}
