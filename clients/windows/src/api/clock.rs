//! The server's clock, from the `Date` header of its answers, and
//! `Retry-After`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, DATE, RETRY_AFTER};

/// Longest `Retry-After` honoured: a broken or hostile value must not stall
/// the queue for days.
pub(super) const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// A device clock off by more than this gets a warning (CONTRACT §1).
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
pub const CLOCK_SKEW_WARNING_SECS: i64 = 120;

/// The server's clock relative to this device's, read from the `Date`
/// header of the latest response: server minus device, in whole seconds
/// (positive = this computer's clock is behind). Clones share the value, so
/// the window can read what the worker's client last saw.
#[derive(Debug, Clone)]
pub struct ServerClock(Arc<AtomicI64>);

/// Marks "no response seen yet".
pub(super) const UNKNOWN_OFFSET: i64 = i64::MIN;

impl Default for ServerClock {
    fn default() -> Self {
        Self(Arc::new(AtomicI64::new(UNKNOWN_OFFSET)))
    }
}

impl ServerClock {
    /// Server minus device, in seconds, from the latest response.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn offset_secs(&self) -> Option<i64> {
        Some(self.0.load(Ordering::Relaxed)).filter(|&offset| offset != UNKNOWN_OFFSET)
    }

    pub fn record(&self, offset_secs: i64) {
        self.0.store(offset_secs.max(UNKNOWN_OFFSET + 1), Ordering::Relaxed);
    }

    /// Minutes the device clock is off by, once that is worth a warning:
    /// positive = behind the server, negative = ahead.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn skew_minutes(&self) -> Option<i64> {
        self.offset_secs().and_then(skew_warning_minutes)
    }
}

/// The "clock is off by N min" warning for an offset, if it exceeds
/// [`CLOCK_SKEW_WARNING_SECS`]. Timestamps are never rewritten.
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
pub fn skew_warning_minutes(offset_secs: i64) -> Option<i64> {
    (offset_secs.abs() > CLOCK_SKEW_WARNING_SECS).then(|| (offset_secs as f64 / 60.0).round() as i64)
}

/// Server minus device seconds, from an HTTP `Date` header
/// (`Tue, 05 Oct 2026 10:00:00 GMT`).
pub fn clock_offset_from_date(date_header: &str, device_now: DateTime<Utc>) -> Option<i64> {
    let server = DateTime::parse_from_rfc2822(date_header.trim()).ok()?.with_timezone(&Utc);
    Some((server - device_now).num_seconds())
}

/// How long a `Retry-After` header asks to wait: delta-seconds or an HTTP
/// date. A date is measured against the server's own clock when it sent one,
/// so a wrong device clock does not distort the wait.
pub fn parse_retry_after(value: &str, server_now: Option<DateTime<Utc>>, device_now: DateTime<Utc>) -> Option<Duration> {
    let value = value.trim();
    let wait = match value.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),
        Err(_) => {
            let at = DateTime::parse_from_rfc2822(value).ok()?.with_timezone(&Utc);
            (at - server_now.unwrap_or(device_now)).to_std().unwrap_or_default()
        }
    };
    Some(wait.min(MAX_RETRY_AFTER))
}

pub(super) fn retry_after_of(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let server_now = headers
        .get(DATE)
        .and_then(|date| date.to_str().ok())
        .and_then(|date| DateTime::parse_from_rfc2822(date.trim()).ok())
        .map(|date| date.with_timezone(&Utc));
    parse_retry_after(value, server_now, Utc::now())
}
