//! Control task: sign-in, workspaces, projects, heartbeats, totals and
//! device status, off the worker loop.
//!
//! Ported from the Windows `worker/control.rs` over the Linux [`ApiClient`]:
//! the worker's conversations with the server other than uploads run on a
//! task of their own, so the worker's loop never waits on the network — a
//! pause, a lock or a suspend is handled at once whatever the server is doing.
//!
//! Requests are handled one at a time. A [`Farewell`] (signing out,
//! quitting) jumps the line and cuts short whatever is in flight: the last
//! heartbeat must leave within [`FAREWELL_TIMEOUT`].
//!
//! Linux deltas: the device's status (`GET /api/app/device/{id}/status`) is
//! checked on request, and a 403 `Tracker:DeviceRevoked` on any call is
//! reported as [`ControlEvent::DeviceRevoked`]; `WorkspaceArchived` and
//! `WorkspaceRequired` lose the workspace like `NotWorkspaceMember`.

use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use super::model::{Heartbeat, Project, Workspace};
use super::state::StateHandle;
use super::worker::{auth_message, local_day_and_week_start, past_days_of_week, Account};
use crate::api::{ApiClient, ApiError, AuthError, ServerClock, HEARTBEAT_TIMEOUT};

/// Budget of the last heartbeat ("stopped", "signedout"): best effort only.
pub const FAREWELL_TIMEOUT: Duration = Duration::from_millis(1_500);

/// How long the earlier days of the week chart are trusted before being
/// fetched again — an offline backlog uploading late can still change them.
const WEEK_HISTORY_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

#[derive(Debug)]
pub enum ControlRequest {
    /// Check a sign-in the user typed: sign in, then list the user's
    /// workspaces. Only a sign-in the server accepted is used.
    SignIn(Account),
    /// Use this account (and its workspace) from now on without checking it
    /// first, or none.
    UseAccount(Option<Account>),
    /// The signed-in user's workspaces.
    MyWorkspaces,
    RefreshProjects,
    RefreshTotals,
    Heartbeat(Box<Heartbeat>),
    /// Is this device revoked in the current workspace?
    DeviceStatus,
}

/// The last thing said before letting go of the account: a best-effort
/// heartbeat, then the account is dropped.
#[derive(Debug)]
pub struct Farewell {
    pub heartbeat: Option<Box<Heartbeat>>,
    pub done: oneshot::Sender<()>,
}

#[derive(Debug)]
pub enum ControlEvent {
    /// The server accepted the sign-in; `account` holds its token.
    SignedIn {
        account: Account,
        workspaces: Result<Vec<Workspace>, ApiError>,
    },
    SignInFailed(AuthError),
    MyWorkspaces(Result<Vec<Workspace>, String>),
    /// 403 `NotWorkspaceMember` / `WorkspaceArchived` or 400
    /// `WorkspaceRequired` for this workspace.
    WorkspaceLost(String),
    /// 403 `DeviceRevoked`, or the device status says so, for this workspace.
    DeviceRevoked(String),
    Projects(Vec<Project>),
    ProjectsFailed(ApiError),
    /// The server no longer accepts the saved sign-in.
    SignInExpired(String),
    /// Whether the server answered.
    Reachable(bool),
}

/// The worker's side of the control task.
pub struct ControlLink {
    requests: UnboundedSender<ControlRequest>,
    urgent: UnboundedSender<Farewell>,
}

impl ControlLink {
    pub fn send(&self, request: ControlRequest) {
        if self.requests.send(request).is_err() {
            tracing::warn!("the server task has stopped");
        }
    }

    /// Say goodbye (and drop the account), waiting at most `wait` for it.
    pub async fn farewell(&self, heartbeat: Option<Heartbeat>, wait: Duration) {
        let (done, finished) = oneshot::channel();
        if self
            .urgent
            .send(Farewell {
                heartbeat: heartbeat.map(Box::new),
                done,
            })
            .is_err()
        {
            return;
        }
        let _ = tokio::time::timeout(wait, finished).await;
    }

    /// Say goodbye without waiting (signing out: the clients do not wait).
    pub fn farewell_later(&self, heartbeat: Option<Heartbeat>) {
        let (done, _) = oneshot::channel();
        let _ = self.urgent.send(Farewell {
            heartbeat: heartbeat.map(Box::new),
            done,
        });
    }

