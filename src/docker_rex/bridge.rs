//! Owner-local host relay. The configured socket directory must live under a
//! trusted parent (no attacker-controlled ancestor or untrusted same-UID code).
use std::{
    future::Future,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{self as aio, AsyncRead, AsyncWrite},
    net::{UnixListener, UnixStream},
    sync::oneshot,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

const SOCKET: &str = "rex.sock";
const LOCK: &str = "relay.lock";
const MAX_CONNECTIONS: usize = 16;

fn denied() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

/// This is the immutable owned-boot registration from VMM, not a cache entry
/// resolved by guest name. Do not log or serialize this structure to the UI.
#[derive(Clone, PartialEq, Eq)]
pub struct Receipt {
    id: String,
    started_at: String,
    nonce: [u8; 32],
}

impl Receipt {
    pub fn new(id: String, started_at: String, nonce: [u8; 32]) -> io::Result<Self> {
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
            || started_at.is_empty()
        {
            return Err(denied());
        }
        Ok(Self {
            id,
            started_at,
            nonce,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn started_at(&self) -> &str {
        &self.started_at
    }
}

/// Drop must initiate termination of the remote exec, not merely detach from
/// it. A verified transport that delivers EOF to a cooperative gate may meet
/// this contract; adapter drop is not an acknowledgement of remote exit.
/// Implementations must also clean up an exec if canceled within open_fixed.
pub struct Exec<R, W, Lease> {
    pub stdout: R,
    pub stdin: W,
    pub lease: Lease,
}

pub trait Engine: Send + Sync + 'static {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;
    type Lease: Send + 'static;

    /// Inspect by exact ID; return None if gone, stopped, or not owned.
    fn running_started_at(
        &self,
        full_id: &str,
    ) -> impl Future<Output = io::Result<Option<String>>> + Send;

    /// Exec the fixed gate by exact ID (no command from Rex), no TTY, with
    /// separately demultiplexed stdin/stdout. Never forward stderr. The gate
    /// re-checks the expected nonce inside the guest (inspect/exec race).
    fn open_fixed(
        &self,
        full_id: &str,
    ) -> impl Future<Output = io::Result<Exec<Self::Reader, Self::Writer, Self::Lease>>> + Send;
}

struct Binding {
    path: PathBuf,
    dev: u64,
    ino: u64,
    _lock: std::fs::File,
}

impl Drop for Binding {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.file_type().is_socket() && m.dev() == self.dev && m.ino() == self.ino)
        {
            let _ = std::fs::remove_file(&self.path);
        }
        // Release explicitly: a concurrent fork can transiently inherit this
        // open file description despite close-on-exec.
        // SAFETY: unlock uses the descriptor still owned by this Binding.
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self._lock), libc::LOCK_UN) };
    }
}

fn bind(dir: &Path, uid: u32) -> io::Result<(UnixListener, Binding)> {
    if !dir.is_absolute() || std::fs::canonicalize(dir)? != dir {
        return Err(denied());
    }
    let m = std::fs::symlink_metadata(dir)?;
    if !m.is_dir() || m.uid() != uid || m.mode() & 0o7777 != 0o700 {
        return Err(denied());
    }
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(dir.join(LOCK))?;
    let m = lock.metadata()?;
    if !m.is_file() || m.uid() != uid || m.mode() & 0o7777 != 0o600 || m.nlink() != 1 {
        return Err(denied());
    }
    // SAFETY: flock does not retain this borrowed descriptor; Binding owns it.
    if unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&lock),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let path = dir.join(SOCKET);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_socket() && m.uid() == uid => {
            // Do not unlink a non-cooperating live server, or assume every
            // failed connect means stale.
            let probe = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
            probe.set_nonblocking(true)?;
            match probe.connect(&socket2::SockAddr::unix(&path)?) {
                Err(e) if e.raw_os_error() == Some(libc::ECONNREFUSED) => {
                    std::fs::remove_file(&path)?
                }
                Ok(_) => return Err(io::ErrorKind::AddrInUse.into()),
                Err(e) => return Err(e),
            }
        }
        Ok(_) => return Err(denied()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&path)?;
    let m = std::fs::symlink_metadata(&path)?;
    let binding = Binding {
        path,
        dev: m.dev(),
        ino: m.ino(),
        _lock: lock,
    };
    std::fs::set_permissions(&binding.path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, binding))
}

