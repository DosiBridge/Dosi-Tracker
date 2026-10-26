//! The worker's small helpers: sign-in errors, the account's debug form,
//! the window's lists and the task note.

use super::*;

// ---- Signing in ------------------------------------------------------------------------

#[test]
fn only_a_refusal_counts_as_a_rejected_sign_in() {
    // Anything but the server refusing the credentials — here a plain
    // failure, as when the server cannot be reached — keeps the sign-in.
    assert!(!is_auth_rejection(&AuthError::Offline("tcp connect error".into())));
}

/// Serve one HTTP response with `status` and `body` on a local port,
/// returning its URL.
fn one_shot_server(status: &'static str, body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

#[tokio::test]
async fn refused_credentials_are_a_rejected_sign_in() {
    let refused = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
    let mut client = ApiClient::new(one_shot_server("400 Bad Request", refused), "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("the server refuses");
    assert!(is_auth_rejection(&error));
    assert_eq!(friendly_login_error(&error), "Invalid username or password!");
}

#[tokio::test]
async fn a_server_error_is_not_a_rejected_sign_in() {
    let mut client = ApiClient::new(one_shot_server("503 Service Unavailable", ""), "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("the server fails");
    assert!(!is_auth_rejection(&error));
}

#[tokio::test]
async fn an_unreachable_server_is_not_a_rejected_sign_in() {
    // Nothing listens on port 9 (discard) locally: a real connect error.
    let mut client = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
    let error = client.login().await.expect_err("nothing is listening");
    assert!(!is_auth_rejection(&error));
}

#[test]
fn accounts_never_show_their_password() {
    let account = Account::new("https://t.example".into(), "a@acme.com".into(), "s3cr3t".into()).in_workspace("w1", "Acme");
    let shown = format!("{account:?}");
    assert!(!shown.contains("s3cr3t") && shown.contains("a@acme.com"), "{shown}");
}

// ---- The window's lists ------------------------------------------------------------------

fn block(row_id: i64) -> RecentBlock {
    let at = utc("2026-09-30T10:00:00Z");
    RecentBlock {
        row_id,
        project: "Website".into(),
        started_at: at,
        ended_at: at,
        productivity: 50,
        keyboard_hits: Some(0),
        mouse_clicks: Some(0),
        screenshot: false,
        upload: UploadState::Queued,
    }
}

#[test]
fn recent_blocks_are_newest_first_and_bounded() {
    let mut blocks = Vec::new();
    for id in 1..=(RECENT_BLOCKS as i64 + 3) {
        push_recent_block(&mut blocks, block(id));
    }
    assert_eq!(blocks.len(), RECENT_BLOCKS);
    assert_eq!(blocks[0].row_id, RECENT_BLOCKS as i64 + 3);
    assert_eq!(blocks.last().unwrap().row_id, 4);
}

#[test]
fn upload_results_are_matched_to_their_block() {
    let state = StateHandle::new();
    state.update(|s| {
        push_recent_block(&mut s.recent_blocks, block(1));
        push_recent_block(&mut s.recent_blocks, block(2));
    });
    set_upload_state(&state, 1, UploadState::Uploaded);
    set_upload_state(&state, 2, UploadState::Rejected);
    set_upload_state(&state, 99, UploadState::Uploaded); // not listed: ignored
    let blocks = state.snapshot().recent_blocks;
    assert_eq!(blocks[0].upload, UploadState::Rejected);
    assert_eq!(blocks[1].upload, UploadState::Uploaded);
}

#[test]
fn task_notes_are_one_tidy_line_of_at_most_two_hundred_characters() {
    assert_eq!(clean_note("  Invoice   export\n\tfor ACME \u{7} "), Some("Invoice export for ACME".into()));
    assert_eq!(clean_note(" \n "), None);
    let long = "word ".repeat(100);
    let kept = clean_note(&long).unwrap();
    assert!(kept.chars().count() <= TASK_NOTE_MAX_CHARS);
    assert!(!kept.ends_with(' '));
}
