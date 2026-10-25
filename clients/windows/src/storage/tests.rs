use super::limits::LimitReport;
use super::open::{set_aside, user_only_key, with_suffix};
use super::*;

const ALICE: &str = "acct\u{1f}https://tracker.acme.com\u{1f}alice@acme.com";
const BOB: &str = "acct\u{1f}https://tracker.acme.com\u{1f}bob@acme.com";
/// The same email on another organisation's server.
const ALICE_ELSEWHERE: &str = "acct\u{1f}https://tracker.globex.com\u{1f}alice@acme.com";
const WS: &str = "w1";

fn sample(project: &str) -> Activity {
    let now = Utc::now();
    Activity {
        productivity: Some(40),
        mouse_clicks: Some(1),
        keyboard_hits: Some(2),
        ..Activity::new(project, now, now)
    }
}

fn with_captures(project: &str) -> Activity {
    Activity {
        screenshot_png_base64: Some("c2NyZWVu".into()),
        screenshot_thumb_jpg_base64: Some("dGh1bWI=".into()),
        webcam_jpg_base64: Some("d2ViY2Ft".into()),
        screenshot_captured_at: Some(Utc::now()),
        screenshot_blurred: Some(false),
        ..sample(project)
    }
}

/// A row whose images weigh `bytes` in total.
fn with_image_bytes(project: &str, bytes: usize) -> Activity {
    Activity { screenshot_png_base64: Some("A".repeat(bytes)), ..sample(project) }
}

fn store() -> Storage {
    Storage::open(":memory:", None).expect("in-memory sqlite")
}

/// Everything pending for `owner`, drained through the batch API.
fn pending(s: &Storage, owner: &str) -> Vec<(i64, Activity)> {
    s.pending_batch(owner, 0, u32::MAX).unwrap()
}

fn backdate(s: &Storage, id: i64, days: u32) {
    s.conn
        .execute(
            "UPDATE activities SET created_at = datetime('now', ?2) WHERE id = ?1",
            rusqlite::params![id, format!("-{days} days")],
        )
        .unwrap();
}

fn raw_payload(s: &Storage, id: i64) -> Activity {
    let payload: String = s
        .conn
        .query_row("SELECT payload FROM activities WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn temp_db_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("dosi-tracker-test-{}.db", uuid::Uuid::new_v4()))
}

#[test]
fn owner_key_is_case_and_whitespace_insensitive() {
    assert_eq!(owner_key("https://Tracker.Acme.com/", " Alice@Acme.com "), ALICE);
    assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), owner_key("https://tracker.acme.com", "bob@acme.com"));
    // Keys of older builds (user only, or workspace + user) never match.
    assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), "user\u{1f}alice@acme.com");
    assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), "acme\u{1f}alice@acme.com");
    assert_eq!(user_only_key(ALICE).as_deref(), Some("user\u{1f}alice@acme.com"));
    assert_eq!(user_only_key("user\u{1f}alice@acme.com"), None);
}

#[test]
fn the_same_email_on_another_server_is_another_account() {
    let s = store();
    s.enqueue(&with_captures("acme-work"), ALICE, WS).unwrap();
    assert_eq!(owner_key("https://tracker.globex.com", "alice@acme.com"), ALICE_ELSEWHERE);
    // Signed in with the same email on another organisation's server:
    // nothing recorded for the first one is ever sent there.
    assert!(pending(&s, ALICE_ELSEWHERE).is_empty());
    assert_eq!(s.due_count(ALICE_ELSEWHERE).unwrap(), 0);
    assert_eq!(s.pending_count(ALICE).unwrap(), 1);
    // ...and it expires there like any other account's rows.
    let id = pending(&s, ALICE)[0].0;
    backdate(&s, id, 20);
    assert_eq!(s.purge_foreign_pending_older_than(Some(ALICE_ELSEWHERE), 14).unwrap(), 1);
}

