//! Blocking IPC client used by the CLI and the GUI: lstat-checked socket,
//! `hello` version negotiation, requests and state subscriptions.
//!
//! - [`Client::connect`] checks the socket with `lstat` (a socket of our own
//!   uid, never a symlink) before connecting, then says `hello`. A daemon of
//!   another protocol major is reported as [`ClientError::Version`].
//! - [`Client::call`] sends one request and waits for its answer (events that
//!   arrive meanwhile are kept for [`Subscription`]).
//! - [`connect_or_start`] starts the daemon when it is not running:
//!   `systemctl --user start dosi-tracker.service`, else a detached
//!   `dosi-tracker daemon --detach`, waiting at most [`START_WAIT`] for the
//!   socket. `--no-start` and `DOSI_NO_AUTOSTART=1` turn that off.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zeroize::Zeroize;

use super::{
    decode, encode, method, ErrorBody, ErrorCode, Event, FrameDecoder, HelloParams, HelloResult,
    Request, ServerFrame, PROTOCOL_VERSION,
};

/// How long a plain request may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(40);

/// How long `connect_or_start` waits for a starting daemon's socket.
pub const START_WAIT: Duration = Duration::from_secs(5);

/// Why talking to the daemon failed.
#[derive(Debug)]
pub enum ClientError {
    /// No daemon is listening (no socket, or nobody behind it).
    NotRunning(String),
    /// The socket is not ours: never connected to.
    Unsafe(String),
    /// The daemon speaks another protocol major.
    Version(ErrorBody),
    /// The daemon answered with an error.
    Daemon(ErrorBody),
    /// No answer in time.
    Timeout,
    /// The connection broke or the answer was not understood.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotRunning(why) => {
                write!(f, "the Dosi Tracker agent is not running ({why})")
            }
            ClientError::Unsafe(why) => write!(f, "refusing the agent's socket: {why}"),
            ClientError::Version(error) | ClientError::Daemon(error) => f.write_str(&error.message),
            ClientError::Timeout => f.write_str("the agent did not answer in time"),
            ClientError::Protocol(why) => write!(f, "talking to the agent failed: {why}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A request's successful answer.
#[derive(Debug, Clone, PartialEq)]
pub struct CallResult {
    pub result: Value,
    pub changed: Option<bool>,
}

/// One connection to the daemon.
pub struct Client {
    stream: UnixStream,
    decoder: FrameDecoder,
    next_id: u64,
    /// Decoded frames not read yet.
    frames: VecDeque<ServerFrame>,
    /// Events that arrived while waiting for an answer.
    events: VecDeque<Event>,
    hello: HelloResult,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("hello", &self.hello)
            .finish()
    }
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Is `socket` a socket of ours? `Ok(false)` when it does not exist.
pub fn socket_present(socket: &Path) -> Result<bool, ClientError> {
    match std::fs::symlink_metadata(socket) {
        Ok(meta) => {
            if !meta.file_type().is_socket() {
                return Err(ClientError::Unsafe(format!(
                    "{} is not a socket",
                    socket.display()
                )));
            }
            if meta.uid() != euid() {
                return Err(ClientError::Unsafe(format!(
                    "{} belongs to another user",
                    socket.display()
                )));
            }
            Ok(true)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ClientError::NotRunning(e.to_string())),
    }
}

impl Client {
    /// Connect to `socket` and say `hello` as `client` (`cli` or `gui`).
    pub fn connect(socket: &Path, client: &str) -> Result<Self, ClientError> {
        if !socket_present(socket)? {
            return Err(ClientError::NotRunning("no socket".into()));
        }
        let stream = UnixStream::connect(socket).map_err(|e| match e.kind() {
            io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound => {
                ClientError::NotRunning(e.to_string())
            }
            _ => ClientError::Protocol(e.to_string()),
        })?;
        let mut this = Self {
            stream,
            decoder: FrameDecoder::new(),
            next_id: 0,
            frames: VecDeque::new(),
            events: VecDeque::new(),
            hello: HelloResult {
                protocol: 0,
                daemon_version: String::new(),
                pid: 0,
                started_at: chrono::Utc::now(),
            },
        };
        let params = serde_json::to_value(HelloParams {
            client: client.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        })
        .unwrap_or(Value::Null);
        let answer = this
            .call_with_timeout(method::HELLO, params, Duration::from_secs(5))
            .map_err(|e| match e {
                ClientError::Daemon(error) if error.code == ErrorCode::UnsupportedVersion => {
                    ClientError::Version(error)
                }
                ClientError::Daemon(error) if error.code == ErrorCode::Busy => {
                    ClientError::Daemon(error)
                }
                other => other,
            })?;
        this.hello = serde_json::from_value(answer.result)
            .map_err(|e| ClientError::Protocol(format!("bad hello answer: {e}")))?;
        if this.hello.protocol != PROTOCOL_VERSION {
            return Err(ClientError::Version(
                ErrorBody::new(
                    ErrorCode::UnsupportedVersion,
                    format!(
                        "The running agent speaks IPC protocol {}, this command {PROTOCOL_VERSION}.",
                        this.hello.protocol
                    ),
                )
                .with_hint("restart with dosi-tracker service restart"),
            ));
        }
        Ok(this)
    }

    /// The daemon's `hello` answer.
    pub fn hello(&self) -> &HelloResult {
        &self.hello
    }

    /// Send `method` and wait at most [`CALL_TIMEOUT`] for the answer.
    pub fn call(&mut self, method: &str, params: Value) -> Result<CallResult, ClientError> {
        self.call_with_timeout(method, params, CALL_TIMEOUT)
    }

    /// Send `method` and wait at most `timeout` for the answer.
    pub fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<CallResult, ClientError> {
        self.next_id += 1;
        let id = self.next_id;
        let request = Request {
            v: PROTOCOL_VERSION,
            id,
            method: method.into(),
            params,
        };
        let mut frame = encode(&request).map_err(|e| ClientError::Protocol(e.to_string()))?;
        drop(request);
        let written = self.stream.write_all(&frame);
        frame.zeroize();
        written.map_err(|e| ClientError::Protocol(e.to_string()))?;
        let deadline = Instant::now() + timeout;
        loop {
            match self.next_frame(deadline)? {
                ServerFrame::Event(event) => self.events.push_back(event),
                ServerFrame::Response(response) if response.id == id || response.id == 0 => {
                    if let Some(error) = response.error {
                        return Err(ClientError::Daemon(error));
                    }
                    return Ok(CallResult {
                        result: response.result.unwrap_or(Value::Null),
                        changed: response.changed,
                    });
                }
                ServerFrame::Response(_) => {}
            }
        }
    }

    /// Subscribe to state events (the first one carries the current snapshot).
    pub fn subscribe(mut self) -> Result<Subscription, ClientError> {
        self.call(method::STATE_SUBSCRIBE, json!({}))?;
        Ok(Subscription { client: self })
    }

    fn next_frame(&mut self, deadline: Instant) -> Result<ServerFrame, ClientError> {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            // Frames already buffered come first.
            if let Some(frame) = self.pending_frame()? {
                return Ok(frame);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Timeout);
            }
            self.stream
                .set_read_timeout(Some(left))
                .map_err(|e| ClientError::Protocol(e.to_string()))?;
            let n = match self.stream.read(&mut buffer) {
                Ok(0) => {
                    return Err(ClientError::Protocol(
                        "the agent closed the connection".into(),
                    ))
                }
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(ClientError::Timeout)
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(ClientError::Protocol(e.to_string())),
            };
            let frames = self
                .decoder
                .push(&buffer[..n])
                .map_err(|e| ClientError::Protocol(e.to_string()));
            buffer[..n].zeroize();
            for frame in frames? {
                self.frames.push_back(frame_value(&frame)?);
            }
        }
    }

    fn pending_frame(&mut self) -> Result<Option<ServerFrame>, ClientError> {
        Ok(self.frames.pop_front())
    }
}

