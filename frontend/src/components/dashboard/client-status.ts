import { agoLabel } from "@/lib/live-session";
import { countDays, formatDayKey } from "@/lib/time-zone";

/* ============================================================================
 * A project's status for its client, derived from data — when work was last
 * tracked on it — never asserted ("On track" with nothing tracked for weeks).
 * There is no budget or deadline in the data, so the status speaks only to
 * recency, and says what it is based on.
 * ========================================================================== */

export interface ClientProjectStatus {
  label: string;
  /** What the label is based on, in words. */
  detail: string;
  tone: "success" | "info" | "warning" | "muted";
}

const DAY_MS = 86_400_000;

/**
 * "Active" when work was tracked within the last two days, "Active this week"
 * within seven, "Quiet for N days" within thirty, otherwise "No recent work".
 * `lastWorkAt` is a UTC instant (null when nothing was tracked in the window).
 */
export function clientProjectStatus(lastWorkAt: string | null, now: Date): ClientProjectStatus {
  const at = lastWorkAt ? Date.parse(lastWorkAt) : Number.NaN;
  if (!Number.isFinite(at)) {
    return { label: "No recent work", detail: "Nothing tracked in the last 30 days", tone: "muted" };
  }
  const days = Math.max(0, (now.getTime() - at) / DAY_MS);
  return byAge(days, `Last work tracked ${agoLabel(new Date(at).toISOString(), now)}`);
}

/**
 * The same badge when only the calendar day of the last work is known (a
 * daily series): the age is counted in whole days of the reporting zone, and
 * the detail names the day, never a time of day the data doesn't hold.
 * `lastWorkDay` and `todayKey` are day keys ("2026-07-14") of one zone.
 */
export function clientProjectStatusOnDay(lastWorkDay: string | null, todayKey: string): ClientProjectStatus {
  if (!lastWorkDay) return clientProjectStatus(null, new Date(0));
  const days = Math.max(0, countDays(lastWorkDay, todayKey) - 1);
  const when =
    days === 0 ? "today" : days === 1 ? "yesterday" : `on ${formatDayKey(lastWorkDay, { weekday: true })}`;
  return byAge(days, `Last work tracked ${when}`);
}

function byAge(days: number, detail: string): ClientProjectStatus {
  if (days <= 2) return { label: "Active", detail, tone: "success" };
  if (days <= 7) return { label: "Active this week", detail, tone: "info" };
  if (days <= 30) return { label: `Quiet for ${Math.floor(days)} days`, detail, tone: "warning" };
  return { label: "No recent work", detail, tone: "muted" };
}