    /// A link with no task behind it: the test reads what would be sent.
    #[cfg(test)]
    pub fn detached() -> (
        Self,
        UnboundedReceiver<ControlRequest>,
        UnboundedReceiver<Farewell>,
    ) {
        let (requests, requests_rx) = tokio::sync::mpsc::unbounded_channel();
        let (urgent, urgent_rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { requests, urgent }, requests_rx, urgent_rx)
    }
}

/// Start the control task on the current runtime. `time_zone` is the
/// device's IANA zone, so the totals follow its calendar days.
pub fn spawn(
    state: StateHandle,
    clock: ServerClock,
    events: UnboundedSender<ControlEvent>,
    time_zone: Option<String>,
) -> ControlLink {
    let (requests, requests_rx) = tokio::sync::mpsc::unbounded_channel();
    let (urgent, urgent_rx) = tokio::sync::mpsc::unbounded_channel();
    let control = Control {
        client: None,
        state,
        clock,
        events,
        time_zone,
    };
    tokio::spawn(control.run(requests_rx, urgent_rx));
    ControlLink { requests, urgent }
}

struct Control {
    client: Option<ApiClient>,
    state: StateHandle,
    clock: ServerClock,
    events: UnboundedSender<ControlEvent>,
    time_zone: Option<String>,
}

impl Control {
    async fn run(
        mut self,
        mut requests: UnboundedReceiver<ControlRequest>,
        mut urgent: UnboundedReceiver<Farewell>,
    ) {
        loop {
            tokio::select! {
                biased;
                Some(farewell) = urgent.recv() => self.farewell(farewell).await,
                request = requests.recv() => {
                    let Some(request) = request else { return };
                    tokio::select! {
                        biased;
                        // Dropping the request in flight.
                        Some(farewell) = urgent.recv() => self.farewell(farewell).await,
                        _ = self.handle(request) => {}
                    }
                }
            }
        }
    }

    fn emit(&self, event: ControlEvent) {
        let _ = self.events.send(event);
    }

    fn workspace(&self) -> Option<String> {
        self.client
            .as_ref()
            .and_then(ApiClient::workspace_id)
            .map(str::to_owned)
    }

    /// Tell the worker what a failure means: a refused sign-in, a terminal
    /// workspace or device answer, or the server being out of reach.
    fn report(&self, error: &ApiError) {
        match error {
            ApiError::Auth(AuthError::Rejected(reason)) => {
                self.emit(ControlEvent::SignInExpired(reason.clone()))
            }
            ApiError::NotWorkspaceMember(_)
            | ApiError::WorkspaceArchived(_)
            | ApiError::WorkspaceRequired(_) => {
                if let Some(workspace) = self.workspace() {
                    self.emit(ControlEvent::WorkspaceLost(workspace));
                }
            }
            ApiError::DeviceRevoked(_) => {
                if let Some(workspace) = self.workspace() {
                    self.emit(ControlEvent::DeviceRevoked(workspace));
                }
            }
            ApiError::Offline(_) => self.emit(ControlEvent::Reachable(false)),
            // The server answered, if not with what was asked.
            _ => self.emit(ControlEvent::Reachable(true)),
        }
    }

