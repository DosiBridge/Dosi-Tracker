//! The `dosi-ipc` thread: Unix socket listener (0600 in a 0700 dir),
//! SO_PEERCRED same-uid check, requests to worker commands, subscriptions.
//!
//! - [`DaemonLock`] is the single-instance lock (`daemon.lock`, `flock`). The
//!   daemon holds it for life; the CLI holds it for offline writes so no
//!   daemon can start in the middle of one.
//! - [`bind`] creates the runtime directory privately, removes a stale socket
//!   (only with the lock held, so no live daemon is listening on it), binds,
//!   sets mode 0600 and checks the result with `lstat`: our socket, mode
//!   0600, in our 0700 directory. Anything else is refused.
//! - [`spawn`] runs the listener on a tokio current-thread runtime. A
//!   connection whose peer uid is not ours is closed before a byte is read.
//!   At most [`super::MAX_CONNECTIONS`] are served; the first request must be
//!   `hello`; a frame must complete within [`super::FRAME_TIMEOUT`]; a client
//!   that does not subscribe is dropped after [`super::IDLE_TIMEOUT`].
//! - What the methods do is the [`Dispatch`] implementation's business (the
//!   daemon's maps them to worker commands). `hello`, `state.subscribe` and
//!   `ui.show` are answered here: subscriptions get coalesced `state` events
//!   (at most one per [`super::EVENT_MIN_INTERVAL`]) and GUI subscribers get
//!   the `show` events.
//! - Request parameters are never logged; frame buffers are wiped.

use std::future::Future;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use zeroize::Zeroize;

use super::{
    decode, encode, method, ErrorBody, ErrorCode, Event, FrameDecoder, FrameError, HelloParams,
    HelloResult, Request, Response, EVENT_MIN_INTERVAL, EVENT_SHOW, EVENT_STATE, FRAME_TIMEOUT,
    IDLE_TIMEOUT, MAX_CONNECTIONS, PROTOCOL_VERSION,
};

/// A boxed future the server can run on its runtime.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A method's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub result: Value,
    /// For commands: whether anything changed.
    pub changed: Option<bool>,
}

impl Reply {
    /// A plain answer (queries).
    pub fn value(result: Value) -> Self {
        Self {
            result,
            changed: None,
        }
    }

    /// A command's answer.
    pub fn changed(result: Value, changed: bool) -> Self {
        Self {
            result,
            changed: Some(changed),
        }
    }
}

/// What the methods do.
pub trait Dispatch: Send + Sync + 'static {
    /// The daemon's identity for `hello`.
    fn hello(&self) -> HelloResult;
    /// Run `method` (never `hello`, `state.subscribe` or `ui.show`).
    fn call(&self, method: &str, params: Value) -> BoxFuture<Result<Reply, ErrorBody>>;
    /// The state's change counter.
    fn changes(&self) -> watch::Receiver<u64>;
    /// The snapshot as the `state` event carries it.
    fn snapshot(&self) -> Value;
}

// ---- Single-instance lock ------------------------------------------------------------------

/// `daemon.lock`, held with `flock(LOCK_EX)` until dropped.
#[derive(Debug)]
pub struct DaemonLock {
    file: std::fs::File,
    path: PathBuf,
}

impl DaemonLock {
    /// Take the lock at `path` (its directory is created privately). `None`
    /// when another process holds it.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        if let Some(dir) = path.parent() {
            crate::paths::ensure_private_dir(dir)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions and cannot fail.
        if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refusing {}: not a private file of ours", path.display()),
            ));
        }
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is open for the lifetime of `file`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(Some(Self {
                file,
                path: path.to_path_buf(),
            }));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Ok(None)
        } else {
            Err(error)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Keep the lock across `exec` (the detached daemon re-executes itself
    /// after an upgrade and must not lose its single-instance lock).
    pub fn inherit_across_exec(&self) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let fd = self.file.as_raw_fd();
        // SAFETY: plain fcntl calls on a descriptor we own.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

// ---- Binding -------------------------------------------------------------------------------

/// Why the socket could not be set up. The daemon exits 9 on [`BindError::Unsafe`].
#[derive(Debug)]
pub enum BindError {
    /// The directory or the socket is not ours or not private.
    Unsafe(String),
    Io(io::Error),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::Unsafe(why) => write!(f, "unsafe IPC socket: {why}"),
            BindError::Io(e) => write!(f, "cannot listen on the IPC socket: {e}"),
        }
    }
}

impl std::error::Error for BindError {}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Check that `dir` is our 0700 directory.
pub fn check_private_dir(dir: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    if meta.uid() != euid() {
        return Err(format!("{} is owned by another user", dir.display()));
    }
    if meta.mode() & 0o777 != 0o700 {
        return Err(format!(
            "{} has mode {:o}, not 0700",
            dir.display(),
            meta.mode() & 0o777
        ));
    }
    Ok(())
}