#[test]
fn rows_tagged_with_the_user_alone_move_to_the_saved_accounts_server() {
    let path = temp_db_path();
    let path_str = path.to_string_lossy().into_owned();
    {
        let s = Storage::open(&path_str, None).unwrap();
        // As the previous build tagged them: the user, no server.
        for (owner, project) in [("user\u{1f}alice@acme.com", "alice"), ("user\u{1f}bob@acme.com", "bob")] {
            s.enqueue(&sample(project), owner, WS).unwrap();
        }
    }
    {
        let s = Storage::open(&path_str, Some(ALICE)).unwrap();
        let alice: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(alice, ["alice"], "the saved sign-in's rows, now on its server");
        assert_eq!(owned(&s, "user\u{1f}bob@acme.com"), 1, "another user's rows keep their old tag (never uploaded)");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_held_workspace_waits_without_holding_up_the_others() {
    let s = store();
    let w1 = s.enqueue(&sample("p1"), ALICE, "w1").unwrap();
    let w2 = s.enqueue(&sample("p2"), ALICE, "w2").unwrap();
    s.enqueue(&sample("bob"), BOB, "w1").unwrap();

    assert_eq!(s.hold_workspace(ALICE, "w1", "No access to Acme.").unwrap(), 1, "only Alice's rows");
    let due: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
    assert_eq!(due, [w2]);
    assert_eq!(s.due_count(ALICE).unwrap(), 1);
    assert_eq!(s.pending_count(ALICE).unwrap(), 2, "still shown as waiting");
    assert_eq!(s.next_retry_at(ALICE).unwrap(), None, "no timer: it waits for the workspace");
    assert_eq!(s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == w1).unwrap().reason.as_deref(), Some("No access to Acme."));
    assert_eq!(s.due_count(BOB).unwrap(), 1);

    assert_eq!(s.release_workspace(ALICE, "w1").unwrap(), 1);
    assert_eq!(s.due_count(ALICE).unwrap(), 2);
    assert_eq!(s.release_workspace(ALICE, "w1").unwrap(), 0, "nothing left to release");

    // Held for longer than the retention: purged with the parked rows.
    s.hold_workspace(ALICE, "w1", "archived").unwrap();
    backdate(&s, w1, 20);
    backdate(&s, w2, 20);
    assert_eq!(s.purge_rejected_older_than(14).unwrap(), 1, "the held row only");
    assert_eq!(s.pending_count(ALICE).unwrap(), 1);
}

#[test]
fn a_deferred_row_waits_for_its_time_and_counts_its_attempts() {
    let s = store();
    let first = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
    let second = s.enqueue(&sample("p2"), ALICE, WS).unwrap();
    let later = Utc::now() + chrono::Duration::minutes(10);

    assert_eq!(s.defer(first, "HTTP 422 Tracker:New: ?", later).unwrap(), 1);
    let due = s.pending_rows(ALICE, 0, 10).unwrap();
    assert_eq!(due.iter().map(|r| r.id).collect::<Vec<_>>(), [second], "the next row goes on");
    assert_eq!(s.due_count(ALICE).unwrap(), 1);
    assert_eq!(s.next_retry_at(ALICE).unwrap().map(|t| t.timestamp()), Some(later.timestamp()));

    // Its time has come: due again, with the attempts so far.
    assert_eq!(s.defer(first, "again", Utc::now() - chrono::Duration::seconds(1)).unwrap(), 2);
    let row = s.pending_rows(ALICE, 0, 10).unwrap().into_iter().find(|r| r.id == first).unwrap();
    assert_eq!(row.attempts, 2);
    assert_eq!(s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == first).unwrap().reason.as_deref(), Some("again"));
    assert_eq!(s.defer(999, "gone", later).unwrap(), 0, "an unknown row");
}

#[test]
fn each_row_keeps_its_workspace_and_rows_without_one_never_upload() {
    let s = store();
    s.enqueue(&sample("a"), ALICE, "w1").unwrap();
    s.enqueue(&sample("b"), ALICE, "w2").unwrap();
    s.conn
        .execute("INSERT INTO activities (payload, synced, owner) VALUES (?1, 0, ?2)", [serde_json::to_string(&sample("old")).unwrap(), ALICE.to_owned()])
        .unwrap();
    let rows = s.pending_rows(ALICE, 0, 10).unwrap();
    let seen: Vec<(&str, &str)> = rows.iter().map(|r| (r.activity.project_id.as_str(), r.workspace_id.as_str())).collect();
    assert_eq!(seen, [("a", "w1"), ("b", "w2")]);
    assert_eq!(s.pending_count(ALICE).unwrap(), 2, "a row without a workspace is not waiting to upload");
}