    async fn handle(&mut self, request: ControlRequest) {
        match request {
            ControlRequest::SignIn(account) => {
                let mut candidate = account.client(&self.clock);
                match candidate.login().await {
                    Ok(()) => {
                        let workspaces = candidate.workspaces_mine().await;
                        // The typed password stays with this check only: the
                        // client kept afterwards never signs in again by itself.
                        self.client = Some(account.clone().without_password().client(&self.clock));
                        self.emit(ControlEvent::SignedIn {
                            account,
                            workspaces,
                        });
                    }
                    Err(error) => self.emit(ControlEvent::SignInFailed(error)),
                }
            }
            ControlRequest::UseAccount(account) => {
                self.client = account.map(|account| account.without_password().client(&self.clock));
            }
            ControlRequest::MyWorkspaces => {
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                let found = client.workspaces_mine().await;
                if let Err(error) = &found {
                    self.report(error);
                }
                self.emit(ControlEvent::MyWorkspaces(
                    found.map_err(|e| api_message(&e)),
                ));
            }
            ControlRequest::RefreshProjects => {
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                match client.my_projects().await {
                    Ok(projects) => {
                        self.emit(ControlEvent::Reachable(true));
                        self.emit(ControlEvent::Projects(projects));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not load the projects");
                        self.report(&error);
                        self.emit(ControlEvent::ProjectsFailed(error));
                    }
                }
            }
            ControlRequest::RefreshTotals => {
                let zone = self.time_zone.clone();
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                if let Err(error) = refresh_totals(client, &self.state, zone.as_deref()).await {
                    self.report(&error);
                }
            }
            ControlRequest::Heartbeat(heartbeat) => {
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                match client.heartbeat(&heartbeat, HEARTBEAT_TIMEOUT).await {
                    Ok(_) => self.emit(ControlEvent::Reachable(true)),
                    Err(error) => {
                        // Never retried or queued: the next one is due in a minute.
                        tracing::debug!(%error, "heartbeat failed");
                        self.report(&error);
                    }
                }
            }
            ControlRequest::DeviceStatus => {
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                let Some(device_id) = client.device_id().map(str::to_owned) else {
                    return;
                };
                if client.workspace_id().is_none() {
                    return;
                }
                match client.device_status(&device_id).await {
                    Ok(status) if status.is_revoked => {
                        tracing::warn!("the device status says this device was revoked");
                        if let Some(workspace) = self.workspace() {
                            self.emit(ControlEvent::DeviceRevoked(workspace));
                        }
                    }
                    Ok(_) => self.emit(ControlEvent::Reachable(true)),
                    Err(error) => {
                        tracing::debug!(%error, "could not check the device's status");
                        self.report(&error);
                    }
                }
            }
        }
    }

    async fn farewell(&mut self, farewell: Farewell) {
        if let (Some(client), Some(heartbeat)) =
            (self.client.as_mut(), farewell.heartbeat.as_deref())
        {
            if let Err(error) = client.heartbeat(heartbeat, FAREWELL_TIMEOUT).await {
                tracing::debug!(%error, "the last heartbeat did not go out");
            }
        }
        self.client = None;
        let _ = farewell.done.send(());
    }
}

fn api_message(error: &ApiError) -> String {
    match error {
        ApiError::Auth(e) => auth_message(e),
        ApiError::Offline(_) => auth_message(&AuthError::Offline(String::new())),
        ApiError::Http { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Refresh the today / this-week totals shown in the clients. Failures keep
/// the previous values on screen, but are logged (and returned): a silently
/// failing request once left these cards at zero with no trace of why.
async fn refresh_totals(
    client: &mut ApiClient,
    state: &StateHandle,
    zone: Option<&str>,
) -> Result<(), ApiError> {
    let now = Utc::now();
    let (today_start, week_start) = local_day_and_week_start(now, &Local);

    let today = client
        .summary(today_start, now, zone)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, "could not load today's totals"))?;
    state.update(|s| {
        s.tracked_today_secs = today.total_tracked_minutes.max(0.0) * 60.0;
        s.last_productivity = today
            .average_productivity
            .map(|p| p.clamp(0.0, 100.0).round() as u8);
    });
    let week = client
        .summary(week_start, now, zone)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, "could not load this week's totals"))?;
    state.update(|s| s.tracked_week_secs = week.total_tracked_minutes.max(0.0) * 60.0);
    refresh_week_days(client, state, now, zone).await
}