/// Check that `socket` is our 0600 socket (not a symlink).
pub fn check_socket(socket: &Path) -> Result<(), String> {
    let meta =
        std::fs::symlink_metadata(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    if !meta.file_type().is_socket() {
        return Err(format!("{} is not a socket", socket.display()));
    }
    if meta.uid() != euid() {
        return Err(format!("{} is owned by another user", socket.display()));
    }
    if meta.mode() & 0o777 != 0o600 {
        return Err(format!(
            "{} has mode {:o}, not 0600",
            socket.display(),
            meta.mode() & 0o777
        ));
    }
    Ok(())
}

/// Bind `socket` (design 10). `_lock` proves no other daemon owns the path.
pub fn bind(
    socket: &Path,
    _lock: &DaemonLock,
) -> Result<std::os::unix::net::UnixListener, BindError> {
    let dir = socket
        .parent()
        .ok_or_else(|| BindError::Unsafe("the socket path has no directory".into()))?;
    crate::paths::ensure_private_dir(dir).map_err(|e| BindError::Unsafe(e.to_string()))?;
    check_private_dir(dir).map_err(BindError::Unsafe)?;

    match std::fs::symlink_metadata(socket) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == euid() => {
            // Stale: we hold the lock, so nobody is listening on it.
            std::fs::remove_file(socket).map_err(BindError::Io)?;
            tracing::info!("removed a stale IPC socket");
        }
        Ok(_) => {
            return Err(BindError::Unsafe(format!(
                "{} exists and is not our socket",
                socket.display()
            )))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(BindError::Io(e)),
    }

    // The mode is set right after binding rather than with a temporary umask:
    // the umask is process-wide and other threads create files meanwhile.
    // Nobody else can reach the socket in between (the directory is 0700).
    let listener = std::os::unix::net::UnixListener::bind(socket).map_err(BindError::Io)?;
    if let Err(e) = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(socket);
        return Err(BindError::Io(e));
    }
    if let Err(why) = check_socket(socket) {
        let _ = std::fs::remove_file(socket);
        return Err(BindError::Unsafe(why));
    }
    Ok(listener)
}

// ---- Serving -------------------------------------------------------------------------------

/// Reads the peer's uid (and pid) of a connection.
pub type PeerCredentials = Arc<dyn Fn(&UnixStream) -> io::Result<(u32, Option<i32>)> + Send + Sync>;

/// How the server behaves; the defaults are protocol v1's.
#[derive(Clone)]
pub struct ServerConfig {
    /// Only this uid is served (the daemon's euid).
    pub expected_uid: u32,
    /// `SO_PEERCRED` by default; tests inject other credentials.
    pub peer_credentials: PeerCredentials,
    pub max_connections: usize,
    pub idle_timeout: Duration,
    pub frame_timeout: Duration,
    pub event_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            expected_uid: euid(),
            peer_credentials: Arc::new(|stream: &UnixStream| {
                let cred = stream.peer_cred()?;
                Ok((cred.uid(), cred.pid()))
            }),
            max_connections: MAX_CONNECTIONS,
            idle_timeout: IDLE_TIMEOUT,
            frame_timeout: FRAME_TIMEOUT,
            event_interval: EVENT_MIN_INTERVAL,
        }
    }
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("expected_uid", &self.expected_uid)
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// The running `dosi-ipc` thread.
pub struct ServerHandle {
    stop: watch::Sender<bool>,
    show: broadcast::Sender<Value>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Send a `show` event to every subscribed GUI; returns how many got it.
    pub fn show(&self, view: Option<&str>) -> usize {
        self.show.send(json!({ "view": view })).unwrap_or(0)
    }

    /// Stop accepting and close every connection.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Shared {
    config: ServerConfig,
    dispatch: Arc<dyn Dispatch>,
    show: broadcast::Sender<Value>,
    active: AtomicUsize,
    stop: watch::Receiver<bool>,
}

/// Counts a connection while it lives.
struct Slot(Arc<Shared>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serve `listener` on the `dosi-ipc` thread.
pub fn spawn(
    listener: std::os::unix::net::UnixListener,
    config: ServerConfig,
    dispatch: Arc<dyn Dispatch>,
) -> io::Result<ServerHandle> {
    listener.set_nonblocking(true)?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let (show, _) = broadcast::channel(16);
    let shared = Arc::new(Shared {
        config,
        dispatch,
        show: show.clone(),
        active: AtomicUsize::new(0),
        stop: stop_rx,
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let thread = std::thread::Builder::new()
        .name("dosi-ipc".into())
        .spawn(move || {
            runtime.block_on(accept_loop(listener, shared));
            // Connections still open are dropped with the runtime.
        })?;
    Ok(ServerHandle {
        stop: stop_tx,
        show,
        thread: Some(thread),
    })
}

async fn accept_loop(listener: std::os::unix::net::UnixListener, shared: Arc<Shared>) {
    let listener = match tokio::net::UnixListener::from_std(listener) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(error = %e, "the IPC listener cannot be used");
            return;
        }
    };
    let mut stop = shared.stop.clone();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let shared = shared.clone();
                    tokio::spawn(async move { connection(stream, shared).await });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "accepting an IPC connection failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            _ = stop.changed() => break,
        }
    }
}

