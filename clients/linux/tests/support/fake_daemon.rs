//! Fake daemon speaking IPC protocol v1, for CLI and GUI client tests.
//!
//! It binds a 0600 socket in a private directory and answers each request
//! line with what the test's `answer` function returns (a JSON value written
//! as one frame), so client behaviour on unusual answers (another protocol
//! major, errors, slow answers) can be tested without the real daemon.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

/// Answers one request (`None`: say nothing).
pub type Answer = Arc<dyn Fn(&Value) -> Option<Value> + Send + Sync>;

/// A running fake daemon; stops when dropped.
pub struct FakeDaemon {
    pub socket: PathBuf,
    stop: Arc<AtomicBool>,
}

impl FakeDaemon {
    /// Listen at `socket` (its directory is created 0700).
    pub fn start(socket: &Path, answer: Answer) -> Self {
        let dir = socket.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let answer = answer.clone();
                        std::thread::spawn(move || {
                            stream.set_nonblocking(false).unwrap();
                            let mut writer = stream.try_clone().unwrap();
                            for line in BufReader::new(stream).lines() {
                                let Ok(line) = line else { break };
                                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                                    continue;
                                };
                                if let Some(reply) = answer(&request) {
                                    let mut frame = reply.to_string();
                                    frame.push('\n');
                                    if writer.write_all(frame.as_bytes()).is_err() {
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            socket: socket.to_path_buf(),
            stop,
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.socket);
    }
}
