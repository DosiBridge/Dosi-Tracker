use std::io::{Read, Write};
use std::time::Instant;

use super::clock::{parse_retry_after, skew_warning_minutes, MAX_RETRY_AFTER};
use super::tokens::GrantedToken;
use super::*;

#[test]
fn http_client_builds_with_native_roots_and_timeouts() {
    // Loads the Windows certificate store; must never panic on a normal machine.
    let _client = build_http_client();
}

#[test]
fn owner_key_matches_the_queue_tagging() {
    let client = ApiClient::new("https://example.invalid/", "Alice@Acme.com".into(), "secret".into());
    assert_eq!(client.owner_key(), crate::storage::owner_key("https://example.invalid", "alice@acme.com"));
    let elsewhere = ApiClient::new("https://other.invalid", "Alice@Acme.com".into(), "secret".into());
    assert_ne!(elsewhere.owner_key(), client.owner_key(), "the same email on another server is another account");
}

#[test]
fn the_base_url_never_ends_with_a_slash() {
    let client = ApiClient::new(" https://tracker.example.com// ", "a".into(), "b".into());
    assert_eq!(client.base_url(), "https://tracker.example.com");
}

#[test]
fn summary_query_survives_url_encoding() {
    let from = DateTime::parse_from_rfc3339("2026-09-29T18:00:00+00:00").unwrap().with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339("2026-09-30T10:29:24.976+00:00").unwrap().with_timezone(&Utc);
    let request = reqwest::Client::new()
        .get("https://example.invalid/api/app/reporting/summary")
        .query(&summary_query(from, to, Some("America/Argentina/Buenos_Aires")))
        .build()
        .unwrap();

    // What the server sees after decoding must still parse to the same instants.
    let pairs: std::collections::HashMap<_, _> = request.url().query_pairs().into_owned().collect();
    let parse = |key: &str| DateTime::parse_from_rfc3339(&pairs[key]).unwrap().with_timezone(&Utc);
    assert_eq!(parse("From"), from);
    assert_eq!(parse("To"), to);
    assert_eq!(pairs["TimeZone"], "America/Argentina/Buenos_Aires");

    // No zone: the parameter is left out (the server's default applies).
    assert_eq!(summary_query(from, to, None).len(), 2);
    assert_eq!(summary_query(from, to, Some("  ")).len(), 2);
}