/// One frame to write, or the end of the connection.
type Outgoing = Vec<u8>;

fn frame_of(response: &Response) -> Outgoing {
    encode(response).unwrap_or_else(|_| {
        encode(&Response::err(
            response.id,
            ErrorBody::new(ErrorCode::Internal, "the answer is too large"),
        ))
        .unwrap_or_default()
    })
}

async fn connection(stream: UnixStream, shared: Arc<Shared>) {
    // Same uid only, decided before anything is read from the peer.
    match (shared.config.peer_credentials)(&stream) {
        Ok((uid, _)) if uid == shared.config.expected_uid => {}
        Ok((uid, pid)) => {
            tracing::warn!(uid, ?pid, "IPC connection from another user refused");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %e, "IPC peer credentials unavailable; connection refused");
            return;
        }
    }
    let count = shared.active.fetch_add(1, Ordering::SeqCst) + 1;
    let _slot = Slot(shared.clone());
    let (mut reader, mut writer) = stream.into_split();
    if count > shared.config.max_connections {
        let frame = frame_of(&Response::err(
            0,
            ErrorBody::new(ErrorCode::Busy, "Too many clients are connected.")
                .with_hint("close another Dosi Tracker window or command and try again"),
        ));
        let _ = writer.write_all(&frame).await;
        return;
    }

    let (out_tx, mut out_rx) = mpsc::channel::<Outgoing>(64);
    let writer_task = tokio::spawn(async move {
        while let Some(mut frame) = out_rx.recv().await {
            let written = writer.write_all(&frame).await;
            frame.zeroize();
            if written.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });

    let mut session = Session {
        shared: shared.clone(),
        out: out_tx,
        hello: None,
        subscription: None,
        close: false,
    };
    let mut decoder = FrameDecoder::new();
    let mut buffer = vec![0u8; 16 * 1024];
    let mut stop = shared.stop.clone();
    let mut last_request = Instant::now();
    let mut frame_started: Option<Instant> = None;
    loop {
        let deadline = match frame_started {
            Some(started) => started + shared.config.frame_timeout,
            None if session.subscription.is_some() => Instant::now() + Duration::from_secs(86_400),
            None => last_request + shared.config.idle_timeout,
        };
        let read = tokio::select! {
            read = reader.read(&mut buffer) => read,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::debug!("IPC connection timed out");
                break;
            }
            _ = stop.changed() => break,
        };
        let n = match read {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let pushed = decoder.push(&buffer[..n]);
        buffer[..n].zeroize();
        let frames = match pushed {
            Ok(frames) => frames,
            Err(FrameError::TooLarge) => {
                session
                    .send(Response::err(
                        0,
                        ErrorBody::new(ErrorCode::BadRequest, "The request is too large."),
                    ))
                    .await;
                break;
            }
            Err(FrameError::Malformed(_)) => break,
        };
        frame_started = if decoder.has_partial() {
            frame_started.or(Some(Instant::now()))
        } else {
            None
        };
        if !frames.is_empty() {
            last_request = Instant::now();
        }
        for frame in frames {
            session.handle(&frame).await;
            if session.close {
                break;
            }
        }
        if session.close {
            break;
        }
    }
    if let Some(task) = session.subscription.take() {
        task.abort();
    }
    drop(session);
    let _ = writer_task.await;
}

/// One connection's state.
struct Session {
    shared: Arc<Shared>,
    out: mpsc::Sender<Outgoing>,
    /// The client's `hello`, once received.
    hello: Option<HelloParams>,
    subscription: Option<tokio::task::JoinHandle<()>>,
    close: bool,
}

impl Session {
    async fn send(&self, response: Response) {
        let _ = self.out.send(frame_of(&response)).await;
    }

