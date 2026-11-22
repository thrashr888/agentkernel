//! Opt-in owner-local Rex transport for the vetted M5 Docker Desktop path.
//!
//! Requires a short, canonical owner-only endpoint root and explicit owner
//! inputs: a trusted local Docker CLI/socket, immutable prepared image ID,
//! and trusted SHA-256 values for the local ARM64 Rex, gate, and relay bytes.
//! Prepared guest contract: /usr/libexec/rex-eof-guest reads the root-owned
//! boot marker at /run/agentkernel-rex-boot/boot and runs the fixed
//! /usr/libexec/agentkernel-rex-relay; Rex owns its separate 0700 socket
//! directory /run/agentkernel-rex as uid/gid 1000. Provision these out of
//! band. This module never downloads assets, registers Rex hosts, or exposes
//! an HTTP/SSH surface. Revocation closes local streams and initiates guest
//! EOF on the vetted path; it does not acknowledge guest exit.
use std::{io, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

const ACK: u8 = 1;

/// Run only on the owned Engine exec's demultiplexed non-TTY stdout and stdin.
/// Never attach the client socket or forward stderr before this returns success.
/// Takes ownership so cancellation, timeout or denial cannot return a stream.
/// The Engine adapter still owns exec cancellation and the validated boot receipt.
async fn authorize_stream<R, W>(
    mut stdout: R,
    mut stdin: W,
    nonce: &[u8; 32],
    deadline: Duration,
) -> io::Result<(R, W)>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    timeout(deadline, async {
        stdin.write_all(nonce).await?;
        stdin.flush().await?;
        let mut reply = [0];
        stdout.read_exact(&mut reply).await?;
        if reply[0] != ACK {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok((stdout, stdin))
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

// The crate exposes only the typed owner-local boundary, not the raw Engine.
mod bridge;
mod docker_engine;
mod registry;

pub use registry::{OwnerLocalRex, PreparedGuestAssets, VerifiedOwnedBoot};
