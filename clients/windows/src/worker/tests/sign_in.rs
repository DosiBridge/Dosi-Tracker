//! Signing in and out, an expired sign-in, and the saved sign-in.

use super::*;

#[test]
fn a_refused_password_stops_tracking_keeps_the_queue_and_asks_for_it_again() {
    let mut h = tracking();
    h.machine.input_at(at("10:05:00"));
    h.machine.set(at("10:05:00"));
    h.control_requests();
    h.upload_commands();
    let owner = h.owner();
    h.worker.on_upload(UploadEvent::SignInExpired { owner, reason: "Invalid username or password!".into() });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::SignInExpired);
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:05:00"), Some(EndReason::SignOut))]);
    let snapshot = h.state.snapshot();
    assert!(snapshot.login_error.unwrap().contains("Invalid username or password!"));
    assert_eq!((snapshot.display_name.as_str(), snapshot.workspace.as_str()), ("alice@acme.com", "Acme"), "prefilled");
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Account(None))));
    assert!(h.control_requests().iter().any(|r| matches!(r, ControlRequest::UseAccount(None))));
    assert!(h.heartbeats().is_empty(), "no heartbeats without a sign-in");

    // The password, entered again.
    h.command(at("10:10:00"), Command::SignIn { workspace: None, username: "alice@acme.com".into(), password: "new".into(), server: None });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, AppConfig::default().api_base_url);
    // The saved workspace is picked again among several.
    let workspaces = vec![Workspace { id: "w0".into(), name: "Other".into(), role: None, plan_name: None, is_owner: false, is_archived: false },
        Workspace { id: "w1".into(), name: "Acme".into(), role: None, plan_name: None, is_owner: false, is_archived: false }];
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(workspaces) });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:10:00")));
    assert!(h.worker.account.as_ref().unwrap().password.is_empty(), "the password is let go once accepted");
    assert!(h.worker.saved.as_ref().unwrap().legacy_password.is_empty());
    assert_eq!(h.queued().len(), 1, "the earlier block is still queued for this account");
}

#[test]
fn a_sign_in_with_a_server_address_saves_it() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    h.command(at("10:00:00"), Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("ftp://nope".into()),
    });
    assert!(h.state.snapshot().login_error.unwrap().contains("https://"), "an unusable address is refused at once");
    assert!(h.control_requests().is_empty());

    h.command(at("10:00:05"), Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("tracker.acme.com/".into()),
    });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_eq!(account.server, "https://tracker.acme.com");
    let only = Workspace { id: "w1".into(), name: "Acme".into(), role: None, plan_name: None, is_owner: false, is_archived: false };
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![only]) });
    h.worker.on_control(ControlEvent::Projects(vec![project("a", 5), project("b", 5)]));
    h.worker.publish();
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::ChooseProject, "a first sign-in with several projects asks");
    assert_eq!(snapshot.server_url, "https://tracker.acme.com");
    assert_eq!(h.worker.saved.as_ref().unwrap().server.as_deref(), Some("https://tracker.acme.com"));
}

#[test]
fn signing_out_saves_the_block_says_goodbye_and_forgets_the_account() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:31"), Command::SignOut { at: at("10:04:30") });
    let farewell = h.farewells.try_recv().expect("a last heartbeat");
    assert_eq!(farewell.heartbeat.map(|hb| hb.status), Some(PresenceStatus::SignedOut));
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::SignOut { .. })));
    // Saved for the account, which uploads it when it signs in again.
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:04:30"), Some(EndReason::SignOut))]);
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.status, TrackerStatus::SignedOut);
    assert!(snapshot.display_name.is_empty() && snapshot.projects.is_empty() && snapshot.recent_blocks.is_empty());
    assert_eq!(snapshot.pending_uploads, 0, "the next account does not see this one's queue");
}

#[tokio::test]
async fn quitting_saves_the_block_at_the_click_and_says_stopped() {
    let mut h = tracking();
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:07:01"));
    let mut farewells = std::mem::replace(&mut h.farewells, tokio::sync::mpsc::unbounded_channel().1);
    let answered = tokio::spawn(async move {
        let farewell = farewells.recv().await.expect("a last heartbeat");
        let status = farewell.heartbeat.as_ref().map(|hb| hb.status);
        let _ = farewell.done.send(());
        status
    });
    h.worker.quit(at("10:07:00")).await;
    assert_eq!(answered.await.unwrap(), Some(PresenceStatus::Stopped));
    assert_eq!(h.spans(), [(at("10:00:00"), at("10:07:00"), Some(EndReason::Quit))]);
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::Shutdown { .. })));
}

// ---- Signing in safely -----------------------------------------------------------------