#[test]
fn report_summaries_tolerate_unmeasured_productivity() {
    let summary: ReportSummary =
        serde_json::from_str(r#"{"totalTrackedMinutes":90.5,"averageProductivity":null,"totalIdleMinutes":12}"#).unwrap();
    assert_eq!(summary.total_tracked_minutes, 90.5);
    assert_eq!(summary.average_productivity, None);
    // An older server always measured.
    let older: ReportSummary = serde_json::from_str(r#"{"totalTrackedMinutes":5,"averageProductivity":61.2}"#).unwrap();
    assert_eq!(older.average_productivity, Some(61.2));
}

fn abp(code: Option<&str>, message: &str) -> String {
    let code = code.map(|c| format!("\"{c}\"")).unwrap_or_else(|| "null".to_string());
    format!(r#"{{"error":{{"code":{code},"message":"{message}","details":null}}}}"#)
}

#[test]
fn upload_failures_are_classified_as_the_contract_says() {
    let wait = Some(Duration::from_secs(30));
    let kind = |status: u16, body: &str| match classify_upload_failure(status, body, None) {
        SubmitError::Transient { .. } => "transient",
        SubmitError::AuthRejected(_) => "auth",
        SubmitError::SubscriptionInactive(_) => "subscription",
        SubmitError::ProjectUnavailable { .. } => "project",
        SubmitError::ClockAhead(_) => "clock",
        SubmitError::Workspace { refusal: WorkspaceRefusal::NotMember, .. } => "workspace",
        SubmitError::Workspace { refusal: WorkspaceRefusal::Archived, .. } => "archived",
        SubmitError::Workspace { refusal: WorkspaceRefusal::Required, .. } => "required",
        SubmitError::DeviceRevoked(_) => "revoked",
        SubmitError::RowRetry(_) => "row",
        SubmitError::StripCaptures(_) => "strip",
        SubmitError::Rejected(_) => "park",
    };

    // Transient: infrastructure, throttling and code-less 4xx.
    for status in [404, 405, 408, 429, 500, 502, 503] {
        assert_eq!(kind(status, ""), "transient", "{status}");
    }
    assert_eq!(kind(400, "<html>bad request</html>"), "transient");
    assert_eq!(kind(403, r#"{"title":"Forbidden"}"#), "transient");
    // A redirect is never followed, and is not this block's fault.
    assert_eq!(kind(307, ""), "transient");
    assert_eq!(kind(308, "<html>moved</html>"), "transient");
    // A business code this agent does not know: that row alone waits.
    assert_eq!(kind(422, &abp(Some("Tracker:SomethingNew"), "?")), "row");
    // A 5xx is transient even with a business code.
    assert_eq!(kind(500, &abp(Some(codes::ACTIVITY_OVERLAPS), "x")), "transient");
    assert_eq!(kind(503, &abp(Some(codes::DEVICE_REVOKED), "x")), "transient");

    // Terminal: the device, or the workspace (never parked).
    assert_eq!(kind(403, &abp(Some(codes::DEVICE_REVOKED), "Removed")), "revoked");
    assert_eq!(kind(403, &abp(Some(codes::NOT_WORKSPACE_MEMBER), "No access")), "workspace");
    assert_eq!(kind(403, &abp(Some(codes::WORKSPACE_ARCHIVED), "Archived")), "archived");
    assert_eq!(kind(400, &abp(Some(codes::WORKSPACE_REQUIRED), "Which workspace?")), "required");
    let required_with_validation = r#"{"error":{"code":"Tracker:WorkspaceRequired","message":"Which workspace?",
            "validationErrors":[{"message":"X-Workspace-Id is required.","members":["workspace"]}]}}"#;
    assert_eq!(kind(400, required_with_validation), "required", "a setup problem, never a park");

    // Special handling.
    assert_eq!(kind(402, &abp(Some(codes::SUBSCRIPTION_INACTIVE), "Inactive")), "subscription");
    assert_eq!(kind(402, ""), "transient", "402 without a code");
    assert_eq!(kind(403, &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member")), "project");
    assert_eq!(kind(409, &abp(Some(codes::PROJECT_ARCHIVED), "Archived")), "project");
    assert_eq!(kind(425, &abp(Some(codes::ACTIVITY_IN_FUTURE), "In the future")), "clock");
    assert_eq!(kind(413, ""), "strip");
    assert_eq!(kind(413, "<html>Request Entity Too Large</html>"), "strip");
    assert_eq!(kind(413, &abp(Some(codes::CAPTURE_TOO_LARGE), "Too big")), "strip");
    assert_eq!(kind(400, &abp(Some(codes::INVALID_CAPTURE_DATA), "Bad image")), "strip");

    // Park: definitive rejections of the block.
    assert_eq!(kind(409, &abp(Some(codes::ACTIVITY_OVERLAPS), "Overlaps")), "park");
    assert_eq!(kind(409, &abp(Some(codes::ACTIVITY_CLIENT_ID_CONFLICT), "Conflict")), "park");
    assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")), "park");
    assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TOO_OLD), "Too old")), "park");
    assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TIME_RANGE_INVALID), "Bad range")), "park");
    let validation = r#"{"error":{"code":null,"message":"Your request is not valid!",
            "validationErrors":[{"message":"Description is too long.","members":["description"]}]}}"#;
    assert_eq!(kind(400, validation), "park");
    // An older server answers 403 for every business error: the code decides.
    assert_eq!(kind(403, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")), "park");
    // Validation errors park the block even if a code comes with them.
    let coded_validation = r#"{"error":{"code":"Volo.Abp.Validation","message":"Your request is not valid!",
            "validationErrors":[{"message":"The field TimeZone is invalid.","members":["timeZone"]}]}}"#;
    assert_eq!(kind(400, coded_validation), "park");
    // ...but only on a 400: elsewhere the code is just one this agent does
    // not know, and that row alone is tried again later.
    assert_eq!(kind(422, coded_validation), "row");

    // The server's message is kept for the window; Retry-After is passed on.
    match classify_upload_failure(400, validation, None) {
        SubmitError::Rejected(message) => assert_eq!(message, "Your request is not valid! Description is too long."),
        other => panic!("expected a rejection, got {other:?}"),
    }
    match classify_upload_failure(429, "", wait) {
        SubmitError::Transient { retry_after, .. } => assert_eq!(retry_after, wait),
        other => panic!("expected transient, got {other:?}"),
    }
    match classify_upload_failure(403, &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member"), None) {
        SubmitError::ProjectUnavailable { code, message } => {
            assert_eq!(code, codes::NOT_PROJECT_MEMBER);
            assert_eq!(message, "Not a member");
        }
        other => panic!("expected project unavailable, got {other:?}"),
    }
}