#[test]
fn enqueue_then_pending_roundtrips() {
    let s = store();
    s.enqueue(&sample("p1"), ALICE, WS).unwrap();
    let pending = pending(&s, ALICE);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.project_id, "p1");
    assert_eq!(pending[0].1.productivity, Some(40));
}

#[test]
fn only_the_signed_in_accounts_rows_are_uploadable() {
    let s = store();
    s.enqueue(&sample("alice-1"), ALICE, WS).unwrap();
    s.enqueue(&sample("bob-1"), BOB, WS).unwrap();
    s.enqueue(&sample("alice-2"), ALICE, WS).unwrap();

    let alice: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
    assert_eq!(alice, ["alice-1", "alice-2"]);
    let bob: Vec<String> = pending(&s, BOB).into_iter().map(|(_, a)| a.project_id).collect();
    assert_eq!(bob, ["bob-1"]);
    assert_eq!(s.pending_count(ALICE).unwrap(), 2);
    assert_eq!(s.pending_count(BOB).unwrap(), 1);
    assert!(pending(&s, "user\u{1f}mallory@acme.com").is_empty());
}

#[test]
fn batches_page_through_the_queue_oldest_first() {
    let s = store();
    for i in 0..5 {
        s.enqueue(&sample(&format!("p{i}")), ALICE, WS).unwrap();
    }
    s.enqueue(&sample("bob"), BOB, WS).unwrap();

    let mut seen = Vec::new();
    let mut cursor = 0;
    loop {
        let batch = s.pending_batch(ALICE, cursor, 2).unwrap();
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 2);
        cursor = batch.last().unwrap().0;
        seen.extend(batch.into_iter().map(|(_, a)| a.project_id));
    }
    assert_eq!(seen, ["p0", "p1", "p2", "p3", "p4"]);
}

#[test]
fn unreadable_payloads_are_parked_not_skipped_forever() {
    let s = store();
    s.conn
        .execute(
            "INSERT INTO activities (payload, synced, owner, workspace_id) VALUES ('{not json', 0, ?1, 'w1')",
            [ALICE],
        )
        .unwrap();
    s.enqueue(&sample("ok"), ALICE, WS).unwrap();

    let batch = pending(&s, ALICE);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].1.project_id, "ok");
    assert_eq!(s.pending_count(ALICE).unwrap(), 1);
    assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
}

#[test]
fn synced_rows_are_removed_from_the_queue() {
    let s = store();
    let id = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
    s.remove_synced(id).unwrap();
    assert!(pending(&s, ALICE).is_empty());
}

#[test]
fn rejected_rows_are_parked_not_retried() {
    let s = store();
    let id = s.enqueue(&with_captures("p1"), ALICE, WS).unwrap();
    s.mark_rejected(id, "HTTP 400: bad project").unwrap();

    // Parked rows must never come back as pending — that is what used to
    // block the whole queue behind one poison payload.
    assert!(pending(&s, ALICE).is_empty());
    assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
    assert_eq!(s.rejected_count(BOB).unwrap(), 0);

    // The images are dropped; the time data stays for diagnostics.
    let parked = raw_payload(&s, id);
    assert!(!parked.has_captures());
    assert!(parked.screenshot_captured_at.is_none() && parked.screenshot_blurred.is_none());
    assert_eq!(parked.project_id, "p1");
}

