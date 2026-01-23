//! PTY (Pseudo-Terminal) Support
//!
//! Provides PTY allocation and process spawning for interactive shell sessions
//! inside the guest VM. Uses `openpty()` for PTY creation and `fork()`/`exec()`
//! for process spawning.

use anyhow::{Context, Result, bail};
use nix::pty::{openpty, OpenptyResult};
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitStatus, waitpid, WaitPidFlag};
use nix::unistd::{ForkResult, Pid, close, dup2, fork, setsid};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// A PTY session managing an interactive shell process
pub struct PtySession {
    /// Session identifier
    pub id: String,
    /// Master PTY file descriptor
    master_fd: RawFd,
    /// Child process PID
    child_pid: Option<Pid>,
    /// Async file wrapper for the master fd
    master_file: Option<tokio::fs::File>,
}

impl PtySession {
    /// Spawn a new PTY session with the given command
    ///
    /// # Arguments
    /// * `id` - Unique session identifier
    /// * `command` - Command to run (e.g., "/bin/sh")
    /// * `args` - Command arguments
    /// * `rows` - Initial terminal rows
    /// * `cols` - Initial terminal columns
    /// * `env` - Environment variables
    pub fn spawn(
        id: String,
        command: &str,
        args: &[String],
        rows: u16,
        cols: u16,
        env: Option<&HashMap<String, String>>,
    ) -> Result<Self> {
        // Open a new PTY pair
        let OpenptyResult { master, slave } = openpty(None, None)
            .context("Failed to open PTY")?;

        // Set initial window size
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            if libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) < 0 {
                eprintln!("Warning: Failed to set window size");
            }
        }

        // Fork the process
        match unsafe { fork() }.context("Fork failed")? {
            ForkResult::Child => {
                // Child process: set up PTY and exec

                // Close the master fd in child
                let _ = close(master.as_raw_fd());

                // Create a new session (become session leader)
                setsid().ok();

                // Set the slave as the controlling terminal
                #[allow(clippy::useless_conversion)]
                unsafe {
                    libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY.into(), 0);
                }

                // Redirect stdin/stdout/stderr to the slave PTY
                let slave_fd = slave.as_raw_fd();
                dup2(slave_fd, 0).expect("dup2 stdin failed");
                dup2(slave_fd, 1).expect("dup2 stdout failed");
                dup2(slave_fd, 2).expect("dup2 stderr failed");

                // Close the original slave fd if it's not one of stdin/stdout/stderr
                if slave_fd > 2 {
                    let _ = close(slave_fd);
                }

                // Prepare command and arguments
                let cmd = CString::new(command).expect("Invalid command");
                let mut c_args: Vec<CString> = vec![cmd.clone()];
                for arg in args {
                    c_args.push(CString::new(arg.as_str()).expect("Invalid argument"));
                }

                // Set environment variables
                if let Some(env_vars) = env {
                    for (key, value) in env_vars {
                        let env_str = format!("{}={}", key, value);
                        if let Ok(c_env) = CString::new(env_str) {
                            unsafe {
                                libc::putenv(c_env.into_raw());
                            }
                        }
                    }
                }

                // Set some default environment variables for a usable shell
                for (key, value) in [
                    ("TERM", "xterm-256color"),
                    ("HOME", "/root"),
                    ("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"),
                ] {
                    if env.map_or(true, |e| !e.contains_key(key)) {
                        let env_str = format!("{}={}", key, value);
                        if let Ok(c_env) = CString::new(env_str) {
                            unsafe {
                                libc::putenv(c_env.into_raw());
                            }
                        }
                    }
                }

                // Exec the command
                nix::unistd::execvp(&cmd, &c_args).expect("execvp failed");
                unreachable!()
            }
            ForkResult::Parent { child } => {
                // Parent process: close slave, return session
                let _ = close(slave.as_raw_fd());

                // Create async file wrapper for the master fd
                let master_fd = master.as_raw_fd();
                let master_file = unsafe {
                    std::fs::File::from_raw_fd(master_fd)
                };
                let master_file = tokio::fs::File::from_std(master_file);

                Ok(Self {
                    id,
                    master_fd,
                    child_pid: Some(child),
                    master_file: Some(master_file),
                })
            }
        }
    }

    /// Resize the terminal window
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let result = unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize)
        };
        if result < 0 {
            bail!("Failed to resize terminal: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Write data to the PTY (input to the process)
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if let Some(ref mut file) = self.master_file {
            file.write_all(data).await.context("Failed to write to PTY")?;
            file.flush().await.context("Failed to flush PTY")?;
        }
        Ok(())
    }

    /// Read data from the PTY (output from the process)
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if let Some(ref mut file) = self.master_file {
            let n = file.read(buf).await.context("Failed to read from PTY")?;
            Ok(n)
        } else {
            Ok(0)
        }
    }

    /// Check if the child process is still running
    pub fn is_running(&self) -> bool {
        if let Some(pid) = self.child_pid {
            match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => true,
                Ok(_) => false,
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Get the exit code if the process has exited
    pub fn exit_code(&self) -> Option<i32> {
        if let Some(pid) = self.child_pid {
            match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(_, code)) => Some(code),
                Ok(WaitStatus::Signaled(_, signal, _)) => Some(128 + signal as i32),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Kill the child process
    pub fn kill(&self) -> Result<()> {
        if let Some(pid) = self.child_pid {
            kill(pid, Signal::SIGKILL).context("Failed to kill process")?;
        }
        Ok(())
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.id
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Kill the child process if still running
        if let Some(pid) = self.child_pid.take() {
            let _ = kill(pid, Signal::SIGTERM);
            // Give it a moment to exit gracefully
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = kill(pid, Signal::SIGKILL);
        }
        // The master_file will close the fd when dropped
    }
}

/// Session manager for multiple PTY sessions
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new PTY session
    pub async fn create_session(
        &self,
        command: &str,
        args: &[String],
        rows: u16,
        cols: u16,
        env: Option<&HashMap<String, String>>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let session = PtySession::spawn(id.clone(), command, args, rows, cols, env)?;

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), session);

        Ok(id)
    }

    /// Get a session by ID
    pub async fn get_session(&self, id: &str) -> Option<Arc<Mutex<HashMap<String, PtySession>>>> {
        let sessions = self.sessions.lock().await;
        if sessions.contains_key(id) {
            Some(self.sessions.clone())
        } else {
            None
        }
    }

    /// Write input to a session
    pub async fn write_to_session(&self, id: &str, data: &[u8]) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id) {
            session.write(data).await
        } else {
            bail!("Session not found: {}", id)
        }
    }

    /// Read output from a session
    pub async fn read_from_session(&self, id: &str, buf: &mut [u8]) -> Result<usize> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id) {
            session.read(buf).await
        } else {
            bail!("Session not found: {}", id)
        }
    }

    /// Resize a session's terminal
    pub async fn resize_session(&self, id: &str, rows: u16, cols: u16) -> Result<()> {
        let sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(id) {
            session.resize(rows, cols)
        } else {
            bail!("Session not found: {}", id)
        }
    }

    /// Close a session
    pub async fn close_session(&self, id: &str) -> Result<Option<i32>> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.remove(id) {
            Ok(session.exit_code())
        } else {
            bail!("Session not found: {}", id)
        }
    }

    /// Check if a session exists
    pub async fn has_session(&self, id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.contains_key(id)
    }

    /// Get exit code for a session (if exited)
    pub async fn get_exit_code(&self, id: &str) -> Option<i32> {
        let sessions = self.sessions.lock().await;
        sessions.get(id).and_then(|s| s.exit_code())
    }

    /// Check if a session is still running
    pub async fn is_session_running(&self, id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.get(id).map(|s| s.is_running()).unwrap_or(false)
    }

    /// List all session IDs
    pub async fn list_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.lock().await;
        sessions.keys().cloned().collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_creation() {
        let manager = SessionManager::new();
        assert!(manager.sessions.try_lock().is_ok());
    }
}