#[test]
fn only_refused_credentials_are_a_rejected_sign_in() {
    let invalid_grant = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
    assert_eq!(
        classify_token_failure(400, invalid_grant),
        AuthError::Rejected("Invalid username or password!".into())
    );
    let locked = r#"{"error":"invalid_grant","error_description":"The user account has been locked out."}"#;
    assert_eq!(classify_token_failure(400, locked).user_message(), "The user account has been locked out.");
    assert!(classify_token_failure(401, "").is_rejected());
    assert!(classify_token_failure(403, "").is_rejected());
    assert_eq!(classify_token_failure(401, ""), AuthError::Rejected("Incorrect email or password.".into()));

    // Not about the credentials: a proxy's bare 400, a wrong client, a
    // missing endpoint, throttling and server failures.
    assert!(!classify_token_failure(400, "").is_rejected());
    assert!(!classify_token_failure(400, r#"{"error":"invalid_client"}"#).is_rejected());
    assert!(!classify_token_failure(404, "").is_rejected());
    assert!(matches!(classify_token_failure(429, ""), AuthError::Server(m) if m.contains("Too many")));
    assert!(matches!(classify_token_failure(503, ""), AuthError::Server(m) if m.contains("503")));
    assert!(AuthError::Offline("tcp connect error".into()).user_message().contains("Cannot reach the server"));
}

#[test]
fn retry_after_takes_seconds_or_a_date_and_is_bounded() {
    let now = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
    assert_eq!(parse_retry_after("120", None, now), Some(Duration::from_secs(120)));
    assert_eq!(parse_retry_after(" 0 ", None, now), Some(Duration::ZERO));
    assert_eq!(
        parse_retry_after("Mon, 05 Oct 2026 10:05:00 GMT", None, now),
        Some(Duration::from_secs(300))
    );
    // Measured against the server's clock when it is known.
    let server_now = now + chrono::Duration::minutes(2);
    assert_eq!(
        parse_retry_after("Mon, 05 Oct 2026 10:05:00 GMT", Some(server_now), now),
        Some(Duration::from_secs(180))
    );
    // In the past: retry now. Absurdly far: capped.
    assert_eq!(parse_retry_after("Mon, 05 Oct 2026 09:00:00 GMT", None, now), Some(Duration::ZERO));
    assert_eq!(parse_retry_after("99999999", None, now), Some(MAX_RETRY_AFTER));
    assert_eq!(parse_retry_after("soon", None, now), None);
}

#[test]
fn the_server_clock_offset_comes_from_the_date_header() {
    let device = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
    assert_eq!(clock_offset_from_date("Mon, 05 Oct 2026 10:03:30 GMT", device), Some(210));
    assert_eq!(clock_offset_from_date("Mon, 05 Oct 2026 09:59:00 GMT", device), Some(-60));
    assert_eq!(clock_offset_from_date("yesterday", device), None);

    let clock = ServerClock::default();
    assert_eq!(clock.offset_secs(), None);
    assert_eq!(clock.skew_minutes(), None);
    clock.record(-60);
    assert_eq!(clock.clone().offset_secs(), Some(-60), "clones share the value");
    assert_eq!(clock.skew_minutes(), None, "within two minutes: no warning");
    clock.record(-7 * 60 - 20);
    assert_eq!(clock.skew_minutes(), Some(-7), "device clock 7 min ahead");
    assert_eq!(skew_warning_minutes(121), Some(2));
    assert_eq!(skew_warning_minutes(120), None);
}

// --- Against a local canned server ------------------------------------

/// One raw HTTP/1.1 response.
fn http(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    )
}

fn token_ok() -> String {
    http("200 OK", &[], r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600}"#)
}

/// Read one request (headers and a Content-Length body) off the stream.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
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
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if data.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&data).into_owned()
}

/// Serve `responses` in order, one per connection, and pass each request
/// on as text.
fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let _ = tx.send(read_request(&mut stream));
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    (url, rx)
}

fn client(url: &str) -> ApiClient {
    ApiClient::new(format!("{url}/"), "alice@acme.com".into(), "pw".into()).with_workspace(Some("w1".into()))
}

fn sample_activity() -> Activity {
    let now = Utc::now();
    Activity::new("0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11", now - chrono::Duration::minutes(10), now)
}