pub struct Bridge {
    path: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<io::Result<()>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Bridge {
    /// Bind in a pre-created, owner-only directory unique to this persisted
    /// registration. Reopen that directory only for this same registration.
    /// Requires a Tokio runtime. Keep the handle for the registration lifetime.
    pub fn start<E: Engine>(
        dir: &Path,
        receipt: Receipt,
        engine: Arc<E>,
        deadline: Duration,
    ) -> io::Result<Self> {
        // SAFETY: geteuid has no arguments or pointer requirements.
        let uid = unsafe { libc::geteuid() };
        let (listener, binding) = bind(dir, uid)?;
        let path = binding.path.clone();
        let (stop, mut revoked) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _binding = binding;
            let mut active = JoinSet::new();
            let result = loop {
                tokio::select! {
                    biased;
                    _ = &mut revoked => break Ok(()),
                    _ = active.join_next(), if !active.is_empty() => (),
                    conn = listener.accept(), if active.len() < MAX_CONNECTIONS => {
                        let (stream, _) = match conn { Ok(conn) => conn, Err(e) => break Err(e) };
                        if !authorized_peer(&stream, uid) {
                            continue;
                        }
                        let engine = engine.clone();
                        let receipt = receipt.clone();
                        active.spawn(async move {
                            let _ = connect(stream, &receipt, &*engine, deadline).await;
                        });
                    }
                }
            };
            drop(listener);
            active.shutdown().await;
            result
        });
        Ok(Self {
            path,
            stop: Some(stop),
            task,
        })
    }

    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }

    pub fn socket(&self) -> &Path {
        &self.path
    }

    /// Stops accepting, then drops every stream and its Engine lease before
    /// returning. The registration owner must persist deletion before revoke,
    /// so an app restart cannot recreate this endpoint.
    pub async fn revoke(mut self) -> io::Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        (&mut self.task).await.map_err(io::Error::other)?
    }
}

fn authorized_peer(stream: &UnixStream, uid: u32) -> bool {
    stream.peer_cred().is_ok_and(|peer| peer.uid() == uid)
}

