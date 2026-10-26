//! Fake daemon speaking IPC protocol v1, for CLI and GUI client tests.
//!
//! It binds a 0600 socket in a private directory and answers each request
//! line with what the test's function returns (JSON values written as
//! frames), so client behaviour on unusual answers (another protocol major,
//! errors, slow answers) can be tested without the real daemon. A reply may
//! be several frames (a `state.subscribe` answer followed by a `state`
//! event). Dropping the fake closes every connection, as a dying daemon
//! would.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Answers one request (`None`: say nothing).
pub type Answer = Arc<dyn Fn(&Value) -> Option<Value> + Send + Sync>;

/// Answers one request with any number of frames.
pub type Replies = Arc<dyn Fn(&Value) -> Vec<Value> + Send + Sync>;

/// A running fake daemon; stops when dropped.
pub struct FakeDaemon {
    pub socket: PathBuf,
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<UnixStream>>>,
}

impl FakeDaemon {
    /// Listen at `socket` (its directory is created 0700).
    pub fn start(socket: &Path, answer: Answer) -> Self {
        Self::start_with(
            socket,
            Arc::new(move |request: &Value| answer(request).into_iter().collect()),
        )
    }

    /// Listen at `socket`, answering each request with `replies`.
    pub fn start_with(socket: &Path, replies: Replies) -> Self {
        let dir = socket.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let streams: Arc<Mutex<Vec<UnixStream>>> = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = stop.clone();
        let thread_streams = streams.clone();
        std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Ok(clone) = stream.try_clone() {
                            thread_streams.lock().unwrap().push(clone);
                        }
                        let replies = replies.clone();
                        std::thread::spawn(move || {
                            stream.set_nonblocking(false).unwrap();
                            let mut writer = stream.try_clone().unwrap();
                            for line in BufReader::new(stream).lines() {
                                let Ok(line) = line else { break };
                                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                                    continue;
                                };
                                for reply in replies(&request) {
                                    let mut frame = reply.to_string();
                                    frame.push('\n');
                                    if writer.write_all(frame.as_bytes()).is_err() {
                                        return;
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
            streams,
        }
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.socket);
        for stream in self.streams.lock().unwrap().drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// The `hello` answer of a protocol-1 daemon.
pub fn hello_result(id: &Value) -> Value {
    json!({
        "v": 1, "id": id,
        "result": {
            "protocol": 1,
            "daemon_version": env!("CARGO_PKG_VERSION"),
            "pid": 4242,
            "started_at": "2026-10-25T08:00:00Z"
        }
    })
}

/// A plain successful answer to request `id`.
pub fn ok(id: &Value, result: Value) -> Value {
    json!({ "v": 1, "id": id, "result": result, "changed": true })
}

/// A `state` event carrying `snapshot`.
pub fn state_event(seq: u64, snapshot: Value) -> Value {
    json!({ "v": 1, "event": "state", "seq": seq, "data": snapshot })
}

/// A daemon that says hello, answers `state.subscribe` with one `state`
/// event of `snapshot`, records every method it is asked, and answers
/// everything else with an empty success.
pub fn recording(socket: &Path, snapshot: Value) -> (FakeDaemon, Arc<Mutex<Vec<String>>>) {
    let methods = Arc::new(Mutex::new(Vec::new()));
    let seen = methods.clone();
    let daemon = FakeDaemon::start_with(
        socket,
        Arc::new(move |request: &Value| {
            let id = request["id"].clone();
            let name = request["method"].as_str().unwrap_or_default().to_owned();
            seen.lock().unwrap().push(name.clone());
            match name.as_str() {
                "hello" => vec![hello_result(&id)],
                "state.subscribe" => vec![
                    ok(&id, json!({ "subscribed": true, "seq": 1 })),
                    state_event(1, snapshot.clone()),
                ],
                _ => vec![ok(&id, json!({}))],
            }
        }),
    );
    (daemon, methods)
}