#[test]
fn a_plain_http_server_is_refused_unless_the_admin_allows_it() {
    let mut h = harness_at(at("10:00:00"), None, false, None);
    let sign_in = |server: &str| Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some(server.into()),
    };
    h.command(at("10:00:00"), sign_in("http://10.0.0.5:6006"));
    assert!(h.state.snapshot().login_error.unwrap().contains("not encrypted"));
    assert!(h.control_requests().is_empty(), "nothing is sent");

    // This computer is fine.
    h.command(at("10:00:01"), sign_in("http://localhost:44300"));
    assert!(matches!(h.control_requests().pop(), Some(ControlRequest::SignIn(a)) if a.server == "http://localhost:44300"));

    // The admin opted in: allowed, and the window warns.
    let mut h = harness_at(at("10:00:00"), None, false, None);
    h.worker.cfg.allow_insecure_http = true;
    h.command(at("10:00:00"), sign_in("http://10.0.0.5:6006"));
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert!(account.allow_insecure_http);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme")]) });
    assert!(h.state.snapshot().unencrypted_connection);
}

#[test]
fn every_sign_in_names_this_device_and_never_keeps_the_password() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    let mut h = harness_at(at("10:00:00"), None, false, Some(file.clone()));
    let account = typed_sign_in(&mut h, None);
    assert_eq!(account.device_id.as_deref(), Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"));
    // The server granted a refresh token with the sign-in.
    let account = Account { tokens: TokenCell::with_refresh_token(Some("r3fr3sh".into())), ..account };
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme")]) });
    let used = used_account(&mut h);
    assert_eq!(used.password, "", "the password is let go");
    assert_eq!(used.device_id.as_deref(), Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"));
    let saved = credentials::load_from(&file).expect("saved");
    assert_eq!((saved.legacy_password.as_str(), saved.refresh_token.as_deref()), ("", Some("r3fr3sh")));
    assert!(!saved.outdated_format);

    // A server that granted no refresh token: the password is not kept
    // either — the sign-in ends with its access token.
    let mut h = harness_at(at("10:00:00"), None, false, Some(file.clone()));
    let account = typed_sign_in(&mut h, None);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Acme")]) });
    assert_eq!(used_account(&mut h).password, "");
    assert!(credentials::load_from(&file).unwrap().refresh_token.is_none());

    // A sign-in saved by an older build with its password and a refresh
    // token: the password is never used, and is gone from the file at start.
    let mut older = self::saved(vec![project("p1", 5)], None);
    older.refresh_token = Some("r1".into());
    older.legacy_password = "old-pw".into();
    save_legacy(&file, &older);
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));
    assert_eq!(used_account(&mut h).password, "", "a refresh token never falls back to the password");
    let rewritten = std::fs::read(&file).unwrap();
    assert!(crate::dpapi::unprotect(&rewritten, None).is_err(), "rewritten with the app's entropy");
    let reloaded = credentials::load_from(&file).unwrap();
    assert_eq!((reloaded.legacy_password.as_str(), reloaded.outdated_format), ("", false));
    assert_eq!(reloaded.refresh_token.as_deref(), Some("r1"));
    let _ = std::fs::remove_file(&file);
}

/// Save `saved` as builds before this one did: the password in the JSON,
/// and DPAPI without the app's entropy.
fn save_legacy(file: &std::path::Path, saved: &Credentials) {
    let mut json = serde_json::to_value(saved).unwrap();
    json["password"] = serde_json::Value::String(saved.legacy_password.clone());
    std::fs::write(file, crate::dpapi::protect(&serde_json::to_vec(&json).unwrap(), None).unwrap()).unwrap();
}

#[test]
fn an_older_password_only_sign_in_is_exchanged_once_for_a_refresh_token() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    let mut older = saved(vec![project("p1", 5)], None);
    older.legacy_password = "old-pw".into();
    save_legacy(&file, &older);
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));

    // Off the disk at once; in memory only for the one exchange.
    let on_disk = credentials::load_from(&file).unwrap();
    assert_eq!((on_disk.legacy_password.as_str(), on_disk.refresh_token.as_deref()), ("", None));
    let used = used_account(&mut h);
    assert_eq!(used.password, "old-pw");

    // The server granted a refresh token for it: saved, and the password
    // is dropped from memory too.
    used.tokens.grant_for_test(Some("r1"));
    h.tick(at("10:00:02"));
    let on_disk = credentials::load_from(&file).unwrap();
    assert_eq!(on_disk.refresh_token.as_deref(), Some("r1"));
    assert!(h.worker.account.as_ref().unwrap().password.is_empty());
    assert!(h.worker.saved.as_ref().unwrap().legacy_password.is_empty());
    let _ = std::fs::remove_file(&file);
}