/// Fetch the totals of this week's earlier days (Monday up to yesterday) for
/// the week chart: once per day, then again only when stale, so a refresh
/// usually costs no extra requests.
async fn refresh_week_days(
    client: &mut ApiClient,
    state: &StateHandle,
    now: DateTime<Utc>,
    zone: Option<&str>,
) -> Result<(), ApiError> {
    let (today_start, _) = local_day_and_week_start(now, &Local);
    let fresh = state
        .snapshot()
        .week_days_fetched
        .is_some_and(|(day, at)| day == today_start && now - at < WEEK_HISTORY_TTL);
    if fresh {
        return Ok(());
    }
    let mut secs = [0.0; 7];
    for (slot, from, to) in past_days_of_week(now, &Local) {
        // On failure, what is on screen stays; the next refresh tries again.
        let day = client.summary(from, to, zone).await.inspect_err(
            |e| tracing::warn!(error = %e, "could not load this week's daily totals"),
        )?;
        secs[slot] = day.total_tracked_minutes.max(0.0) * 60.0;
    }
    state.update(|s| {
        s.week_day_secs = secs;
        s.week_days_fetched = Some((today_start, now));
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const WS: &str = "6d1f3c2a-9b8e-4f70-a1b2-c3d4e5f60718";
    const DEVICE: &str = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11";

    /// Read one request (headers and a Content-Length body).
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    /// Serve `responses` in order, one per connection; pass each request on.
    fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = tx.send(read_request(&mut stream));
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (url, rx)
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn token() -> String {
        http(
            "200 OK",
            r#"{"access_token":"t","expires_in":3600,"refresh_token":"r1"}"#,
        )
    }

    fn account(url: &str) -> Account {
        let mut account =
            Account::new(url.into(), "alice@acme.com".into(), "pw").in_workspace(WS, "Acme");
        account.device_id = Some(DEVICE.into());
        account
    }

    fn link() -> (ControlLink, UnboundedReceiver<ControlEvent>) {
        let (tx, events) = tokio::sync::mpsc::unbounded_channel();
        (
            spawn(
                StateHandle::new(),
                ServerClock::default(),
                tx,
                Some("UTC".into()),
            ),
            events,
        )
    }

    async fn next(events: &mut UnboundedReceiver<ControlEvent>) -> ControlEvent {
        tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("an event")
            .expect("open")
    }

    #[tokio::test]
    async fn a_sign_in_is_checked_before_it_is_used() {
        let (url, requests) = canned_server(vec![
            token(),
            http("200 OK", &format!(r#"[{{"id":"{WS}","name":"Acme"}}]"#)),
        ]);
        let (link, mut events) = link();
        link.send(ControlRequest::SignIn(account(&url)));
        match next(&mut events).await {
            ControlEvent::SignedIn {
                account,
                workspaces,
            } => {
                assert_eq!(account.username, "alice@acme.com");
                assert_eq!(workspaces.unwrap()[0].name, "Acme");
            }
            other => panic!("expected a sign-in, got {other:?}"),
        }
        let requests: Vec<String> = requests.try_iter().collect();
        assert!(requests[0].starts_with("POST /connect/token"));
        assert!(
            requests[0].contains(&format!("device_id={DEVICE}")),
            "{}",
            requests[0]
        );
        assert!(!requests[0].contains(concat!("__", "tenant")));
        assert!(requests[1].starts_with("GET /api/app/workspaces/mine"));
        assert!(
            !requests[1].to_ascii_lowercase().contains("x-workspace-id"),
            "workspaces/mine is not workspace-scoped"
        );
    }

    #[tokio::test]
    async fn a_refused_sign_in_is_reported_and_a_refused_refresh_means_expired() {
        let refused =
            r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        let (url, requests) = canned_server(vec![
            http("400 Bad Request", refused),
            http("400 Bad Request", refused),
        ]);
        let (link, mut events) = link();

        link.send(ControlRequest::SignIn(account(&url)));
        match next(&mut events).await {
            ControlEvent::SignInFailed(error) => assert!(error.is_rejected()),
            other => panic!("expected a refusal, got {other:?}"),
        }

        // A saved sign-in whose refresh token has since been revoked: the
        // refused refresh is "sign-in expired", never a silent password login.
        let mut saved = account(&url);
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("old".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::RefreshProjects);
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::SignInExpired(_)
        ));
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::ProjectsFailed(_)
        ));
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains("grant_type=refresh_token") && !requests[1].contains("password"),
            "no password grant after the refused refresh: {}",
            requests[1]
        );
    }

    #[tokio::test]
    async fn an_unreachable_server_is_reported_as_such() {
        let (link, mut events) = link();
        let mut saved = account("http://127.0.0.1:9");
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::RefreshProjects);
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::Reachable(false)
        ));
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::ProjectsFailed(ApiError::Offline(_))
        ));
    }

    #[tokio::test]
    async fn a_farewell_cuts_a_slow_request_short() {
        // Accepts connections, never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
            }
        });
        let (link, _events) = link();
        let mut slow = account(&url);
        slow.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(slow)));
        link.send(ControlRequest::RefreshProjects);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let started = std::time::Instant::now();
        link.farewell(None, Duration::from_secs(5)).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "not stuck behind the 30 s request"
        );
    }

    #[tokio::test]
    async fn losing_the_workspace_is_reported() {
        let lost = r#"{"error":{"code":"Tracker:NotWorkspaceMember","message":"No access"}}"#;
        let (url, _requests) = canned_server(vec![token(), http("403 Forbidden", lost)]);
        let (link, mut events) = link();
        let mut saved = account(&url);
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::RefreshProjects);
        match next(&mut events).await {
            ControlEvent::WorkspaceLost(workspace) => assert_eq!(workspace, WS),
            other => panic!("expected the workspace lost, got {other:?}"),
        }
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::ProjectsFailed(ApiError::NotWorkspaceMember(_))
        ));
    }

    #[tokio::test]
    async fn an_archived_workspace_is_lost_too() {
        let archived = r#"{"error":{"code":"Tracker:WorkspaceArchived","message":"Archived"}}"#;
        let (url, _requests) = canned_server(vec![token(), http("403 Forbidden", archived)]);
        let (link, mut events) = link();
        let mut saved = account(&url);
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::MyWorkspaces);
        assert!(matches!(next(&mut events).await, ControlEvent::WorkspaceLost(w) if w == WS));
        assert!(matches!(
            next(&mut events).await,
            ControlEvent::MyWorkspaces(Err(_))
        ));
    }

    #[tokio::test]
    async fn a_revoked_device_is_reported_from_its_status_and_from_a_403() {
        let (url, requests) = canned_server(vec![
            token(),
            http(
                "200 OK",
                &format!(r#"{{"deviceId":"{DEVICE}","isRevoked":true}}"#),
            ),
            http(
                "403 Forbidden",
                r#"{"error":{"code":"Tracker:DeviceRevoked","message":"Revoked"}}"#,
            ),
        ]);
        let (link, mut events) = link();
        let mut saved = account(&url);
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::DeviceStatus);
        assert!(matches!(next(&mut events).await, ControlEvent::DeviceRevoked(w) if w == WS));
        let heartbeat = Heartbeat {
            device_id: DEVICE.into(),
            device_name: None,
            platform: Some("linux".into()),
            os_version: None,
            agent_version: None,
            status: crate::agent::model::PresenceStatus::Tracking,
            project_id: None,
            status_since: None,
            current_app: None,
            time_zone: None,
            utc_offset_minutes: None,
            sent_at: None,
        };
        link.send(ControlRequest::Heartbeat(Box::new(heartbeat)));
        assert!(matches!(next(&mut events).await, ControlEvent::DeviceRevoked(w) if w == WS));
        let requests: Vec<String> = requests.try_iter().collect();
        assert!(requests[1].starts_with(&format!("GET /api/app/device/{DEVICE}/status")));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains(&format!("x-workspace-id: {WS}")));
    }

    #[tokio::test]
    async fn the_totals_use_the_device_time_zone() {
        let summary = r#"{"totalTrackedMinutes":90,"averageProductivity":64.4}"#;
        let mut responses = vec![token(), http("200 OK", summary), http("200 OK", summary)];
        // At most six earlier days of this week.
        responses.extend((0..6).map(|_| http("200 OK", summary)));
        let (url, requests) = canned_server(responses);
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let state = StateHandle::new();
        let link = spawn(
            state.clone(),
            ServerClock::default(),
            tx,
            Some("Asia/Dhaka".into()),
        );
        let mut saved = account(&url);
        saved.tokens = crate::api::TokenCell::with_refresh_token(Some("r".into()));
        link.send(ControlRequest::UseAccount(Some(saved)));
        link.send(ControlRequest::RefreshTotals);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while state.snapshot().tracked_week_secs == 0.0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snapshot = state.snapshot();
        assert_eq!(snapshot.tracked_today_secs, 90.0 * 60.0);
        assert_eq!(snapshot.last_productivity, Some(64));
        let first = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(first.starts_with("POST /connect/token"));
        let summary_request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            summary_request.contains("TimeZone=Asia%2FDhaka"),
            "{summary_request}"
        );
        drop(link);
        let _ = events.try_recv();
    }
}