#[tokio::test]
async fn a_401_upload_renews_the_token_and_resends_with_sent_at() {
    let granted = http("200 OK", &[], r#"{"access_token":"t0k3n","refresh_token":"r1","expires_in":3600}"#);
    let (url, requests) = canned_server(vec![
        granted,
        http("401 Unauthorized", &[], ""),
        token_ok(),
        http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], "{}"),
    ]);
    let mut api = client(&url);
    api.submit_activity(&sample_activity()).await.expect("uploaded on the second try");

    let requests: Vec<String> = requests.try_iter().collect();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("POST /connect/token "), "{}", requests[0]);
    assert!(requests[1].starts_with("POST /api/app/activity "), "no double slash: {}", requests[1]);
    assert!(requests[2].starts_with("POST /connect/token"));
    assert!(body_of(&requests[2]).contains("grant_type=refresh_token"), "renewed, never the password again");
    let upload = &requests[3];
    assert!(upload.contains("authorization: Bearer t0k3n") || upload.contains("Authorization: Bearer t0k3n"));
    let body = &upload[upload.find("\r\n\r\n").unwrap() + 4..];
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    let sent_at = json["sentAt"].as_str().expect("sentAt is stamped at send time");
    assert!(DateTime::parse_from_rfc3339(sent_at).is_ok());
    assert!(json.get("productivity").is_none(), "unmeasured: left out");
    // The Date header of the last answer set the clock offset.
    assert!(api.server_clock().offset_secs().is_some());
}

#[tokio::test]
async fn a_refused_sign_in_on_any_call_means_sign_in_expired() {
    // The cached token expired server-side (401) and the refresh token
    // is refused.
    let granted = http("200 OK", &[], r#"{"access_token":"t0k3n","refresh_token":"r1","expires_in":3600}"#);
    let (url, _requests) = canned_server(vec![
        granted,
        http("401 Unauthorized", &[], ""),
        http("400 Bad Request", &[], r#"{"error":"invalid_grant","error_description":"The token is no longer valid."}"#),
    ]);
    let error = client(&url).my_projects().await.unwrap_err();
    assert!(error.is_auth_rejected(), "{error}");
    assert_eq!(error.status(), None);

    // Unreachable is not a refusal.
    let mut offline = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
    assert!(!offline.my_projects().await.unwrap_err().is_auth_rejected());
}

#[tokio::test]
async fn a_shared_server_clock_sees_every_response() {
    let (url, _requests) = canned_server(vec![
        http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], r#"{"access_token":"t","expires_in":3600}"#),
        http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], "[]"),
    ]);
    // What the window reads, handed to a client created later (a new
    // sign-in): it is updated by that client's answers.
    let window_clock = ServerClock::default();
    let mut api = client(&url).with_server_clock(window_clock.clone());
    assert_eq!(window_clock.offset_secs(), None);
    assert!(api.my_projects().await.unwrap().is_empty());
    let fixed = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
    let expected = (fixed - Utc::now()).num_seconds();
    let seen = window_clock.offset_secs().expect("recorded from the Date header");
    assert!((seen - expected).abs() <= 5, "{seen} vs {expected}");
    assert_eq!(api.server_clock().offset_secs(), Some(seen), "one shared value");
}

#[tokio::test]
async fn refused_credentials_during_an_upload_mean_sign_in_expired() {
    let (url, _requests) = canned_server(vec![http(
        "400 Bad Request",
        &[],
        r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#,
    )]);
    match client(&url).submit_activity(&sample_activity()).await {
        Err(SubmitError::AuthRejected(reason)) => assert_eq!(reason, "Invalid username or password!"),
        other => panic!("expected sign-in expired, got {other:?}"),
    }
}

#[tokio::test]
async fn an_oversized_upload_asks_for_its_captures_to_be_stripped() {
    let (url, _requests) = canned_server(vec![token_ok(), http("413 Payload Too Large", &[], "")]);
    assert!(matches!(
        client(&url).submit_activity(&sample_activity()).await,
        Err(SubmitError::StripCaptures(_))
    ));
}

#[tokio::test]
async fn throttled_uploads_carry_the_retry_after() {
    let (url, _requests) = canned_server(vec![token_ok(), http("429 Too Many Requests", &[("Retry-After", "90")], "")]);
    match client(&url).submit_activity(&sample_activity()).await {
        Err(SubmitError::Transient { retry_after, .. }) => assert_eq!(retry_after, Some(Duration::from_secs(90))),
        other => panic!("expected transient, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unreachable_server_is_transient_and_offline() {
    // Nothing listens on port 9 (discard) locally: a real connect error.
    let mut api = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
    assert!(matches!(api.login().await, Err(AuthError::Offline(_))));
    assert!(matches!(api.submit_activity(&sample_activity()).await, Err(SubmitError::Transient { .. })));
    assert!(matches!(api.my_projects().await, Err(ApiError::Offline(_))));
}

#[tokio::test]
async fn projects_parse_with_the_new_policy_fields_and_errors_keep_the_abp_body() {
    let projects = r#"[{"id":"p1","title":"Website","intervalMinutes":10,"allowScreenshot":true,
            "allowWebcam":false,"allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,
            "allowRunningPrograms":true,"idleThresholdMinutes":15,"blurScreenshots":true}]"#;
    let (url, requests) = canned_server(vec![
        token_ok(),
        http("200 OK", &[], projects),
        http("403 Forbidden", &[], &abp(Some("Volo.Authorization:010001"), "Authorization failed!")),
    ]);
    let mut api = client(&url);
    let list = api.my_projects().await.unwrap();
    assert_eq!(list[0].idle_threshold_minutes, 15);
    assert!(list[0].blur_screenshots);

    let error = api
        .summary(Utc::now() - chrono::Duration::hours(1), Utc::now(), Some("Asia/Dhaka"))
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(403));
    assert!(error.to_string().contains("Authorization failed!"));
    let summary_request = requests.try_iter().nth(2).unwrap();
    assert!(summary_request.contains("TimeZone=Asia%2FDhaka"), "{summary_request}");
}

