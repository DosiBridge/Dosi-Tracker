//! Block timing: where each block starts and ends (CONTRACT §1 "Block timing
//! rules" and §4).
//!
//! Pure logic driven by explicit instants, so every rule is tested without a
//! clock or the OS:
//!
//! - While tracking, blocks follow one another without gaps and never exceed
//!   the interval.
//! - A block whose end finds the user inactive is *held*: if input comes back
//!   it is kept as it is; if the idle threshold confirms the user left, the
//!   cut lands at their last input — even when that was in an earlier block.
//!   (The input logs keep two hours, enough to build a held block late.)
//! - The user's own actions end the block at the moment they acted.
//! - A block shorter than a minute is dropped.

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::CapturePermissions;
use crate::model::EndReason;
use crate::tracking::MIN_BLOCK;

/// A block whose end finds no input within this long of it is held until the
/// user shows they are still there.
pub const HOLD_GRACE: TimeDelta = TimeDelta::seconds(5);

/// What a block is recorded under, fixed when the block starts: a refreshed
/// policy applies from the next block on.
#[derive(Debug, Clone)]
pub struct BlockPolicy {
    pub project_id: String,
    pub perms: CapturePermissions,
    /// The planned length of a block (5..60 min).
    pub interval: TimeDelta,
    /// No input for this long ends the block at the last input; `None` when
    /// the project turned idle detection off.
    pub idle_threshold: Option<TimeDelta>,
}

pub type BlockId = u64;

/// The block being accumulated.
#[derive(Debug, Clone)]
pub struct OpenBlock {
    pub id: BlockId,
    pub started_at: DateTime<Utc>,
    pub policy: BlockPolicy,
    pub description: Option<String>,
    /// When its screenshot is taken: a random moment in 10%..90% of the
    /// interval, so nobody can tidy their screen for it.
    pub capture_at: DateTime<Utc>,
    /// The screenshot moment has been handled.
    pub captured: bool,
}

impl OpenBlock {
    /// Where the block ends if nothing ends it earlier.
    pub fn planned_end(&self) -> DateTime<Utc> {
        self.started_at + self.policy.interval
    }
}

/// A block whose end is final: to be built and queued.
#[derive(Debug, Clone)]
pub struct FinishedBlock {
    pub id: BlockId,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub end_reason: EndReason,
    pub policy: BlockPolicy,
    pub description: Option<String>,
    /// It ended before its screenshot moment: one may be taken at the end
    /// instead (unless the end reason forbids it).
    pub capture_at_end: bool,
}

impl FinishedBlock {
    pub fn length(&self) -> TimeDelta {
        self.ended_at - self.started_at
    }
}

/// The screenshot moment of a block starting at `started_at`: `fraction`
/// (0..1) mapped into 10%..90% of the interval.
pub fn capture_moment(started_at: DateTime<Utc>, interval: TimeDelta, fraction: f64) -> DateTime<Utc> {
    let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.5 };
    let share = 0.1 + 0.8 * fraction;
    started_at + TimeDelta::milliseconds((interval.num_milliseconds() as f64 * share) as i64)
}

/// A uniformly random fraction in `[0, 1)`, from the OS random generator
/// (through a v4 UUID's 56 fully random bytes 9..16).
pub fn random_fraction() -> f64 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let mut word = [0u8; 8];
    word[..7].copy_from_slice(&bytes[9..16]);
    (u64::from_le_bytes(word) >> 3) as f64 / (1u64 << 53) as f64
}

/// One uninterrupted stretch of tracking.
#[derive(Debug)]
pub struct Stretch {
    /// When it began (the session clock).
    pub since: DateTime<Utc>,
    pub block: OpenBlock,
    /// Finished blocks waiting to see whether the user is still there.
    held: Vec<FinishedBlock>,
    /// The latest activity when the first of them was held: input after it
    /// releases them.
    held_activity: Option<DateTime<Utc>>,
    /// What the next block is recorded under.
    policy: BlockPolicy,
    description: Option<String>,
}

impl Stretch {
    /// Where the time not yet finished begins: the first held block, else
    /// the open one.
    pub fn unsaved_since(&self) -> DateTime<Utc> {
        self.held.first().map_or(self.block.started_at, |block| block.started_at)
    }