#[test]
fn recent_rows_list_both_states_with_the_server_reason() {
    let s = store();
    let first = sample("p1");
    let a = s.enqueue(&first, ALICE, WS).unwrap();
    let b = s.enqueue(&with_captures("p2"), ALICE, WS).unwrap();
    let c = s.enqueue(&sample("p3"), ALICE, WS).unwrap();
    s.enqueue(&sample("bob"), BOB, WS).unwrap();
    s.mark_rejected(b, "Overlaps another block.").unwrap();
    s.note_attempt(c, Some("Your computer clock is ahead")).unwrap();

    let rows = s.recent_rows(ALICE, 10).unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [c, b, a], "newest first, Alice's only");
    assert_eq!(rows[0].status, RowStatus::Queued);
    assert_eq!(rows[0].reason.as_deref(), Some("Your computer clock is ahead"));
    assert_eq!(rows[1].status, RowStatus::Rejected);
    assert_eq!(rows[1].reason.as_deref(), Some("Overlaps another block."));
    assert!(!rows[1].has_captures, "parking drops the images");
    assert_eq!(rows[2].project_id.as_deref(), Some("p1"));
    // Millisecond precision is what the column keeps.
    let ms = |t: DateTime<Utc>| t.timestamp_millis();
    assert_eq!(rows[2].started_at.map(ms), Some(ms(first.started_at)));
    assert_eq!(rows[2].ended_at.map(ms), Some(ms(first.ended_at)));

    assert_eq!(s.recent_rows(ALICE, 1).unwrap().len(), 1);
    // Clearing the note; a parked row's reason cannot be overwritten by it.
    s.note_attempt(c, None).unwrap();
    s.note_attempt(b, Some("ignored")).unwrap();
    let rows = s.recent_rows(ALICE, 10).unwrap();
    assert_eq!(rows[0].reason, None);
    assert_eq!(rows[1].reason.as_deref(), Some("Overlaps another block."));
}

#[test]
fn stripping_one_row_keeps_its_time_data_and_place_in_the_queue() {
    let s = store();
    let id = s.enqueue(&with_captures("p1"), ALICE, WS).unwrap();
    let other = s.enqueue(&with_captures("p2"), ALICE, WS).unwrap();
    assert!(s.strip_captures(id).unwrap());

    let row = raw_payload(&s, id);
    assert!(!row.has_captures());
    assert_eq!((row.project_id.as_str(), row.productivity, row.keyboard_hits), ("p1", Some(40), Some(2)));
    assert!(raw_payload(&s, other).has_captures(), "only that row");
    assert_eq!(pending(&s, ALICE).len(), 2, "still queued, to be resent");
    assert!(!s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == id).unwrap().has_captures);

    // Nothing left to strip: resending could not make it smaller.
    assert!(!s.strip_captures(id).unwrap());
    let bare = s.enqueue(&sample("p3"), ALICE, WS).unwrap();
    assert!(!s.strip_captures(bare).unwrap());
    // A parked row is not pending: nothing to strip.
    s.mark_rejected(other, "nope").unwrap();
    assert!(!s.strip_captures(other).unwrap());
}

#[test]
fn purge_keeps_recent_rejects_and_drops_old_ones() {
    let s = store();
    let id = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
    s.mark_rejected(id, "nope").unwrap();

    // Just-parked rows survive a 30-day purge.
    assert_eq!(s.purge_rejected_older_than(30).unwrap(), 0);
    assert_eq!(s.rejected_count(ALICE).unwrap(), 1);

    // Back-date it and it is collected.
    backdate(&s, id, 60);
    assert_eq!(s.purge_rejected_older_than(30).unwrap(), 1);
    assert_eq!(s.rejected_count(ALICE).unwrap(), 0);
}

#[test]
fn another_accounts_rows_wait_then_expire() {
    let s = store();
    let bob_recent = s.enqueue(&sample("bob-recent"), BOB, WS).unwrap();
    let bob_old = s.enqueue(&sample("bob-old"), BOB, WS).unwrap();
    let alice_old = s.enqueue(&sample("alice-old"), ALICE, WS).unwrap();
    backdate(&s, bob_old, 20);
    backdate(&s, alice_old, 20);

    // Alice signed in: Bob's old row goes, his recent one and all of hers stay.
    assert_eq!(s.purge_foreign_pending_older_than(Some(ALICE), 14).unwrap(), 1);
    assert_eq!(s.pending_count(ALICE).unwrap(), 1);
    let bob: Vec<i64> = pending(&s, BOB).into_iter().map(|(id, _)| id).collect();
    assert_eq!(bob, [bob_recent]);

    // Nobody signed in: every old pending row expires.
    assert_eq!(s.purge_foreign_pending_older_than(None, 14).unwrap(), 1);
    assert_eq!(s.pending_count(ALICE).unwrap(), 0);
    assert_eq!(s.pending_count(BOB).unwrap(), 1);
}