    async fn handle(&mut self, frame: &[u8]) {
        let value: Value = match decode(frame) {
            Ok(value) => value,
            Err(_) => {
                return self
                    .send(Response::err(
                        0,
                        ErrorBody::new(ErrorCode::BadRequest, "The request is not valid JSON."),
                    ))
                    .await;
            }
        };
        let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
        match value.get("v").and_then(Value::as_u64) {
            Some(v) if v == u64::from(PROTOCOL_VERSION) => {}
            _ => {
                self.close = true;
                return self
                    .send(Response::err(
                        id,
                        ErrorBody::new(
                            ErrorCode::UnsupportedVersion,
                            format!("This agent speaks IPC protocol {PROTOCOL_VERSION}."),
                        )
                        .with_hint("restart with dosi-tracker service restart"),
                    ))
                    .await;
            }
        }
        let request: Request = match serde_json::from_value(value) {
            Ok(request) => request,
            Err(_) => {
                return self
                    .send(Response::err(
                        id,
                        ErrorBody::new(ErrorCode::BadRequest, "The request is malformed."),
                    ))
                    .await;
            }
        };
        if !method::ALL.contains(&request.method.as_str()) {
            return self
                .send(Response::err(
                    request.id,
                    ErrorBody::new(
                        ErrorCode::UnknownMethod,
                        "This agent does not know that request.",
                    ),
                ))
                .await;
        }
        if self.hello.is_none() && request.method != method::HELLO {
            return self
                .send(Response::err(
                    request.id,
                    ErrorBody::new(ErrorCode::BadRequest, "Send hello first."),
                ))
                .await;
        }
        tracing::debug!(method = %request.method, "IPC request");
        match request.method.as_str() {
            method::HELLO => self.hello(request).await,
            method::STATE_SUBSCRIBE => self.subscribe(request.id).await,
            method::UI_SHOW => {
                let view = request
                    .params
                    .get("view")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let forwarded = self.shared.show.send(json!({ "view": view })).unwrap_or(0);
                self.send(Response::ok(
                    request.id,
                    json!({ "forwarded": forwarded > 0 }),
                    None,
                ))
                .await;
            }
            _ => {
                let Request {
                    id, method, params, ..
                } = request;
                let call = self.shared.dispatch.call(&method, params);
                let out = self.out.clone();
                tokio::spawn(async move {
                    let response = match call.await {
                        Ok(reply) => Response::ok(id, reply.result, reply.changed),
                        Err(error) => Response::err(id, error),
                    };
                    let _ = out.send(frame_of(&response)).await;
                });
            }
        }
    }

    async fn hello(&mut self, request: Request) {
        let params: HelloParams = serde_json::from_value(request.params).unwrap_or(HelloParams {
            client: "unknown".into(),
            version: String::new(),
        });
        self.hello = Some(params);
        let result = self.shared.dispatch.hello();
        self.send(Response::ok(
            request.id,
            serde_json::to_value(result).unwrap_or(Value::Null),
            None,
        ))
        .await;
    }

    async fn subscribe(&mut self, id: u64) {
        let changes = self.shared.dispatch.changes();
        let seq = *changes.borrow();
        self.send(Response::ok(
            id,
            json!({ "subscribed": true, "seq": seq }),
            None,
        ))
        .await;
        if self.subscription.is_some() {
            return;
        }
        let gui = self.hello.as_ref().is_some_and(|h| h.client == "gui");
        let shows = gui.then(|| self.shared.show.subscribe());
        let task = tokio::spawn(subscription(
            changes,
            self.shared.dispatch.clone(),
            self.out.clone(),
            self.shared.config.event_interval,
            shows,
        ));
        self.subscription = Some(task);
    }
}

fn event_frame(event: &str, seq: u64, data: Value) -> Option<Outgoing> {
    encode(&Event {
        v: PROTOCOL_VERSION,
        event: event.into(),
        seq,
        data,
    })
    .ok()
}

/// Push `state` events (the first at once, then coalesced to at most one per
/// `interval`) and, for a GUI, `show` events.
async fn subscription(
    mut changes: watch::Receiver<u64>,
    dispatch: Arc<dyn Dispatch>,
    out: mpsc::Sender<Outgoing>,
    interval: Duration,
    mut shows: Option<broadcast::Receiver<Value>>,
) {
    let seq = *changes.borrow_and_update();
    if let Some(frame) = event_frame(EVENT_STATE, seq, dispatch.snapshot()) {
        if out.send(frame).await.is_err() {
            return;
        }
    }
    let mut last = Instant::now();
    let mut show_seq = 0u64;
    loop {
        let show = async {
            match shows.as_mut() {
                Some(rx) => rx.recv().await.ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                // Coalesce: wait out the interval, then send the latest.
                tokio::time::sleep_until(last + interval).await;
                let seq = *changes.borrow_and_update();
                let Some(frame) = event_frame(EVENT_STATE, seq, dispatch.snapshot()) else {
                    continue;
                };
                if out.send(frame).await.is_err() {
                    return;
                }
                last = Instant::now();
            }
            Some(data) = show => {
                show_seq += 1;
                if let Some(frame) = event_frame(EVENT_SHOW, show_seq, data) {
                    if out.send(frame).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}