    /// Blocks are held waiting for the user.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// The blocks not finished yet: the held ones and the open one.
    pub fn block_ids(&self) -> Vec<BlockId> {
        self.held.iter().map(|block| block.id).chain(std::iter::once(self.block.id)).collect()
    }
}

/// What a tick decided.
#[derive(Debug, Default)]
pub struct TickOutcome {
    /// Blocks to build and queue, oldest first.
    pub finished: Vec<FinishedBlock>,
    /// The idle threshold passed: the stretch ended at this, the user's last
    /// activity.
    pub went_idle: Option<DateTime<Utc>>,
}

/// The blocks of the tracking stretch in progress, if any.
#[derive(Debug, Default)]
pub struct Blocks {
    stretch: Option<Stretch>,
    next_id: BlockId,
}

impl Blocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stretch(&self) -> Option<&Stretch> {
        self.stretch.as_ref()
    }

    pub fn is_open(&self) -> bool {
        self.stretch.is_some()
    }

    /// Start tracking at `at` (a no-op while a stretch is open).
    pub fn start(&mut self, at: DateTime<Utc>, policy: BlockPolicy, description: Option<String>, fraction: f64) {
        if self.stretch.is_some() {
            return;
        }
        let block = new_block(&mut self.next_id, at, &policy, description.clone(), fraction);
        self.stretch = Some(Stretch { since: at, block, held: Vec::new(), held_activity: None, policy, description });
    }

    /// Advance to `now`. `last_activity` is the user's latest keyboard or
    /// mouse input (`None`: none seen); input before the stretch began counts
    /// as the stretch's start.
    pub fn tick(&mut self, now: DateTime<Utc>, last_activity: Option<DateTime<Utc>>, fraction: &mut dyn FnMut() -> f64) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let Some(stretch) = self.stretch.as_mut() else {
            return outcome;
        };
        let activity = last_activity.map_or(stretch.since, |last| last.max(stretch.since));

        // Input since the hold — or idle detection turned off meanwhile —
        // means the held blocks were work.
        let idle_on = stretch.block.policy.idle_threshold.is_some();
        if stretch.holding() && (!idle_on || stretch.held_activity.is_some_and(|held| activity > held)) {
            outcome.finished.append(&mut release(stretch));
        }

        let hold_against = idle_on.then_some(activity);
        outcome.finished.extend(roll(stretch, &mut self.next_id, now, hold_against, fraction));

        if let Some(threshold) = stretch.block.policy.idle_threshold {
            if now - activity >= threshold {
                if let Some(stretch) = self.stretch.take() {
                    outcome.finished.extend(cut_at_last_activity(stretch, now, activity));
                    outcome.went_idle = Some(activity);
                }
            }
        }
        outcome.finished.retain(long_enough);
        outcome
    }

    /// Stop tracking: the held blocks are kept as they are, and the open block
    /// ends at `at` (the moment it happened, never later than `now`) by
    /// `reason`.
    pub fn end(&mut self, at: DateTime<Utc>, now: DateTime<Utc>, reason: EndReason) -> Vec<FinishedBlock> {
        let Some(mut stretch) = self.stretch.take() else {
            return Vec::new();
        };
        let end = at.min(now);
        let mut finished = release(&mut stretch);
        finished.extend(roll(&mut stretch, &mut self.next_id, end, None, &mut || 0.5));
        finished.push(finish(&stretch.block, end, reason));
        finished.retain(long_enough);
        finished
    }

    /// End the open block at `at` by `reason` and start the next one there;
    /// the stretch goes on (a manual sync, a task change).
    pub fn cut(&mut self, at: DateTime<Utc>, now: DateTime<Utc>, reason: EndReason, fraction: f64) -> Vec<FinishedBlock> {
        let Some(stretch) = self.stretch.as_mut() else {
            return Vec::new();
        };
        let end = at.min(now);
        let mut finished = release(stretch);
        finished.extend(roll(stretch, &mut self.next_id, end, None, &mut || 0.5));
        let end = end.max(stretch.block.started_at);
        finished.push(finish(&stretch.block, end, reason));
        stretch.block = new_block(&mut self.next_id, end, &stretch.policy, stretch.description.clone(), fraction);
        finished.retain(long_enough);
        finished
    }

    /// The task note of the next blocks; with `relabel_open`, of the open
    /// block too.
    pub fn set_description(&mut self, description: Option<String>, relabel_open: bool) {
        if let Some(stretch) = self.stretch.as_mut() {
            if relabel_open {
                stretch.block.description = description.clone();
            }
            stretch.description = description;
        }
    }

    /// A refreshed policy for the same project, from the next block on.
    pub fn update_policy(&mut self, policy: BlockPolicy) {
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.policy = policy;
        }
    }

    /// Immediately narrows permissions on both the pending stretch policy and the current open block,
    /// ensuring that any withdrawn capabilities (e.g. screenshots) cannot be scheduled or captured
    /// during the current block.
    pub fn narrow_block_permissions(&mut self, perms: &crate::config::CapturePermissions) {
        if let Some(stretch) = self.stretch.as_mut() {
            stretch.policy.perms = *perms;
            stretch.block.policy.perms = *perms;
        }
    }

    /// The open block whose screenshot moment has come, if it is due one.
    pub fn capture_due(&self, now: DateTime<Utc>) -> Option<BlockId> {
        let block = &self.stretch.as_ref()?.block;
        (!block.captured && block.policy.perms.screenshot && now >= block.capture_at).then_some(block.id)
    }

    /// The screenshot moment of block `id` has been handled.
    pub fn mark_captured(&mut self, id: BlockId) {
        if let Some(stretch) = self.stretch.as_mut().filter(|stretch| stretch.block.id == id) {
            stretch.block.captured = true;
        }
    }
}

