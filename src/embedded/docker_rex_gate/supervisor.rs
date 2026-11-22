//! Fixed guest relay supervision. Detected attach EOF is a terminal revocation,
//! not a client write-half close: the host must keep stdin open until done.
//! A daemon which fails to propagate EOF still needs a separate bounded lease.
use std::{
    io::{self, Read, Write},
    os::fd::{AsRawFd, RawFd},
    os::unix::process::CommandExt,
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

struct Group(Option<Child>);

impl Drop for Group {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // SAFETY: the spawned child leads this dedicated process group.
            // It has not been reaped, so its PID cannot have been reused.
            // This does not reach descendants that escape the process group.
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
            let _ = child.wait();
        }
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is borrowed from the child-stdin handle held by caller.
    let original = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if original < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, original | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn supervise<R: Read + AsRawFd>(input: &mut R, command: &mut Command) -> io::Result<()> {
    supervise_with_stall_limit(input, command, Duration::from_secs(5))
}

fn supervise_with_stall_limit<R: Read + AsRawFd>(
    input: &mut R,
    command: &mut Command,
    stall: Duration,
) -> io::Result<()> {
    let child = command
        .env_clear()
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .spawn()?;
    let mut child = Group(Some(child));
    let mut to_child: ChildStdin = child.0.as_mut().unwrap().stdin.take().unwrap();
    poll_stream(input, &mut to_child, &mut child, stall)
}

fn poll_stream<R: Read + AsRawFd>(
    input: &mut R,
    out: &mut ChildStdin,
    child: &mut Group,
    stall: Duration,
) -> io::Result<()> {
    set_nonblocking(out.as_raw_fd())?;
    let mut pending = [0; 8192];
    let (mut start, mut end) = (0, 0);
    let mut last_progress = Instant::now();
    loop {
        // A full pipe may mask EOF on some poll implementations. Bound
        // backpressure only; an idle attachment or a progressing session
        // has no wall-clock cap.
        if start != end && last_progress.elapsed() >= stall {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut fds = [
            libc::pollfd {
                fd: input.as_raw_fd(),
                events: if start == end { libc::POLLIN } else { 0 },
                revents: 0,
            },
            libc::pollfd {
                fd: out.as_raw_fd(),
                events: if start != end { libc::POLLOUT } else { 0 },
                revents: 0,
            },
        ];
        // SAFETY: array memory is valid for two entries during this call.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 250) };
        if ready < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        // A closed Docker stdin is the end of this grant. Do not deliver queued
        // client bytes after seeing it, even if the relay stopped reading.
        let closed = libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;
        if fds[0].revents & closed != 0 || fds[1].revents & closed != 0 {
            return Ok(());
        }
        if start == end && fds[0].revents & libc::POLLIN != 0 {
            match input.read(&mut pending) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    start = 0;
                    end = n;
                    last_progress = Instant::now();
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        if start != end
            && (fds[1].revents & libc::POLLOUT != 0 || fds[0].revents & libc::POLLIN != 0)
        {
            match out.write(&pending[start..end]) {
                Ok(n) if n > 0 => {
                    start += n;
                    last_progress = Instant::now();
                }
                Ok(_) => (),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        // Detect a gone relay without waiting for client input. The group
        // isn't signalled after wait has reaped its leader.
        if child.0.as_mut().unwrap().try_wait()?.is_some() {
            child.0.take();
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::pipe, sync::mpsc, thread, time::Duration};

    #[test]
    fn idle_attachment_stays_alive_and_eof_reaps_relay() {
        let (mut read, write) = pipe().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            tx.send(supervise(&mut read, Command::new("/bin/sleep").arg("60")))
                .unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(350)).is_err());
        drop(write);
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn pending_client_bytes_cannot_mask_hangup_indefinitely() {
        let (mut read, mut write) = pipe().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            tx.send(supervise_with_stall_limit(
                &mut read,
                Command::new("/bin/sleep").arg("60"),
                Duration::from_millis(700),
            ))
            .unwrap();
        });
        set_nonblocking(write.as_raw_fd()).unwrap();
        let data = [1; 8192];
        let start = std::time::Instant::now();
        let mut written = 0;
        while start.elapsed() < Duration::from_millis(350) {
            match write.write(&data) {
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        }
        assert!(written > 0);
        assert!(rx.try_recv().is_err());
        drop(write);
        if let Err(e) = rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        }
        worker.join().unwrap();
    }
}
