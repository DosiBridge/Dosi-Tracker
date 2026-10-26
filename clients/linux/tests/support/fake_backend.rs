//! Stateful fake Dosi-Tracker backend (HTTP) for end-to-end tests: token,
//! workspaces, projects, activity, heartbeat, summary and device status.
//!
//! It listens on loopback (plain `http://`, which the agent allows only for
//! loopback), answers the endpoints the agent calls the way the ABP backend
//! does, and records every request so tests can check headers and bodies
//! (no `__tenant`, `device_id` on token requests, `X-Workspace-Id` on scoped
//! calls, null counts). Every connection is closed after one answer.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// One request as the fake saw it.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    /// The path with the query string.
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl Recorded {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The form fields of a token request.
    pub fn form(&self) -> BTreeMap<String, String> {
        self.body
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (decode(k), decode(v)))
            .collect()
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How the fake answers `POST /api/app/activity`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityMode {
    Accept,
    /// 503: the agent keeps the row and retries later.
    Unavailable,
    /// 403 `Tracker:DeviceRevoked`.
    DeviceRevoked,
}

/// The fake's state, shared with the test.
#[derive(Debug)]
pub struct BackendState {
    pub users: BTreeMap<String, String>,
    pub workspaces: Vec<Value>,
    /// Projects per workspace id.
    pub projects: BTreeMap<String, Vec<Value>>,
    pub activity_mode: ActivityMode,
    pub device_revoked: bool,
    pub requests: Vec<Recorded>,
    /// Refresh tokens issued and not yet used (they rotate).
    pub refresh_tokens: Vec<String>,
    pub counter: u64,
}

/// A running fake backend; stops when dropped.
pub struct FakeBackend {
    pub url: String,
    pub state: Arc<Mutex<BackendState>>,
    stop: Arc<AtomicBool>,
}

/// A workspace id the tests use.
pub const WS_ACME: &str = "6f1d3a2b-1c4e-4b7a-9a10-0000000000a1";
pub const WS_OTHER: &str = "6f1d3a2b-1c4e-4b7a-9a10-0000000000b2";
pub const WS_ARCHIVED: &str = "6f1d3a2b-1c4e-4b7a-9a10-0000000000c3";
pub const PROJECT_WEBSITE: &str = "7a2e4b3c-2d5f-4c8b-8b21-0000000000d4";
pub const PROJECT_MOBILE: &str = "7a2e4b3c-2d5f-4c8b-8b21-0000000000e5";

/// A project as `my-projects` lists it.
pub fn project(id: &str, title: &str, idle_minutes: u32) -> Value {
    json!({
        "id": id, "title": title, "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true,
        "allowMouse": true, "allowActiveWindow": true, "allowRunningPrograms": true,
        "idleThresholdMinutes": idle_minutes, "blurScreenshots": false
    })
}

pub fn workspace(id: &str, name: &str, archived: bool) -> Value {
    json!({
        "id": id, "name": name, "role": "member", "planName": "Team",
        "isOwner": false, "isArchived": archived, "archivedTime": null
    })
}

impl FakeBackend {
    /// One user (`email`, `password`) in one workspace with two projects.
    pub fn start(email: &str, password: &str) -> Self {
        let mut users = BTreeMap::new();
        users.insert(email.to_lowercase(), password.to_owned());
        let mut projects = BTreeMap::new();
        projects.insert(
            WS_ACME.to_owned(),
            vec![
                project(PROJECT_WEBSITE, "Website", 5),
                project(PROJECT_MOBILE, "Mobile", 5),
            ],
        );
        projects.insert(
            WS_OTHER.to_owned(),
            vec![project(PROJECT_WEBSITE, "Website", 5)],
        );
        let state = Arc::new(Mutex::new(BackendState {
            users,
            workspaces: vec![workspace(WS_ACME, "Acme", false)],
            projects,
            activity_mode: ActivityMode::Accept,
            device_revoked: false,
            requests: Vec::new(),
            refresh_tokens: Vec::new(),
            counter: 0,
        }));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake backend");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_state, thread_stop) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = thread_state.clone();
                        std::thread::spawn(move || serve(stream, state));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { url, state, stop }
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut BackendState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.with(|s| s.requests.clone())
    }

    /// Requests to `path` (without query).
    pub fn requests_to(&self, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.path() == path)
            .collect()
    }

    /// Wait until `pred` holds for the recorded requests (at most `timeout`).
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&[Recorded]) -> bool) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if pred(&self.requests()) {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut data = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    let (head_end, length) = loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            break (end, length);
        }
    };
    while data.len() < head_end + 4 + length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let body_end = (head_end + 4 + length).min(data.len());
    let body = String::from_utf8_lossy(&data[head_end + 4..body_end]).into_owned();
    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: &str, body: &Value) {
    let text = body.to_string();
    let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nDate: {date}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn abp_error(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<BackendState>>) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let mut s = state.lock().unwrap();
    s.requests.push(request.clone());
    let authorized = request
        .header("authorization")
        .is_some_and(|v| v.starts_with("Bearer access-"));
    let workspace = request.header("x-workspace-id").map(str::to_owned);
    let (status, body) = match (request.method.as_str(), request.path()) {
        ("POST", "/connect/token") => {
            let form = request.form();
            let grant = form.get("grant_type").cloned().unwrap_or_default();
            let ok = match grant.as_str() {
                "password" => {
                    let user = form
                        .get("username")
                        .map(|u| u.to_lowercase())
                        .unwrap_or_default();
                    s.users
                        .get(&user)
                        .is_some_and(|pw| Some(pw) == form.get("password"))
                }
                "refresh_token" => {
                    let token = form.get("refresh_token").cloned().unwrap_or_default();
                    let known = s.refresh_tokens.contains(&token);
                    s.refresh_tokens.retain(|t| *t != token);
                    known
                }
                _ => false,
            };
            if ok {
                s.counter += 1;
                let refresh = format!("refresh-{}", s.counter);
                s.refresh_tokens.push(refresh.clone());
                (
                    "200 OK",
                    json!({
                        "access_token": format!("access-{}", s.counter),
                        "token_type": "Bearer", "expires_in": 3600,
                        "refresh_token": refresh,
                    }),
                )
            } else {
                (
                    "400 Bad Request",
                    json!({"error": "invalid_grant", "error_description": "The username or password is incorrect."}),
                )
            }
        }
        (_, _) if !authorized => ("401 Unauthorized", json!({})),
        ("GET", "/api/app/workspaces/mine") => ("200 OK", json!(s.workspaces)),
        ("GET", "/api/app/project/my-projects") => match workspace {
            Some(ws) => (
                "200 OK",
                json!(s
                    .projects
                    .get(&ws.to_lowercase())
                    .cloned()
                    .unwrap_or_default()),
            ),
            None => (
                "400 Bad Request",
                abp_error("Tracker:WorkspaceRequired", "A workspace is required."),
            ),
        },
        ("POST", "/api/app/activity") => match s.activity_mode {
            ActivityMode::Accept => {
                let id = request.json()["clientActivityId"].clone();
                ("200 OK", json!({ "clientActivityId": id }))
            }
            ActivityMode::Unavailable => ("503 Service Unavailable", json!({})),
            ActivityMode::DeviceRevoked => (
                "403 Forbidden",
                abp_error("Tracker:DeviceRevoked", "This device was removed."),
            ),
        },
        ("POST", "/api/app/presence/heartbeat") => (
            "200 OK",
            json!({ "serverTime": chrono::Utc::now().to_rfc3339() }),
        ),
        ("GET", "/api/app/reporting/summary") => (
            "200 OK",
            json!({ "totalTrackedMinutes": 0, "averageProductivity": null }),
        ),
        ("GET", path) if path.starts_with("/api/app/device/") && path.ends_with("/status") => {
            let id = path
                .trim_start_matches("/api/app/device/")
                .trim_end_matches("/status");
            (
                "200 OK",
                json!({ "deviceId": id, "isRevoked": s.device_revoked, "revokedAt": null, "reason": null }),
            )
        }
        _ => ("404 Not Found", json!({})),
    };
    drop(s);
    respond(&mut stream, status, &body);
}