async fn connect<E: Engine>(
    stream: UnixStream,
    receipt: &Receipt,
    engine: &E,
    deadline: Duration,
) -> io::Result<()> {
    let ((stdout, stdin), _lease) = timeout(deadline, async {
        if engine.running_started_at(receipt.id()).await?.as_deref() != Some(receipt.started_at()) {
            return Err(denied());
        }
        let Exec {
            stdout,
            stdin,
            lease,
        } = engine.open_fixed(receipt.id()).await?;
        let io = super::authorize_stream(stdout, stdin, &receipt.nonce, deadline).await?;
        Ok::<_, io::Error>((io, lease))
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    let mut client = stream;
    let mut gate = Combined { stdout, stdin };
    aio::copy_bidirectional(&mut client, &mut gate).await?;
    Ok(())
}

struct Combined<R, W> {
    stdout: R,
    stdin: W,
}
impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for Combined<R, W> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut aio::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stdout).poll_read(cx, buf)
    }
}
impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for Combined<R, W> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().stdin).poll_write(cx, data)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stdin).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stdin).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

    const MAX: Duration = Duration::from_secs(2);
    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let dir = PathBuf::from(format!(
                "/tmp/akr-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
            Self(std::fs::canonicalize(dir).unwrap())
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        generation: std::sync::Mutex<String>,
        nonce: u8,
        opens: AtomicUsize,
        kills: Arc<AtomicUsize>,
    }
    impl Fixture {
        fn new(nonce: u8) -> Arc<Self> {
            Arc::new(Self {
                generation: std::sync::Mutex::new("boot-1".into()),
                nonce,
                opens: AtomicUsize::new(0),
                kills: Arc::new(AtomicUsize::new(0)),
            })
        }
    }
    struct Lease(JoinHandle<()>, Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.abort();
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn receipt() -> Receipt {
        Receipt::new("a".repeat(64), "boot-1".into(), [7; 32]).unwrap()
    }
    impl Engine for Fixture {
        type Reader = ReadHalf<DuplexStream>;
        type Writer = WriteHalf<DuplexStream>;
        type Lease = Lease;
        async fn running_started_at(&self, full_id: &str) -> io::Result<Option<String>> {
            assert_eq!(full_id, receipt().id());
            Ok(Some(self.generation.lock().unwrap().clone()))
        }
        async fn open_fixed(
            &self,
            full_id: &str,
        ) -> io::Result<Exec<Self::Reader, Self::Writer, Self::Lease>> {
            assert_eq!(full_id, receipt().id());
            self.opens.fetch_add(1, Ordering::SeqCst);
            let (host, mut guest) = aio::duplex(1024);
            let (stdout, stdin) = aio::split(host);
            let ok = [self.nonce; 32];
            let task = tokio::spawn(async move {
                let mut got = [0; 32];
                guest.read_exact(&mut got).await.unwrap();
                if ok == [0; 32] {
                    std::future::pending::<()>().await;
                }
                if ok != got {
                    return;
                }
                guest.write_all(b"\x01marker").await.unwrap();
                let mut request = [0; 4];
                if guest.read_exact(&mut request).await.is_ok() {
                    guest.write_all(&request).await.unwrap();
                }
                std::future::pending::<()>().await;
            });
            Ok(Exec {
                stdout,
                stdin,
                lease: Lease(task, self.kills.clone()),
            })
        }
    }
    async fn marker(bridge: &Bridge) -> UnixStream {
        let mut client = UnixStream::connect(bridge.socket()).await.unwrap();
        let mut out = [0; 6];
        timeout(MAX, client.read_exact(&mut out))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&out, b"marker");
        client
    }
    async fn closed(mut client: UnixStream) {
        assert!(matches!(
            timeout(MAX, client.read(&mut [0])).await.unwrap(),
            Ok(0) | Err(_)
        ));
    }

    #[tokio::test]
    async fn forward_same_uid_then_revoke_cancels_and_removes_endpoint() {
        let dir = Dir::new();
        let e = Fixture::new(7);
        let bridge = Bridge::start(&dir.0, receipt(), e.clone(), MAX).unwrap();
        let socket = bridge.socket().to_owned();
        assert_eq!(
            std::fs::symlink_metadata(&socket).unwrap().mode() & 0o7777,
            0o600
        );
        assert!(Bridge::start(&dir.0, receipt(), e.clone(), MAX).is_err());
        let mut client = marker(&bridge).await;
        assert!(!authorized_peer(
            &client,
            unsafe { libc::geteuid() }.wrapping_add(1)
        ));
        client.write_all(b"ping").await.unwrap();
        let mut out = [0; 4];
        timeout(MAX, client.read_exact(&mut out))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&out, b"ping");
        bridge.revoke().await.unwrap();
        assert_eq!(e.kills.load(Ordering::SeqCst), 1);
        closed(client).await;
        assert!(UnixStream::connect(&socket).await.is_err());
    }

    #[tokio::test]
    async fn same_name_cannot_redirect_and_same_id_new_generation_stops_before_exec() {
        let dir = Dir::new();
        let e = Fixture::new(7);
        let bridge = Bridge::start(&dir.0, receipt(), e.clone(), MAX).unwrap();
        *e.generation.lock().unwrap() = "boot-2".into();
        closed(UnixStream::connect(bridge.socket()).await.unwrap()).await;
        assert_eq!(e.opens.load(Ordering::SeqCst), 0);
        bridge.revoke().await.unwrap();
    }

    #[tokio::test]
    async fn restart_keeps_path_for_same_receipt_and_rejects_nonce_mismatch() {
        let dir = Dir::new();
        let e = Fixture::new(7);
        let first = Bridge::start(&dir.0, receipt(), e, MAX).unwrap();
        let socket = first.socket().to_owned();
        first.revoke().await.unwrap();
        let stale = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        drop(stale);
        let e = Fixture::new(7);
        let second = Bridge::start(&dir.0, receipt(), e, MAX).unwrap();
        assert_eq!(second.socket(), socket);
        let client = marker(&second).await;
        second.revoke().await.unwrap();
        closed(client).await;

        let e = Fixture::new(8);
        let changed = Bridge::start(&dir.0, receipt(), e.clone(), MAX).unwrap();
        closed(UnixStream::connect(changed.socket()).await.unwrap()).await;
        assert_eq!(e.kills.load(Ordering::SeqCst), 1);
        changed.revoke().await.unwrap();
    }

    #[tokio::test]
    async fn revoke_during_pending_ack_cancels_the_exec() {
        let dir = Dir::new();
        let e = Fixture::new(0);
        let bridge = Bridge::start(&dir.0, receipt(), e.clone(), MAX).unwrap();
        let client = UnixStream::connect(bridge.socket()).await.unwrap();
        timeout(MAX, async {
            while e.opens.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        bridge.revoke().await.unwrap();
        assert_eq!(e.kills.load(Ordering::SeqCst), 1);
        closed(client).await;
    }

    #[tokio::test]
    async fn denies_public_and_symlink_directory_and_non_socket() {
        let dir = Dir::new();
        let e = Fixture::new(7);
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Bridge::start(&dir.0, receipt(), e.clone(), MAX).is_err());
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&dir.0, dir.0.join("alias")).unwrap();
        assert!(Bridge::start(&dir.0.join("alias"), receipt(), e.clone(), MAX).is_err());
        std::fs::write(dir.0.join(SOCKET), b"do not unlink").unwrap();
        assert!(Bridge::start(&dir.0, receipt(), e, MAX).is_err());
        assert_eq!(std::fs::read(dir.0.join(SOCKET)).unwrap(), b"do not unlink");
    }
}
