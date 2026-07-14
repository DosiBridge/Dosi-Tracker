//! Dosi-Tracker Windows agent.
//!
//! A tiny, background, event-driven activity tracker. It logs in, fetches the
//! projects the user may track, then once per interval captures a snapshot
//! (screenshot / webcam / active window / input counts), stores it locally
//! (offline-first), and syncs pending snapshots to the backend.
//!
//! Resource profile: input listening is event-driven and heavy captures only
//! run once per interval, so idle CPU stays near zero.

mod api;
mod config;
mod model;
mod storage;
mod tracking;

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;

use api::ApiClient;
use config::AppConfig;
use storage::Storage;
use tracking::Tracker;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = AppConfig::load()?;
    tracing::info!(?cfg, "starting dosi-tracker agent");

    let store = Storage::open("dosi-tracker.db")?;
    let mut client = ApiClient::new(cfg.api_base_url.clone());

    // Credentials would normally come from a login prompt / secure store.
    let username = std::env::var("DOSI_USERNAME").unwrap_or_default();
    let password = std::env::var("DOSI_PASSWORD").unwrap_or_default();

    if !username.is_empty() {
        match client.login(&username, &password).await {
            Ok(session) => tracing::info!(user = %session.display_name, "logged in"),
            Err(e) => tracing::warn!(?e, "login failed; running in offline capture mode"),
        }
    }

    // Pick the first project the user can track (a real UI would let them choose).
    let project = client
        .projects()
        .await
        .ok()
        .and_then(|mut p| p.drain(..).next());

    let (project_id, mut perms, interval) = match &project {
        Some(p) => (
            p.id.clone(),
            config::CapturePermissions {
                screenshot: p.allow_screenshot,
                webcam: p.allow_webcam,
                keyboard: p.allow_keyboard,
                mouse: p.allow_mouse,
                active_window: p.allow_active_window,
                running_programs: p.allow_running_programs,
            },
            Duration::from_secs(p.interval_minutes.clamp(5, 60) * 60),
        ),
        None => {
            tracing::warn!("no project available; using local config defaults");
            ("local".to_string(), cfg.capture.clone(), cfg.interval())
        }
    };

    // Server-side permissions win, but never enable what local config forbade.
    perms.screenshot &= cfg.capture.screenshot;
    perms.webcam &= cfg.capture.webcam;

    let tracker = Tracker::new(perms);

    tracing::info!(?interval, %project_id, "tracking loop started");
    let mut ticker = tokio::time::interval(interval);
    // Skip the immediate first tick so the first snapshot covers a full interval.
    ticker.tick().await;

    loop {
        let started_at = Utc::now();

        tokio::select! {
            _ = ticker.tick() => {
                let activity = tracker.snapshot(&project_id, started_at);
                match store.enqueue(&activity) {
                    Ok(id) => tracing::debug!(id, "activity queued"),
                    Err(e) => tracing::error!(?e, "failed to queue activity"),
                }
                sync_pending(&client, &store).await;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutdown requested; syncing remaining activities");
                sync_pending(&client, &store).await;
                break;
            }
        }
    }

    Ok(())
}

/// Upload any locally-queued activities that have not been synced yet.
async fn sync_pending(client: &ApiClient, store: &Storage) {
    let pending = match store.pending() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(?e, "failed to read pending activities");
            return;
        }
    };

    for (id, activity) in pending {
        match client.submit_activity(&activity).await {
            Ok(_) => {
                let _ = store.mark_synced(id);
                tracing::debug!(id, "activity synced");
            }
            Err(e) => {
                // Keep it queued; retry on the next interval.
                tracing::warn!(?e, id, "sync failed; will retry");
                break;
            }
        }
    }
}