fn heartbeat() -> Heartbeat {
    Heartbeat {
        device_id: uuid::Uuid::new_v4().to_string(),
        device_name: None,
        platform: Some("windows".into()),
        os_version: None,
        agent_version: None,
        status: crate::model::PresenceStatus::Stopped,
        project_id: None,
        status_since: None,
        current_app: None,
        time_zone: None,
        utc_offset_minutes: None,
        sent_at: None,
    }
}

#[tokio::test]
async fn heartbeats_are_bounded_by_their_timeout() {
    // Accepts connections but never answers.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    let heartbeat = Heartbeat {
        device_id: uuid::Uuid::new_v4().to_string(),
        device_name: None,
        platform: Some("windows".into()),
        os_version: None,
        agent_version: None,
        status: crate::model::PresenceStatus::Stopped,
        project_id: None,
        status_since: None,
        current_app: None,
        time_zone: None,
        utc_offset_minutes: None,
        sent_at: None,
    };
    let started = Instant::now();
    let result = client(&url).heartbeat(&heartbeat, Duration::from_millis(800)).await;
    assert!(matches!(result, Err(ApiError::Offline(_))), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn heartbeats_post_the_contract_body_and_read_the_server_time() {
    let (url, requests) = canned_server(vec![
        token_ok(),
        http("200 OK", &[], r#"{"serverTime":"2026-10-05T10:00:00Z"}"#),
    ]);
    let heartbeat = Heartbeat {
        device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
        device_name: Some("DESKTOP-1".into()),
        platform: Some("windows".into()),
        os_version: None,
        agent_version: Some("0.2.0".into()),
        status: crate::model::PresenceStatus::Tracking,
        project_id: Some("p1".into()),
        status_since: None,
        current_app: Some("code.exe".into()),
        time_zone: Some("Asia/Dhaka".into()),
        utc_offset_minutes: Some(360),
        sent_at: None,
    };
    let result = client(&url).heartbeat(&heartbeat, Duration::from_secs(5)).await.unwrap();
    assert_eq!(result.server_time, crate::model::parse_server_time("2026-10-05T10:00:00Z"));
    let request = requests.try_iter().nth(1).unwrap();
    assert!(request.starts_with("POST /api/app/presence/heartbeat "), "{request}");
    let body: serde_json::Value = serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
    assert!(body["sentAt"].is_string());
}

// --- Workspaces, the header and token renewal -------------------------

fn body_of(request: &str) -> &str {
    &request[request.find("\r\n\r\n").unwrap() + 4..]
}

fn header_of<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let head = &request[..request.find("\r\n\r\n").unwrap_or(request.len())];
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[tokio::test]
async fn the_password_sign_in_sends_no_tenant_and_asks_for_a_refresh_token() {
    let (url, requests) = canned_server(vec![token_ok()]);
    client(&url).login().await.unwrap();
    let request = requests.try_iter().next().unwrap();
    assert!(request.starts_with("POST /connect/token "), "{request}");
    assert!(!request.to_lowercase().contains("__tenant"), "{request}");
    let form = body_of(&request);
    for part in ["grant_type=password", "client_id=Tracker_App", "username=alice%40acme.com", "password=pw", "offline_access"] {
        assert!(form.contains(part), "{part} in {form}");
    }
    assert_eq!(header_of(&request, WORKSPACE_HEADER), None);
}

#[tokio::test]
async fn workspace_calls_carry_the_header_and_token_and_mine_calls_do_not() {
    let mine = r#"[{"id":"w1","name":"Acme","role":"Owner","planName":"Pro","isOwner":true},{"id":"w2","name":"Globex","role":"Member"}]"#;
    let (url, requests) = canned_server(vec![
        token_ok(),
        http("200 OK", &[], mine),
        http("200 OK", &[], "[]"),
        http("200 OK", &[], r#"{"totalTrackedMinutes":1}"#),
        http("200 OK", &[], "{}"),
        http("200 OK", &[], "{}"),
    ]);
    let mut api = client(&url);
    let workspaces = api.workspaces_mine().await.unwrap();
    assert_eq!(workspaces.len(), 2);
    assert_eq!((workspaces[0].id.as_str(), workspaces[0].role.as_deref(), workspaces[0].is_owner), ("w1", Some("Owner"), true));
    assert_eq!(workspaces[1].plan_name, None);

    api.set_workspace(Some("w2".into()));
    api.my_projects().await.unwrap();
    api.summary(Utc::now() - chrono::Duration::hours(1), Utc::now(), None).await.unwrap();
    api.heartbeat(&heartbeat(), Duration::from_secs(5)).await.unwrap();
    api.submit_activity(&sample_activity()).await.unwrap();

    let requests: Vec<String> = requests.try_iter().collect();
    assert_eq!(header_of(&requests[0], WORKSPACE_HEADER), None, "the token request");
    assert!(requests[1].starts_with("GET /api/app/workspaces/mine "), "{}", requests[1]);
    assert_eq!(header_of(&requests[1], WORKSPACE_HEADER), None, "listing the workspaces");
    for request in &requests[2..] {
        assert_eq!(header_of(request, WORKSPACE_HEADER), Some("w2"), "{request}");
        assert_eq!(header_of(request, "authorization"), Some("Bearer t0k3n"));
    }
}

#[tokio::test]
async fn an_expired_token_is_renewed_with_the_refresh_token_not_the_password() {
    let refreshed = http("200 OK", &[], r#"{"access_token":"n3w","refresh_token":"r2","expires_in":3600}"#);
    let (url, requests) = canned_server(vec![http("401 Unauthorized", &[], ""), refreshed, http("200 OK", &[], "[]")]);
    let tokens = TokenCell::with_refresh_token(Some("r1".into()));
    tokens.set(GrantedToken { access_token: "old".into(), refresh_token: Some("r1".into()), expires_at: Instant::now() + Duration::from_secs(600) });
    let mut api = client(&url).with_tokens(tokens.clone()).with_workspace(Some("w1".into()));
    api.my_projects().await.unwrap();

    let requests: Vec<String> = requests.try_iter().collect();
    assert_eq!(header_of(&requests[0], "authorization"), Some("Bearer old"));
    let form = body_of(&requests[1]);
    assert!(form.contains("grant_type=refresh_token") && form.contains("refresh_token=r1"), "{form}");
    assert!(!form.contains("password"), "{form}");
    assert_eq!(header_of(&requests[2], "authorization"), Some("Bearer n3w"));
    assert_eq!(tokens.refresh_token().as_deref(), Some("r2"), "the rotated refresh token is kept");

    // A saved refresh token is used before the first request.
    let (url, requests) = canned_server(vec![token_ok(), http("200 OK", &[], "[]")]);
    let mut api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("saved".into())));
    api.workspaces_mine().await.unwrap();
    assert!(body_of(&requests.try_iter().next().unwrap()).contains("refresh_token=saved"));
}

#[tokio::test]
async fn a_refused_refresh_token_means_sign_in_expired_never_a_silent_password_sign_in() {
    let refused = http("400 Bad Request", &[], r#"{"error":"invalid_grant","error_description":"The token is no longer valid."}"#);
    let (url, requests) = canned_server(vec![refused.clone(), token_ok(), http("200 OK", &[], "[]")]);
    // Even with a password at hand.
    let mut api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    let error = api.workspaces_mine().await.unwrap_err();
    assert!(error.is_auth_rejected(), "{error}");
    assert!(error.to_string().contains("The token is no longer valid."), "{error}");
    let requests: Vec<String> = requests.try_iter().collect();
    assert_eq!(requests.len(), 1, "only the refresh: {requests:?}");
    assert!(!body_of(&requests[0]).contains("grant_type=password"));
    assert!(!body_of(&requests[0]).contains("password=pw"));

    // Without a password at hand it is the same.
    let (url, _requests) = canned_server(vec![refused]);
    let mut api = ApiClient::new(url, "alice@acme.com".into(), String::new()).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    assert!(api.workspaces_mine().await.unwrap_err().is_auth_rejected());
}

#[tokio::test]
async fn the_password_is_sent_once_even_when_the_server_grants_no_refresh_token() {
    // A token without a refresh token, that then expires (401).
    let (url, requests) = canned_server(vec![token_ok(), http("401 Unauthorized", &[], ""), token_ok()]);
    let tokens = TokenCell::default();
    let mut api = client(&url).with_tokens(tokens.clone());
    api.login().await.unwrap();
    assert!(api.password.is_empty(), "forgotten once accepted");
    // Another client of the same sign-in that still holds the password.
    let mut other = client(&url).with_tokens(tokens);
    let error = other.my_projects().await.unwrap_err();
    assert!(error.is_auth_rejected(), "sign-in expired, not a silent password sign-in: {error}");
    let requests: Vec<String> = requests.try_iter().collect();
    assert_eq!(requests.len(), 2, "the sign-in and the refused request only: {requests:?}");
    assert!(body_of(&requests[0]).contains("grant_type=password"));
}

#[tokio::test]
async fn signing_out_revokes_the_refresh_token() {
    // No discovery document: ABP's default endpoint.
    let (url, requests) = canned_server(vec![http("404 Not Found", &[], ""), http("200 OK", &[], "")]);
    let tokens = TokenCell::with_refresh_token(Some("r1".into()));
    let api = client(&url).with_tokens(tokens.clone()).with_device_id(Some("d1".into()));
    assert_eq!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Revoked);
    let requests: Vec<String> = requests.try_iter().collect();
    assert!(requests[0].starts_with("GET /.well-known/openid-configuration "), "{}", requests[0]);
    assert!(requests[1].starts_with("POST /connect/revocat "), "no discovery: ABP's default path: {}", requests[1]);
    let form = body_of(&requests[1]);
    for part in ["token=r1", "token_type_hint=refresh_token", "client_id=Tracker_App"] {
        assert!(form.contains(part), "{form}");
    }
    assert!(tokens.refresh_token().is_none(), "forgotten locally");
    assert_eq!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::NothingToRevoke);
}

#[tokio::test]
async fn the_revocation_endpoint_is_taken_from_discovery_only_on_this_server() {
    // A listener whose address the discovery document can name.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let discovery = format!(r#"{{"revocation_endpoint":"{url}/oauth/revoke"}}"#);
    let (tx, requests) = std::sync::mpsc::channel();
    let responses = vec![http("200 OK", &[], &discovery), http("200 OK", &[], "")];
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let _ = tx.send(read_request(&mut stream));
            let _ = stream.write_all(response.as_bytes());
        }
    });
    let api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    assert_eq!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Revoked);
    let requests: Vec<String> = requests.try_iter().collect();
    assert!(requests[1].starts_with("POST /oauth/revoke "), "{}", requests[1]);

    // Another host is never sent the token: this server's default path.
    let elsewhere = http("200 OK", &[], r#"{"revocation_endpoint":"https://evil.example.com/revoke"}"#);
    let (url, requests) = canned_server(vec![elsewhere, http("200 OK", &[], "")]);
    let api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    assert_eq!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Revoked);
    let requests: Vec<String> = requests.try_iter().collect();
    assert!(requests[1].starts_with("POST /connect/revocat "), "{}", requests[1]);
}