fn new_block(next_id: &mut BlockId, at: DateTime<Utc>, policy: &BlockPolicy, description: Option<String>, fraction: f64) -> OpenBlock {
    *next_id += 1;
    OpenBlock {
        id: *next_id,
        started_at: at,
        capture_at: capture_moment(at, policy.interval, fraction),
        captured: false,
        policy: policy.clone(),
        description,
    }
}

fn finish(block: &OpenBlock, end: DateTime<Utc>, reason: EndReason) -> FinishedBlock {
    let ended_at = end.max(block.started_at);
    FinishedBlock {
        id: block.id,
        started_at: block.started_at,
        ended_at,
        end_reason: reason,
        policy: block.policy.clone(),
        description: block.description.clone(),
        capture_at_end: !block.captured && ended_at < block.capture_at,
    }
}

fn release(stretch: &mut Stretch) -> Vec<FinishedBlock> {
    stretch.held_activity = None;
    std::mem::take(&mut stretch.held)
}

/// Finish every interval boundary up to `until`, opening the next block at
/// each. With `hold_against` (the latest activity, when idle detection is
/// on), a block whose end finds the user inactive is held rather than
/// finished — and so is every one after it.
fn roll(
    stretch: &mut Stretch,
    next_id: &mut BlockId,
    until: DateTime<Utc>,
    hold_against: Option<DateTime<Utc>>,
    fraction: &mut dyn FnMut() -> f64,
) -> Vec<FinishedBlock> {
    let mut finished = Vec::new();
    while stretch.block.planned_end() <= until {
        let end = stretch.block.planned_end();
        let block = finish(&stretch.block, end, EndReason::Interval);
        let inactive = hold_against.is_some_and(|activity| end - activity > HOLD_GRACE);
        if inactive || stretch.holding() {
            if !stretch.holding() {
                stretch.held_activity = hold_against;
            }
            stretch.held.push(block);
        } else {
            finished.push(block);
        }
        stretch.block = new_block(next_id, end, &stretch.policy, stretch.description.clone(), fraction());
    }
    finished
}

/// The user left at `last`: what was tracked after it is not work. Blocks
/// ending before it stay, the one it falls in ends there (by `idle`, with no
/// capture), and later ones are dropped.
fn cut_at_last_activity(stretch: Stretch, now: DateTime<Utc>, last: DateTime<Utc>) -> Vec<FinishedBlock> {
    let open = finish(&stretch.block, now, EndReason::Idle);
    stretch
        .held
        .into_iter()
        .chain(std::iter::once(open))
        .filter_map(|mut block| {
            if block.ended_at <= last {
                Some(block)
            } else if block.started_at < last {
                block.ended_at = last;
                block.end_reason = EndReason::Idle;
                block.capture_at_end = false;
                Some(block)
            } else {
                None
            }
        })
        .collect()
}