fn frame_value(frame: &[u8]) -> Result<ServerFrame, ClientError> {
    decode(frame).map_err(|e| ClientError::Protocol(e.to_string()))
}

/// A subscribed connection: [`Subscription::next_event`] blocks for the next
/// event; requests can still be sent on it.
pub struct Subscription {
    client: Client,
}

impl Subscription {
    /// The next event, waiting at most `timeout` (`None`: forever).
    pub fn next_event(&mut self, timeout: Option<Duration>) -> Result<Event, ClientError> {
        if let Some(event) = self.client.events.pop_front() {
            return Ok(event);
        }
        let deadline = Instant::now() + timeout.unwrap_or(Duration::from_secs(365 * 86_400));
        loop {
            match self.client.next_frame(deadline)? {
                ServerFrame::Event(event) => return Ok(event),
                ServerFrame::Response(_) => {}
            }
        }
    }

    pub fn client(&mut self) -> &mut Client {
        &mut self.client
    }
}

impl Iterator for Subscription {
    type Item = Result<Event, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_event(None) {
            Err(ClientError::Protocol(_)) => None,
            other => Some(other),
        }
    }
}

// ---- Starting the daemon -------------------------------------------------------------------

/// How [`connect_or_start`] may start the daemon.
#[derive(Debug, Clone)]
pub struct StartOptions {
    /// Never start it (`--no-start`, `DOSI_NO_AUTOSTART=1`).
    pub no_start: bool,
    /// The `dosi-tracker` binary for the detached fallback.
    pub exe: PathBuf,
    /// `cli` or `gui`.
    pub client: &'static str,
    /// The environment the daemon is started with.
    pub env: Vec<(String, String)>,
}