#[tokio::test]
async fn a_server_without_revocation_or_out_of_reach_still_forgets_the_token() {
    let (url, _requests) = canned_server(vec![http("404 Not Found", &[], ""), http("404 Not Found", &[], "")]);
    let tokens = TokenCell::with_refresh_token(Some("r1".into()));
    let api = client(&url).with_tokens(tokens.clone());
    assert!(matches!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Unsupported(_)));
    assert!(tokens.refresh_token().is_none());

    let unsupported = http("400 Bad Request", &[], r#"{"error":"unsupported_token_type"}"#);
    let (url, _requests) = canned_server(vec![http("404 Not Found", &[], ""), unsupported]);
    let api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    assert!(matches!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Unsupported(_)));

    let tokens = TokenCell::with_refresh_token(Some("r1".into()));
    let api = client("http://127.0.0.1:9").with_tokens(tokens.clone());
    assert!(matches!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Failed(_)));
    assert!(tokens.refresh_token().is_none());

    // Never over plain http:// to another computer.
    let api = ApiClient::new("http://10.0.0.5:6006", "alice@acme.com".into(), String::new())
        .with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
    assert!(matches!(api.revoke_refresh_token(Duration::from_secs(5)).await, Revocation::Failed(_)));
}

#[tokio::test]
async fn losing_workspace_access_has_its_own_error() {
    let not_member = abp(Some(codes::NOT_WORKSPACE_MEMBER), "You are not a member of this workspace.");
    let (url, _requests) = canned_server(vec![
        token_ok(),
        http("403 Forbidden", &[], &not_member),
        http("403 Forbidden", &[], &not_member),
    ]);
    let mut api = client(&url).with_workspace(Some("w1".into()));
    match api.my_projects().await {
        Err(ApiError::Workspace { refusal: WorkspaceRefusal::NotMember, message }) => {
            assert_eq!(message, "You are not a member of this workspace.")
        }
        other => panic!("expected not a member, got {other:?}"),
    }
    assert!(matches!(
        api.submit_activity(&sample_activity()).await,
        Err(SubmitError::Workspace { refusal: WorkspaceRefusal::NotMember, .. })
    ));
    assert!(matches!(classify_upload_failure(403, &not_member, None), SubmitError::Workspace { .. }), "never parked");
}