#[test]
fn blocks_that_cannot_be_saved_turn_the_status_red_until_saving_works_again() {
    let mut h = tracking();
    h.worker.store.break_writes_for_test(true);
    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:00"), Command::Pause { resume_after: None, at: at("10:04:00") });
    assert_eq!(h.status(), TrackerStatus::Paused, "one failure may be passing");
    assert!(h.state.snapshot().last_error.unwrap().starts_with("Could not save locally"));

    h.command(at("10:05:00"), Command::Resume { at: at("10:05:00") });
    h.machine.input_at(at("10:08:00"));
    h.command(at("10:08:00"), Command::Pause { resume_after: None, at: at("10:08:00") });
    assert_eq!(h.status(), TrackerStatus::Stopped, "saving keeps failing: not a green pill");

    h.worker.store.break_writes_for_test(false);
    h.command(at("10:09:00"), Command::Resume { at: at("10:09:00") });
    h.machine.input_at(at("10:12:00"));
    h.command(at("10:12:00"), Command::Pause { resume_after: None, at: at("10:12:00") });
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert!(h.state.snapshot().last_error.is_none(), "the error clears once a block is saved");
    assert_eq!(h.spans(), [(at("10:09:00"), at("10:12:00"), Some(EndReason::Pause))]);
}

#[test]
fn a_rotated_refresh_token_is_saved_at_once() {
    let file = std::env::temp_dir().join(format!("dosi-worker-{}.bin", uuid::Uuid::new_v4()));
    let mut signed_in = saved(vec![project("p1", 5)], None);
    signed_in.refresh_token = Some("r1".into());
    credentials::save_at(&file, &signed_in).unwrap();
    let mut h = harness_at(at("10:00:00"), credentials::load_from(&file), false, Some(file.clone()));
    let tokens = used_account(&mut h).tokens;

    // The uploader renewed the token: the server rotated the refresh token.
    tokens.grant_for_test(Some("r2"));
    h.tick(at("10:00:02"));
    assert_eq!(credentials::load_from(&file).unwrap().refresh_token.as_deref(), Some("r2"), "on the next tick, not the next pause");

    // Nothing changed: nothing written.
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    h.tick(at("10:00:04"));
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written);

    // An expired sign-in's tokens are not saved over the file.
    h.worker.on_control(ControlEvent::SignInExpired("The token is no longer valid.".into()));
    tokens.grant_for_test(Some("r3"));
    h.tick(at("10:00:06"));
    assert_eq!(credentials::load_from(&file).unwrap().refresh_token.as_deref(), Some("r2"));
    let _ = std::fs::remove_file(&file);
}

#[test]
fn signing_out_revokes_the_refresh_token_unless_the_sign_in_had_expired() {
    let mut h = tracking();
    h.command(at("10:04:00"), Command::SignOut { at: at("10:04:00") });
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::SignOut { revoke: true, .. })));

    let mut h = tracking();
    h.worker.on_control(ControlEvent::SignInExpired("The token is no longer valid.".into()));
    h.command(at("10:04:00"), Command::SignOut { at: at("10:04:00") });
    assert!(h.upload_commands().iter().any(|c| matches!(c, UploadCommand::SignOut { revoke: false, .. })));
}

#[test]
fn the_same_email_on_another_server_is_another_account() {
    let mut h = tracking();
    h.machine.input_at(at("10:04:00"));
    h.command(at("10:04:00"), Command::SignOut { at: at("10:04:00") });
    let acme = h.owner();
    assert_eq!(h.worker.store.pending_count(&acme).unwrap(), 1);

    // Signed in again with the same email, on another organisation's server.
    h.command(at("10:05:00"), Command::SignIn {
        workspace: None,
        username: "alice@acme.com".into(),
        password: "pw".into(),
        server: Some("https://tracker.globex.com".into()),
    });
    let account = match h.control_requests().pop() {
        Some(ControlRequest::SignIn(account)) => account,
        other => panic!("expected a sign-in check, got {other:?}"),
    };
    assert_ne!(account.owner(), acme);
    h.worker.on_control(ControlEvent::SignedIn { account, workspaces: Ok(vec![workspace("w1", "Globex")]) });
    let snapshot = h.state.snapshot();
    assert_eq!(snapshot.pending_uploads, 0, "Acme's blocks are not this account's");
    let uploading = h
        .upload_commands()
        .into_iter()
        .rev()
        .find_map(|c| match c {
            UploadCommand::Account(Some(account)) => Some(account),
            _ => None,
        })
        .expect("the uploader has the new account");
    assert_ne!(uploading.owner(), acme, "so it never uploads them there");
}