impl StartOptions {
    /// `DOSI_NO_AUTOSTART=1` (or any value but `0`/empty) means never start.
    pub fn no_autostart_env(env: &[(String, String)]) -> bool {
        env.iter()
            .any(|(k, v)| k == "DOSI_NO_AUTOSTART" && !v.trim().is_empty() && v.trim() != "0")
    }
}

/// How the daemon was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning,
    Systemd,
    Detached,
}

/// Connect, starting the daemon first when it is not running and that is allowed.
pub fn connect_or_start(
    socket: &Path,
    options: &StartOptions,
) -> Result<(Client, Started), ClientError> {
    match Client::connect(socket, options.client) {
        Ok(client) => return Ok((client, Started::AlreadyRunning)),
        Err(ClientError::NotRunning(why)) if options.no_start => {
            return Err(ClientError::NotRunning(why))
        }
        Err(ClientError::NotRunning(_)) => {}
        Err(other) => return Err(other),
    }
    let systemctl = crate::service::Systemctl::new(crate::service::ServiceEnv::from_vars(
        options.env.iter().cloned(),
    ));
    let budget = Instant::now() + START_WAIT;
    let via_systemd = systemctl.user_manager_available(Some(budget))
        && systemctl
            .unit_state(Some(budget))
            .is_ok_and(|state| state.loaded())
        && systemctl.unit("start", true, Some(budget)).is_ok();
    if via_systemd {
        if let Some(client) = wait_for_socket(socket, options.client, Instant::now() + START_WAIT) {
            return Ok((client, Started::Systemd));
        }
    }
    spawn_detached(&options.exe, &options.env)
        .map_err(|e| ClientError::NotRunning(format!("could not start it: {e}")))?;
    wait_for_socket(socket, options.client, Instant::now() + START_WAIT)
        .map(|client| (client, Started::Detached))
        .ok_or_else(|| ClientError::NotRunning("it did not start within 5 seconds".into()))
}

fn wait_for_socket(socket: &Path, client: &'static str, deadline: Instant) -> Option<Client> {
    loop {
        match Client::connect(socket, client) {
            Ok(client) => return Some(client),
            Err(ClientError::NotRunning(_)) | Err(ClientError::Protocol(_)) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `exe daemon --detach` in its own process group with `env`, stdio closed.
pub fn spawn_detached(exe: &Path, env: &[(String, String)]) -> io::Result<u32> {
    use std::os::unix::process::CommandExt;
    let child = std::process::Command::new(exe)
        .args(["daemon", "--detach"])
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(child.id())
}