#[test]
fn terminal_codes_of_other_calls_have_their_own_errors() {
    let archived = abp(Some(codes::WORKSPACE_ARCHIVED), "This workspace is archived.");
    assert!(matches!(
        http_error(403, &archived),
        ApiError::Workspace { refusal: WorkspaceRefusal::Archived, message } if message == "This workspace is archived."
    ));
    assert!(matches!(
        http_error(400, &abp(Some(codes::WORKSPACE_REQUIRED), "?")),
        ApiError::Workspace { refusal: WorkspaceRefusal::Required, .. }
    ));
    assert!(matches!(http_error(403, &abp(Some(codes::DEVICE_REVOKED), "Removed")), ApiError::DeviceRevoked(m) if m == "Removed"));
    // Not from a 5xx, and not without the code.
    assert!(matches!(http_error(500, &archived), ApiError::Http { status: 500, .. }));
    assert!(matches!(http_error(403, ""), ApiError::Http { status: 403, error: None }));
    // Archived workspaces are read from the list.
    let mine: Vec<Workspace> = serde_json::from_str(r#"[{"id":"w1","name":"Old","isArchived":true},{"id":"w2","name":"New"}]"#).unwrap();
    assert!(mine[0].is_archived && !mine[1].is_archived);
}

#[tokio::test]
async fn every_grant_names_this_device() {
    let device = "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11";
    let refreshed = http("200 OK", &[], r#"{"access_token":"n3w","refresh_token":"r2","expires_in":3600}"#);
    let (url, requests) = canned_server(vec![token_ok(), refreshed]);
    let mut api = client(&url).with_device_id(Some(device.into()));
    api.login().await.unwrap();
    api.refresh("r1").await.unwrap();
    let requests: Vec<String> = requests.try_iter().collect();
    for (request, grant) in requests.iter().zip(["grant_type=password", "grant_type=refresh_token"]) {
        let form = body_of(request);
        assert!(form.contains(grant), "{form}");
        assert!(form.contains(&format!("device_id={device}")), "{form}");
        assert!(!request.to_lowercase().contains("__tenant"), "{request}");
    }

    // No id (it could not be stored): no field at all.
    let (url, requests) = canned_server(vec![token_ok()]);
    client(&url).with_device_id(Some("  ".into())).login().await.unwrap();
    assert!(!body_of(&requests.try_iter().next().unwrap()).contains("device_id"));
}

#[tokio::test]
async fn redirects_are_never_followed() {
    // A proxy redirecting the password elsewhere (here: back to itself,
    // which would be followed and answered 200 if redirects were on).
    let (url, requests) = canned_server(vec![
        http("307 Temporary Redirect", &[("Location", "/elsewhere")], ""),
        http("200 OK", &[], r#"{"access_token":"t","expires_in":3600}"#),
    ]);
    let error = client(&url).login().await.unwrap_err();
    assert!(matches!(&error, AuthError::Server(m) if m.contains("307")), "{error:?}");
    assert_eq!(requests.try_iter().count(), 1, "the redirect was not followed");
}

#[tokio::test]
async fn plain_http_to_another_computer_is_refused_before_anything_is_sent() {
    // Nothing listens there: the refusal must come before any connection.
    let mut api = ApiClient::new("http://192.0.2.10:6006", "alice@acme.com".into(), "pw".into());
    match api.login().await {
        Err(AuthError::Server(message)) => assert!(message.contains("not encrypted"), "{message}"),
        other => panic!("expected the refusal, got {other:?}"),
    }
    assert!(matches!(api.my_projects().await, Err(ApiError::Auth(AuthError::Server(_)))));
    assert!(matches!(api.submit_activity(&sample_activity()).await, Err(SubmitError::Transient { .. })));

    // The admin opted in: it is tried (and here fails to connect).
    let mut opted_in = ApiClient::new("http://192.0.2.10:6006", "a".into(), "b".into()).with_insecure_http(true);
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(1), opted_in.login()).await;
    assert!(result.is_err() || matches!(result, Ok(Err(AuthError::Offline(_)))), "{result:?}");
    assert!(started.elapsed() <= Duration::from_secs(2));
}
