//! The worker's conversations with the server other than uploads — signing
//! in, the project list, the presence heartbeat and the totals — on a task
//! of their own, so the worker's loop never waits on the network: a pause,
//! a lock or a suspend is handled at once whatever the server is doing.
//!
//! Requests are handled one at a time. A [`Farewell`] (signing out,
//! quitting) jumps the line and cuts short whatever is in flight: the last
//! heartbeat must leave within a second or two.

use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::api::{ApiClient, ApiError, AuthError, ServerClock};
use crate::model::{Heartbeat, Project};
use crate::state::StateHandle;

use super::{local_day_and_week_start, past_days_of_week, Account};

/// Budget of a regular heartbeat, sign-in included.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

/// Budget of the last heartbeat ("stopped", "signedout"): best effort only.
pub const FAREWELL_TIMEOUT: Duration = Duration::from_millis(1_500);

/// How long the earlier days of the week chart are trusted before being
/// fetched again — an offline backlog uploading late can still change them.
const WEEK_HISTORY_TTL: chrono::TimeDelta = chrono::TimeDelta::minutes(30);

#[derive(Debug)]
pub enum ControlRequest {
    /// Check a sign-in the user typed: sign in, then fetch the projects. Only
    /// a sign-in the server accepted replaces the current account.
    SignIn(Account),
    /// Use this account from now on without checking it first (a saved
    /// sign-in), or none.
    UseAccount(Option<Account>),
    RefreshProjects,
    RefreshTotals,
    Heartbeat(Box<Heartbeat>),
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
    SignedIn { account: Account, projects: Result<Vec<Project>, ApiError> },
    SignInFailed(AuthError),
    Projects(Vec<Project>),
    ProjectsFailed(ApiError),
    /// The server no longer accepts the saved password.
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
        if self.urgent.send(Farewell { heartbeat: heartbeat.map(Box::new), done }).is_err() {
            return;
        }
        let _ = tokio::time::timeout(wait, finished).await;
    }

    /// Say goodbye without waiting (signing out: the window does not wait).
    pub fn farewell_later(&self, heartbeat: Option<Heartbeat>) {
        let (done, _) = oneshot::channel();
        let _ = self.urgent.send(Farewell { heartbeat: heartbeat.map(Box::new), done });
    }

    /// A link with no task behind it: the test reads what would be sent.
    #[cfg(test)]
    pub fn detached() -> (Self, UnboundedReceiver<ControlRequest>, UnboundedReceiver<Farewell>) {
        let (requests, requests_rx) = tokio::sync::mpsc::unbounded_channel();
        let (urgent, urgent_rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { requests, urgent }, requests_rx, urgent_rx)
    }
}

/// Start the control task on the current runtime.
pub fn spawn(state: StateHandle, clock: ServerClock, events: UnboundedSender<ControlEvent>) -> ControlLink {
    let (requests, requests_rx) = tokio::sync::mpsc::unbounded_channel();
    let (urgent, urgent_rx) = tokio::sync::mpsc::unbounded_channel();
    let control = Control { client: None, state, clock, events };
    tokio::spawn(control.run(requests_rx, urgent_rx));
    ControlLink { requests, urgent }
}

struct Control {
    client: Option<ApiClient>,
    state: StateHandle,
    clock: ServerClock,
    events: UnboundedSender<ControlEvent>,
}