fn long_enough(block: &FinishedBlock) -> bool {
    block.length() >= MIN_BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hms: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-10-05T{hms}Z")).unwrap().with_timezone(&Utc)
    }

    fn policy(interval_min: i64, idle_min: Option<i64>) -> BlockPolicy {
        BlockPolicy {
            project_id: "p1".into(),
            perms: CapturePermissions::default(),
            interval: TimeDelta::minutes(interval_min),
            idle_threshold: idle_min.map(TimeDelta::minutes),
        }
    }

    fn started(interval_min: i64, idle_min: Option<i64>) -> Blocks {
        let mut blocks = Blocks::new();
        blocks.start(at("10:00:00"), policy(interval_min, idle_min), Some("Invoices".into()), 0.5);
        blocks
    }

    fn mid() -> impl FnMut() -> f64 {
        || 0.5
    }

    fn spans(blocks: &[FinishedBlock]) -> Vec<(DateTime<Utc>, DateTime<Utc>, EndReason)> {
        blocks.iter().map(|b| (b.started_at, b.ended_at, b.end_reason)).collect()
    }

    #[test]
    fn tracking_blocks_follow_each_other_and_never_exceed_the_interval() {
        let mut blocks = started(10, Some(5));
        // Busy throughout: input a moment ago at every tick.
        let outcome = blocks.tick(at("10:25:00"), Some(at("10:24:59")), &mut mid());
        assert_eq!(
            spans(&outcome.finished),
            [
                (at("10:00:00"), at("10:10:00"), EndReason::Interval),
                (at("10:10:00"), at("10:20:00"), EndReason::Interval),
            ]
        );
        let stretch = blocks.stretch().unwrap();
        assert_eq!(stretch.block.started_at, at("10:20:00"), "the next block starts where the last ended");
        assert_eq!(stretch.since, at("10:00:00"), "one stretch");
        assert!(outcome.went_idle.is_none());
        assert!(outcome.finished.iter().all(|b| b.description.as_deref() == Some("Invoices")));
    }

    #[test]
    fn a_block_ending_while_the_user_is_inactive_waits_for_them() {
        let mut blocks = started(10, Some(30));
        // Reading since 10:08: the 10:10 boundary finds no input.
        let outcome = blocks.tick(at("10:10:02"), Some(at("10:08:00")), &mut mid());
        assert!(outcome.finished.is_empty(), "held, not queued yet");
        let stretch = blocks.stretch().unwrap();
        assert!(stretch.holding());
        assert_eq!(stretch.unsaved_since(), at("10:00:00"));
        assert_eq!(stretch.block.started_at, at("10:10:00"), "the next block runs meanwhile");

        // Still no input: nothing changes.
        assert!(blocks.tick(at("10:15:00"), Some(at("10:08:00")), &mut mid()).finished.is_empty());

        // A key press: it was work after all.
        let outcome = blocks.tick(at("10:16:00"), Some(at("10:15:30")), &mut mid());
        assert_eq!(spans(&outcome.finished), [(at("10:00:00"), at("10:10:00"), EndReason::Interval)]);
        assert!(!blocks.stretch().unwrap().holding());
    }

    #[test]
    fn going_idle_cuts_at_the_last_input_even_across_held_blocks() {
        let mut blocks = started(10, Some(30));
        let last_input = Some(at("10:01:00"));
        for tick in ["10:10:01", "10:20:01", "10:30:01"] {
            let outcome = blocks.tick(at(tick), last_input, &mut mid());
            assert!(outcome.finished.is_empty() && outcome.went_idle.is_none(), "{tick}: held");
        }
        // Thirty minutes without input: the user left at 10:01.
        let outcome = blocks.tick(at("10:31:00"), last_input, &mut mid());
        assert_eq!(outcome.went_idle, Some(at("10:01:00")));
        assert_eq!(spans(&outcome.finished), [(at("10:00:00"), at("10:01:00"), EndReason::Idle)]);
        assert!(!outcome.finished[0].capture_at_end, "no capture for an idle end");
        assert!(!blocks.is_open(), "nothing is tracked while away");
    }

    #[test]
    fn an_idle_block_under_a_minute_is_dropped() {
        let mut blocks = started(10, Some(5));
        let outcome = blocks.tick(at("10:05:40"), Some(at("10:00:30")), &mut mid());
        assert_eq!(outcome.went_idle, Some(at("10:00:30")));
        assert!(outcome.finished.is_empty());
        assert!(!blocks.is_open());
    }

    #[test]
    fn no_input_at_all_since_the_start_cuts_at_the_start() {
        // Started at sign-in by Windows, nobody at the desk.
        let mut blocks = started(10, Some(5));
        let outcome = blocks.tick(at("10:05:00"), Some(at("09:30:00")), &mut mid());
        assert_eq!(outcome.went_idle, Some(at("10:00:00")), "input before the stretch does not count");
        assert!(outcome.finished.is_empty());
        let mut blocks = started(10, Some(5));
        assert_eq!(blocks.tick(at("10:05:00"), None, &mut mid()).went_idle, Some(at("10:00:00")));
    }

    #[test]
    fn with_idle_detection_off_nothing_is_held_or_cut() {
        let mut blocks = started(10, None);
        let outcome = blocks.tick(at("11:00:01"), Some(at("10:00:05")), &mut mid());
        assert_eq!(outcome.finished.len(), 6);
        assert!(outcome.went_idle.is_none());
        assert!(!blocks.stretch().unwrap().holding());
    }

    #[test]
    fn held_blocks_are_released_when_idle_detection_is_turned_off() {
        let mut blocks = started(10, Some(30));
        blocks.tick(at("10:10:01"), Some(at("10:02:00")), &mut mid());
        assert!(blocks.stretch().unwrap().holding());
        // The project now has idle detection off: the open block keeps its
        // own policy, the next one gets the new one.
        blocks.update_policy(policy(10, None));
        assert!(blocks.tick(at("10:15:00"), Some(at("10:02:00")), &mut mid()).finished.is_empty());
        let outcome = blocks.tick(at("10:20:01"), Some(at("10:02:00")), &mut mid());
        // The 10:10-10:20 block was still under the old policy: held too.
        assert!(outcome.finished.is_empty());
        let outcome = blocks.tick(at("10:20:02"), Some(at("10:02:00")), &mut mid());
        assert_eq!(outcome.finished.len(), 2, "released once the open block has no idle threshold");
        assert!(blocks.stretch().unwrap().block.policy.idle_threshold.is_none());
    }

    #[test]
    fn the_user_action_ends_the_block_at_its_own_timestamp() {
        let mut blocks = started(10, Some(5));
        // Paused at 10:04:30; the worker got to it two seconds later.
        let finished = blocks.end(at("10:04:30"), at("10:04:32"), EndReason::Pause);
        assert_eq!(spans(&finished), [(at("10:00:00"), at("10:04:30"), EndReason::Pause)]);
        // The screenshot moment (50% of the interval: 10:05) was not reached.
        assert!(finished[0].capture_at_end);
        assert!(!blocks.is_open());
        assert!(blocks.end(at("10:06:00"), at("10:06:00"), EndReason::Pause).is_empty(), "nothing open");
    }

    #[test]
    fn an_action_after_a_missed_boundary_never_exceeds_the_interval() {
        let mut blocks = started(10, Some(5));
        let finished = blocks.end(at("10:12:00"), at("10:12:01"), EndReason::Quit);
        assert_eq!(
            spans(&finished),
            [
                (at("10:00:00"), at("10:10:00"), EndReason::Interval),
                (at("10:10:00"), at("10:12:00"), EndReason::Quit),
            ]
        );
    }

    #[test]
    fn a_click_just_before_a_boundary_the_worker_already_handled_drops_the_sliver() {
        let mut blocks = started(10, Some(5));
        blocks.tick(at("10:10:00"), Some(at("10:09:59")), &mut mid());
        // Pause clicked at 10:09:59.5, reaching the worker after the boundary.
        let click = at("10:09:59") + TimeDelta::milliseconds(500);
        assert!(blocks.end(click, at("10:10:01"), EndReason::Pause).is_empty());
    }

    #[test]
    fn a_future_timestamp_is_clamped_to_now() {
        let mut blocks = started(10, Some(5));
        let finished = blocks.end(at("10:30:00"), at("10:03:00"), EndReason::Lock);
        assert_eq!(spans(&finished), [(at("10:00:00"), at("10:03:00"), EndReason::Lock)]);
    }

    #[test]
    fn stopping_keeps_held_blocks_as_work() {
        let mut blocks = started(10, Some(30));
        blocks.tick(at("10:10:01"), Some(at("10:03:00")), &mut mid());
        // The screen locks at 10:12, within the idle threshold.
        let finished = blocks.end(at("10:12:00"), at("10:12:00"), EndReason::Lock);
        assert_eq!(
            spans(&finished),
            [
                (at("10:00:00"), at("10:10:00"), EndReason::Interval),
                (at("10:10:00"), at("10:12:00"), EndReason::Lock),
            ]
        );
    }

    #[test]
    fn short_blocks_are_dropped_when_stopping() {
        let mut blocks = started(10, Some(5));
        assert!(blocks.end(at("10:00:59"), at("10:00:59"), EndReason::Project).is_empty());
        let mut blocks = started(10, Some(5));
        assert_eq!(blocks.end(at("10:01:00"), at("10:01:00"), EndReason::Project).len(), 1, "a full minute is kept");
    }

    #[test]
    fn a_cut_ends_the_block_and_the_stretch_goes_on() {
        let mut blocks = started(10, Some(5));
        blocks.set_description(Some("Client call".into()), false);
        let finished = blocks.cut(at("10:04:00"), at("10:04:01"), EndReason::Task, 0.5);
        assert_eq!(spans(&finished), [(at("10:00:00"), at("10:04:00"), EndReason::Task)]);
        assert_eq!(finished[0].description.as_deref(), Some("Invoices"), "the old note stays with its block");
        let stretch = blocks.stretch().unwrap();
        assert_eq!(stretch.since, at("10:00:00"));
        assert_eq!(stretch.block.started_at, at("10:04:00"));
        assert_eq!(stretch.block.description.as_deref(), Some("Client call"));
        assert_eq!(stretch.block.planned_end(), at("10:14:00"), "a full interval from the cut");
    }

    #[test]
    fn a_short_block_can_take_the_new_note_instead_of_ending() {
        let mut blocks = started(10, Some(5));
        blocks.set_description(Some("Client call".into()), true);
        let stretch = blocks.stretch().unwrap();
        assert_eq!(stretch.block.started_at, at("10:00:00"));
        assert_eq!(stretch.block.description.as_deref(), Some("Client call"));
    }

    #[test]
    fn a_refreshed_policy_applies_from_the_next_block() {
        let mut blocks = started(10, Some(5));
        let mut shorter = policy(5, Some(5));
        shorter.perms.screenshot = false;
        blocks.update_policy(shorter);
        let outcome = blocks.tick(at("10:10:00"), Some(at("10:09:59")), &mut mid());
        assert_eq!(outcome.finished[0].policy.interval, TimeDelta::minutes(10), "the open block keeps its policy");
        let stretch = blocks.stretch().unwrap();
        assert_eq!(stretch.block.policy.interval, TimeDelta::minutes(5));
        assert_eq!(stretch.block.planned_end(), at("10:15:00"));
    }

    #[test]
    fn the_screenshot_moment_is_random_within_ten_to_ninety_percent() {
        let start = at("10:00:00");
        let ten = TimeDelta::minutes(10);
        assert_eq!(capture_moment(start, ten, 0.0), at("10:01:00"));
        assert_eq!(capture_moment(start, ten, 1.0), at("10:09:00"));
        assert_eq!(capture_moment(start, ten, 0.5), at("10:05:00"));
        assert_eq!(capture_moment(start, ten, f64::NAN), at("10:05:00"));
        for _ in 0..200 {
            let fraction = random_fraction();
            assert!((0.0..1.0).contains(&fraction));
            let moment = capture_moment(start, ten, fraction);
            assert!(moment >= at("10:01:00") && moment <= at("10:09:00"));
        }
        // Not a constant.
        let draws: std::collections::HashSet<u64> = (0..20).map(|_| random_fraction().to_bits()).collect();
        assert!(draws.len() > 15);
    }

    #[test]
    fn the_screenshot_is_due_once_at_its_moment_when_allowed() {
        let mut blocks = started(10, Some(5));
        assert_eq!(blocks.capture_due(at("10:04:59")), None);
        let id = blocks.capture_due(at("10:05:00")).expect("due at 50%");
        blocks.mark_captured(id);
        assert_eq!(blocks.capture_due(at("10:06:00")), None, "once per block");
        // Ending after the moment: no second capture at the end.
        assert!(!blocks.end(at("10:07:00"), at("10:07:00"), EndReason::Pause)[0].capture_at_end);

        let mut no_screens = Blocks::new();
        let mut none = policy(10, Some(5));
        none.perms.screenshot = false;
        no_screens.start(at("10:00:00"), none, None, 0.5);
        assert_eq!(no_screens.capture_due(at("10:09:00")), None);
    }
}