/// Create a database with the schema shipped before owners existed.
fn legacy_db_with_rows(path: &std::path::Path, rows: usize) {
    legacy_db_with(path, &vec![with_captures("legacy"); rows]);
}

/// A database with the schema shipped before owners existed (and before
/// incremental vacuum), holding `rows`.
fn legacy_db_with(path: &std::path::Path, rows: &[Activity]) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                error TEXT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));",
    )
    .unwrap();
    for row in rows {
        conn.execute(
            "INSERT INTO activities (payload, synced) VALUES (?1, 0)",
            [serde_json::to_string(row).unwrap()],
        )
        .unwrap();
    }
}

fn owned(s: &Storage, owner: &str) -> i64 {
    s.conn.query_row("SELECT COUNT(*) FROM activities WHERE owner = ?1", [owner], |r| r.get(0)).unwrap()
}

fn auto_vacuum_mode(s: &Storage) -> i64 {
    s.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap()
}

#[test]
fn migration_assigns_legacy_rows_to_the_saved_account_once() {
    let path = temp_db_path();
    legacy_db_with_rows(&path, 2);
    let path_str = path.to_string_lossy().into_owned();

    {
        let s = Storage::open(&path_str, Some(ALICE)).unwrap();
        assert_eq!(owned(&s, ALICE), 2);
    }
    {
        // Re-opening (even as someone else) is a no-op: the migration ran once.
        let s = Storage::open(&path_str, Some(BOB)).unwrap();
        assert_eq!(owned(&s, ALICE), 2);
        assert_eq!(owned(&s, BOB), 0);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn migration_fills_the_new_columns_and_switches_to_incremental_vacuum() {
    let path = temp_db_path();
    legacy_db_with_rows(&path, 1);
    let s = Storage::open(&path.to_string_lossy(), Some(ALICE)).unwrap();

    let rows = s.recent_rows(ALICE, 5).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].project_id.as_deref(), Some("legacy"));
    assert!(rows[0].started_at.is_some() && rows[0].ended_at.is_some());
    assert!(rows[0].has_captures, "the legacy row's images were measured");
    let bytes: i64 = s.conn.query_row("SELECT capture_bytes FROM activities", [], |r| r.get(0)).unwrap();
    assert_eq!(bytes as usize, "c2NyZWVu".len() + "dGh1bWI=".len() + "d2ViY2Ft".len());

    assert_eq!(auto_vacuum_mode(&s), 2, "INCREMENTAL");
    drop(s);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_large_older_queue_switches_to_incremental_vacuum_once_drained() {
    let path = temp_db_path();
    let rows: Vec<Activity> = (0..24).map(|i| with_image_bytes(&format!("p{i}"), 64 * 1024)).collect();
    legacy_db_with(&path, &rows);
    let limit = 512 * 1024; // the backlog (~1.5 MB) is over it
    let s = Storage::open_with(&path.to_string_lossy(), Some(ALICE), limit).unwrap();
    s.conn.execute("UPDATE activities SET workspace_id = 'w1'", []).unwrap();
    assert_eq!(auto_vacuum_mode(&s), 0, "not rebuilt while the backlog is large");
    let ids: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 24, "the queue works meanwhile");

    // Half uploaded: still too much live data.
    for &id in &ids[..12] {
        s.remove_synced(id).unwrap();
    }
    s.reclaim_space_with(limit).unwrap();
    assert_eq!(auto_vacuum_mode(&s), 0);

    // Drained: switched, and the file gives the space back.
    let size = || std::fs::metadata(&path).unwrap().len();
    let before = size();
    for &id in &ids[12..] {
        s.remove_synced(id).unwrap();
    }
    s.reclaim_space_with(limit).unwrap();
    assert_eq!(auto_vacuum_mode(&s), 2);
    assert!(size() * 4 < before, "file shrank from {before} to {}", size());
    drop(s);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn legacy_rows_without_a_saved_account_are_never_uploaded() {
    let path = temp_db_path();
    legacy_db_with_rows(&path, 1);
    let path_str = path.to_string_lossy().into_owned();

    {
        let s = Storage::open(&path_str, None).unwrap();
        // A later sign-in must not inherit them...
        let s2 = Storage::open(&path_str, Some(BOB)).unwrap();
        assert!(pending(&s2, BOB).is_empty());
        // ...and they expire like any other foreign row.
        s.conn
            .execute("UPDATE activities SET created_at = datetime('now', '-20 days')", [])
            .unwrap();
        assert_eq!(s.purge_foreign_pending_older_than(Some(BOB), 14).unwrap(), 1);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn limits_shed_images_first_and_keep_time_data() {
    let s = store();
    let ids: Vec<i64> = (0..4)
        .map(|i| s.enqueue(&with_captures(&format!("p{i}")), ALICE, WS).unwrap())
        .collect();
    backdate(&s, ids[3], 40); // newest row, but too old to keep images

    let limits = QueueLimits {
        capture_max_rows: 2,
        capture_max_bytes: u64::MAX,
        capture_max_age_days: 30,
        max_pending_rows: 100,
    };
    let report = s.enforce_limits(&limits).unwrap();
    assert_eq!(report, LimitReport { stripped: 3, dropped: 0 });

    // ids[0], ids[1]: beyond the newest two. ids[3]: too old. ids[2] keeps its images.
    for (i, &id) in ids.iter().enumerate() {
        let row = raw_payload(&s, id);
        assert_eq!(row.screenshot_png_base64.is_some(), i == 2, "row {i}");
        assert_eq!(row.screenshot_thumb_jpg_base64.is_some(), i == 2, "row {i}");
        assert_eq!(row.webcam_jpg_base64.is_some(), i == 2, "row {i}");
        assert_eq!(row.project_id, format!("p{i}"));
        assert_eq!(row.productivity, Some(40));
    }
    assert_eq!(s.pending_count(ALICE).unwrap(), 4);

    // Idempotent: already-stripped rows are not rewritten again.
    assert_eq!(s.enforce_limits(&limits).unwrap(), LimitReport::default());
}

#[test]
fn images_are_kept_newest_first_within_the_byte_budget() {
    let s = store();
    // Oldest to newest: 400 + 300 + 500 + 200 bytes of images, one row without.
    let ids: Vec<i64> = [400, 300, 500, 0, 200]
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| {
            let row = if bytes == 0 { sample(&format!("p{i}")) } else { with_image_bytes(&format!("p{i}"), bytes) };
            s.enqueue(&row, BOB, WS).unwrap()
        })
        .collect();

    let limits = QueueLimits {
        capture_max_rows: 100,
        capture_max_bytes: 1_000,
        capture_max_age_days: 30,
        max_pending_rows: 100,
    };
    // Newest first: 200 (200), 0 (200), 500 (700), 300 (1000), 400 (1400 > 1000).
    assert_eq!(s.enforce_limits(&limits).unwrap(), LimitReport { stripped: 1, dropped: 0 });
    assert!(!raw_payload(&s, ids[0]).has_captures(), "the oldest goes over the budget");
    for &id in &ids[1..] {
        assert_eq!(raw_payload(&s, id).screenshot_png_base64.is_some(), id != ids[3]);
    }

    // A tighter budget sheds the next-oldest too, never a newer row first.
    let tighter = QueueLimits { capture_max_bytes: 700, ..limits };
    assert_eq!(s.enforce_limits(&tighter).unwrap().stripped, 1);
    assert!(!raw_payload(&s, ids[1]).has_captures());
    assert!(raw_payload(&s, ids[2]).has_captures() && raw_payload(&s, ids[4]).has_captures());
}

#[test]
fn the_default_budget_is_300_mb_within_the_newest_500_rows() {
    assert_eq!(QueueLimits::DEFAULT.capture_max_bytes, 300 * 1024 * 1024);
    assert_eq!(QueueLimits::DEFAULT.capture_max_rows, 500);
}

#[test]
fn limits_drop_the_oldest_rows_beyond_the_hard_cap() {
    let s = store();
    for i in 0..5 {
        s.enqueue(&sample(&format!("p{i}")), ALICE, WS).unwrap();
    }
    let parked = s.enqueue(&sample("parked"), ALICE, WS).unwrap();
    s.mark_rejected(parked, "nope").unwrap();

    let limits = QueueLimits {
        capture_max_rows: 10,
        capture_max_bytes: u64::MAX,
        capture_max_age_days: 30,
        max_pending_rows: 3,
    };
    assert_eq!(s.enforce_limits(&limits).unwrap().dropped, 2);

    let left: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
    assert_eq!(left, ["p2", "p3", "p4"]);
    // Parked rows follow their own retention, not the pending cap.
    assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
}

#[test]
fn space_freed_by_uploads_goes_back_to_the_disk() {
    let path = temp_db_path();
    let s = Storage::open(&path.to_string_lossy(), None).unwrap();
    let ids: Vec<i64> = (0..40).map(|i| s.enqueue(&with_image_bytes(&format!("p{i}"), 64 * 1024), ALICE, WS).unwrap()).collect();
    let size = || std::fs::metadata(&path).unwrap().len();
    let full = size();
    for id in ids {
        s.remove_synced(id).unwrap();
    }
    s.reclaim_space().unwrap();
    assert!(size() * 4 < full, "file shrank from {full} to {}", size());
    drop(s);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_damaged_queue_is_moved_aside_and_replaced() {
    let path = temp_db_path();
    // Several pages of something that is not an SQLite database, with a
    // leftover journal (SQLite discards one it cannot use when it reads
    // the file; any it leaves travels with the database).
    let damaged = b"not a database. ".repeat(1024);
    std::fs::write(&path, &damaged).unwrap();
    std::fs::write(with_suffix(&path, "-journal"), b"stale journal").unwrap();

    let opened = Storage::open_or_recover(&path, Some(ALICE)).unwrap();
    let aside = opened.moved_aside.clone().expect("moved aside");
    let name = aside.file_name().unwrap().to_string_lossy().into_owned();
    let stamp = name.strip_prefix("dosi-tracker-test-").and_then(|n| n.split_once(".db.corrupt-")).map(|(_, t)| t);
    assert!(
        stamp.is_some_and(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M%SZ").is_ok()),
        "{name}"
    );
    assert_eq!(std::fs::read(&aside).unwrap(), damaged, "kept as it was, for diagnosis");
    assert!(!with_suffix(&path, "-journal").exists(), "no stale journal next to the new queue");

    // The fresh queue works.
    opened.storage.enqueue(&sample("p1"), ALICE, WS).unwrap();
    assert_eq!(opened.storage.pending_count(ALICE).unwrap(), 1);
    drop(opened);

    // A healthy queue is opened as it is.
    let reopened = Storage::open_or_recover(&path, Some(ALICE)).unwrap();
    assert!(reopened.moved_aside.is_none());
    assert_eq!(reopened.storage.pending_count(ALICE).unwrap(), 1);
    drop(reopened);
    for file in [path.clone(), aside.clone(), with_suffix(&aside, "-journal")] {
        let _ = std::fs::remove_file(file);
    }
}

#[test]
fn setting_aside_moves_the_sidecar_files_with_the_database() {
    let path = temp_db_path();
    std::fs::write(&path, b"db").unwrap();
    std::fs::write(with_suffix(&path, "-journal"), b"journal").unwrap();
    std::fs::write(with_suffix(&path, "-wal"), b"wal").unwrap();
    let at = DateTime::parse_from_rfc3339("2026-10-05T03:04:05Z").unwrap().with_timezone(&Utc);

    let aside = set_aside(&path, at).unwrap();
    assert_eq!(aside, with_suffix(&path, ".corrupt-20261005T030405Z"));
    assert_eq!(std::fs::read(&aside).unwrap(), b"db");
    // Still paired the way SQLite pairs them.
    assert_eq!(std::fs::read(with_suffix(&aside, "-journal")).unwrap(), b"journal");
    assert_eq!(std::fs::read(with_suffix(&aside, "-wal")).unwrap(), b"wal");
    for suffix in ["", "-journal", "-wal", "-shm"] {
        assert!(!with_suffix(&path, suffix).exists(), "{suffix} left behind");
    }
    for suffix in ["", "-journal", "-wal"] {
        let _ = std::fs::remove_file(with_suffix(&aside, suffix));
    }
}

#[test]
fn a_queue_that_cannot_be_opened_for_other_reasons_is_left_alone() {
    // A directory where the file should be: not damage, so nothing is renamed.
    let dir = std::env::temp_dir().join(format!("dosi-tracker-test-dir-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(Storage::open_or_recover(&dir, None).is_err());
    assert!(dir.is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

// Property-based tests: the queue invariants must hold for ANY sequence of operations,
// not just the hand-picked cases above.
use proptest::prelude::*;

proptest! {
    // The batch API returns exactly what was enqueued, in FIFO (insertion) order — the property
    // the whole offline-sync loop relies on to upload oldest-first without gaps or reordering —
    // whatever the batch size.
    #[test]
    fn pending_preserves_enqueue_count_and_fifo_order(
        projects in prop::collection::vec("[a-z][a-z0-9]{0,7}", 0..24),
        batch in 1u32..8,
    ) {
        let s = store();
        for p in &projects {
            s.enqueue(&sample(p), ALICE, WS).unwrap();
        }
        let mut order = Vec::new();
        let mut cursor = 0;
        loop {
            let rows = s.pending_batch(ALICE, cursor, batch).unwrap();
            let Some(&(last, _)) = rows.last() else { break };
            prop_assert!(rows.len() as u32 <= batch);
            cursor = last;
            order.extend(rows.into_iter().map(|(_, a)| a.project_id));
        }
        prop_assert_eq!(order, projects);
    }

    // Every row ends up either uploaded-and-removed or parked-as-rejected; none ever
    // reappears as pending, and rejected_count accounts for exactly the parked rows.
    #[test]
    fn disposed_rows_never_reappear_and_counts_are_conserved(
        keep_flags in prop::collection::vec(any::<bool>(), 0..24)
    ) {
        let s = store();
        for _ in &keep_flags {
            s.enqueue(&sample("p"), ALICE, WS).unwrap();
        }

        let ids: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
        prop_assert_eq!(ids.len(), keep_flags.len());

        let mut rejected = 0u32;
        for (id, &synced_ok) in ids.iter().zip(&keep_flags) {
            if synced_ok {
                s.remove_synced(*id).unwrap();
            } else {
                s.mark_rejected(*id, "server rejected").unwrap();
                rejected += 1;
            }
        }

        prop_assert!(pending(&s, ALICE).is_empty());
        prop_assert_eq!(s.rejected_count(ALICE).unwrap(), rejected);
    }

    // Whatever the image sizes, the newest rows keep their images and the kept total never
    // exceeds the budget.
    #[test]
    fn kept_images_never_exceed_the_byte_budget(
        sizes in prop::collection::vec(0usize..400, 1..20),
        budget in 0u64..2_000,
    ) {
        let s = store();
        let ids: Vec<i64> = sizes.iter().map(|&n| {
            let row = if n == 0 { sample("p") } else { with_image_bytes("p", n) };
            s.enqueue(&row, ALICE, WS).unwrap()
        }).collect();
        let limits = QueueLimits { capture_max_rows: 1_000, capture_max_bytes: budget, capture_max_age_days: 30, max_pending_rows: 1_000 };
        s.enforce_limits(&limits).unwrap();

        let kept: Vec<bool> = ids.iter().map(|&id| raw_payload(&s, id).has_captures()).collect();
        let kept_bytes: u64 = sizes.iter().zip(&kept).filter(|(_, &k)| k).map(|(&n, _)| n as u64).sum();
        prop_assert!(kept_bytes <= budget);
        // Kept rows are a newest-first run: once a row with images is shed, every older one is too.
        let mut shed_seen = false;
        for (i, (&n, &k)) in sizes.iter().zip(&kept).enumerate().rev() {
            if n > 0 && !k { shed_seen = true; }
            prop_assert!(!(shed_seen && k), "row {} kept after a newer one was shed", i);
        }
    }
}