impl Control {
    async fn run(mut self, mut requests: UnboundedReceiver<ControlRequest>, mut urgent: UnboundedReceiver<Farewell>) {
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

    /// Tell the worker what a failure means: a refused sign-in, or the server
    /// being out of reach.
    fn report(&self, error: &ApiError) {
        if let ApiError::Auth(AuthError::Rejected(reason)) = error {
            self.emit(ControlEvent::SignInExpired(reason.clone()));
        } else if matches!(error, ApiError::Offline(_)) {
            self.emit(ControlEvent::Reachable(false));
        } else {
            // The server answered, if not with what was asked.
            self.emit(ControlEvent::Reachable(true));
        }
    }

    async fn handle(&mut self, request: ControlRequest) {
        match request {
            ControlRequest::SignIn(account) => {
                let mut candidate = account.client(&self.clock);
                match candidate.login().await {
                    Ok(()) => {
                        let projects = candidate.my_projects().await;
                        self.client = Some(candidate);
                        self.emit(ControlEvent::SignedIn { account, projects });
                    }
                    Err(error) => self.emit(ControlEvent::SignInFailed(error)),
                }
            }
            ControlRequest::UseAccount(account) => {
                self.client = account.map(|account| account.client(&self.clock));
            }
            ControlRequest::RefreshProjects => {
                let Some(client) = self.client.as_mut() else { return };
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
                let Some(client) = self.client.as_mut() else { return };
                if let Err(error) = refresh_totals(client, &self.state).await {
                    self.report(&error);
                }
            }
            ControlRequest::Heartbeat(heartbeat) => {
                let Some(client) = self.client.as_mut() else { return };
                match client.heartbeat(&heartbeat, HEARTBEAT_TIMEOUT).await {
                    Ok(_) => self.emit(ControlEvent::Reachable(true)),
                    Err(error) => {
                        // Never retried or queued: the next one is due in a minute.
                        tracing::debug!(%error, "heartbeat failed");
                        self.report(&error);
                    }
                }
            }
        }
    }

    async fn farewell(&mut self, farewell: Farewell) {
        if let (Some(client), Some(heartbeat)) = (self.client.as_mut(), farewell.heartbeat.as_deref()) {
            if let Err(error) = client.heartbeat(heartbeat, FAREWELL_TIMEOUT).await {
                tracing::debug!(%error, "the last heartbeat did not go out");
            }
        }
        self.client = None;
        let _ = farewell.done.send(());
    }
}

/// Refresh the today / this-week totals shown in the window. Failures keep
/// the previous values on screen, but are logged (and returned): a silently
/// failing request once left these cards at zero with no trace of why.
async fn refresh_totals(client: &mut ApiClient, state: &StateHandle) -> Result<(), ApiError> {
    let now = Utc::now();
    let (today_start, week_start) = local_day_and_week_start(now, &Local);
    let zone = crate::tracking::system::time_zone();

    let today = client.summary(today_start, now, zone.as_deref()).await.inspect_err(|e| {
        tracing::warn!(error = %e, "could not load today's totals");
    })?;
    state.update(|s| {
        s.tracked_today_secs = today.total_tracked_minutes.max(0.0) * 60.0;
        s.last_productivity = today.average_productivity.unwrap_or(0.0).clamp(0.0, 100.0) as u8;
    });
    let week = client.summary(week_start, now, zone.as_deref()).await.inspect_err(|e| {
        tracing::warn!(error = %e, "could not load this week's totals");
    })?;
    state.update(|s| s.tracked_week_secs = week.total_tracked_minutes.max(0.0) * 60.0);
    refresh_week_days(client, state, now).await
}

/// Fetch the totals of this week's earlier days (Monday up to yesterday) for
/// the week chart: once per day, then again only when stale, so a refresh
/// usually costs no extra requests.
async fn refresh_week_days(client: &mut ApiClient, state: &StateHandle, now: DateTime<Utc>) -> Result<(), ApiError> {
    let (today_start, _) = local_day_and_week_start(now, &Local);
    let fresh = state
        .snapshot()
        .week_days_fetched
        .is_some_and(|(day, at)| day == today_start && now - at < WEEK_HISTORY_TTL);
    if fresh {
        return Ok(());
    }
    let mut secs = [0.0; 7];
    let zone = crate::tracking::system::time_zone();
    for (slot, from, to) in past_days_of_week(now, &Local) {
        // On failure, what is on screen stays; the next refresh tries again.
        let day = client.summary(from, to, zone.as_deref()).await.inspect_err(|e| {
            tracing::warn!(error = %e, "could not load this week's daily totals");
        })?;
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

    /// Serve `responses` in order, one per connection.
    fn canned_server(responses: Vec<String>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut buf = [0u8; 16 * 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        url
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn account(url: &str) -> Account {
        Account { server: url.into(), workspace: "acme".into(), username: "alice@acme.com".into(), password: "pw".into() }
    }

    async fn next(events: &mut UnboundedReceiver<ControlEvent>) -> ControlEvent {
        tokio::time::timeout(Duration::from_secs(10), events.recv()).await.expect("an event").expect("open")
    }

    #[tokio::test]
    async fn a_sign_in_is_checked_before_it_is_used() {
        let token = r#"{"access_token":"t","expires_in":3600}"#;
        let url = canned_server(vec![http("200 OK", token), http("200 OK", "[]")]);
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let link = spawn(StateHandle::new(), ServerClock::default(), tx);
        link.send(ControlRequest::SignIn(account(&url)));
        match next(&mut events).await {
            ControlEvent::SignedIn { account, projects } => {
                assert_eq!(account.username, "alice@acme.com");
                assert!(projects.unwrap().is_empty());
            }
            other => panic!("expected a sign-in, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_refused_sign_in_is_reported_and_a_refused_refresh_means_expired() {
        let refused = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        let url = canned_server(vec![http("400 Bad Request", refused), http("400 Bad Request", refused)]);
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let link = spawn(StateHandle::new(), ServerClock::default(), tx);

        link.send(ControlRequest::SignIn(account(&url)));
        match next(&mut events).await {
            ControlEvent::SignInFailed(error) => assert!(error.is_rejected()),
            other => panic!("expected a refusal, got {other:?}"),
        }

        // A saved sign-in whose password has since changed.
        link.send(ControlRequest::UseAccount(Some(account(&url))));
        link.send(ControlRequest::RefreshProjects);
        assert!(matches!(next(&mut events).await, ControlEvent::SignInExpired(_)));
        assert!(matches!(next(&mut events).await, ControlEvent::ProjectsFailed(_)));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_reported_as_such() {
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let link = spawn(StateHandle::new(), ServerClock::default(), tx);
        link.send(ControlRequest::UseAccount(Some(account("http://127.0.0.1:9"))));
        link.send(ControlRequest::RefreshProjects);
        assert!(matches!(next(&mut events).await, ControlEvent::Reachable(false)));
        assert!(matches!(next(&mut events).await, ControlEvent::ProjectsFailed(ApiError::Offline(_))));
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
        let (tx, _events) = tokio::sync::mpsc::unbounded_channel();
        let link = spawn(StateHandle::new(), ServerClock::default(), tx);
        link.send(ControlRequest::UseAccount(Some(account(&url))));
        link.send(ControlRequest::RefreshProjects);
        tokio::time::sleep(Duration::from_millis(200)).await;

        let started = std::time::Instant::now();
        link.farewell(None, Duration::from_secs(5)).await;
        assert!(started.elapsed() < Duration::from_secs(2), "not stuck behind the 30 s request");
    }
}
