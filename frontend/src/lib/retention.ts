import { MEMBER_CAPTURE_DELETE_LIMITS, RETENTION_LIMITS, type RetentionPolicyDto } from "./api-contract";

/* ============================================================================
 * RETENTION — how long a workspace keeps captures and tracked time (E2E-06).
 * Both policies are off unless the owner turns them on; the server sweeps at
 * start-up and every day after, unless its operator switched the sweeps off
 * (`isEnforced` false: the choice is stored, nothing is deleted).
 * ========================================================================== */

export type RetentionKind = keyof typeof RETENTION_LIMITS;

/** A policy as the form edits it (days as typed). */
export interface RetentionDraft {
  isEnabled: boolean;
  days: string;
}

/** The policy as the server returned it, with its defaults filled in for one it never stored. */
export function retentionDraftOf(kind: RetentionKind, policy: RetentionPolicyDto | null | undefined): RetentionDraft {
  const limits = RETENTION_LIMITS[kind];
  const days = Number(policy?.retentionDays);
  return {
    isEnabled: policy?.isEnabled === true,
    days: String(Number.isInteger(days) && days >= limits.min && days <= limits.max ? days : limits.default),
  };
}

/** The draft's days when they are a whole number in range; null otherwise. */
export function retentionDays(kind: RetentionKind, draft: RetentionDraft): number | null {
  const raw = draft.days.trim();
  if (!/^\d+$/.test(raw)) return null;
  const days = Number(raw);
  const limits = RETENTION_LIMITS[kind];
  return days >= limits.min && days <= limits.max ? days : null;
}

/** What is wrong with a policy's days (only checked while it is on); null when it can be saved. */
export function retentionProblem(kind: RetentionKind, draft: RetentionDraft): string | null {
  if (!draft.isEnabled) return null;
  const limits = RETENTION_LIMITS[kind];
  return retentionDays(kind, draft) === null ? `Enter a whole number of days from ${limits.min} to ${limits.max}.` : null;
}

/**
 * Whether saving `next` over `saved` deletes something the current policy
 * keeps — turning it on, or keeping fewer days — so the owner confirms first.
 */
export function retentionDeletesMore(kind: RetentionKind, saved: RetentionDraft, next: RetentionDraft): boolean {
  if (!next.isEnabled) return false;
  if (!saved.isEnabled) return true;
  const before = retentionDays(kind, saved);
  const after = retentionDays(kind, next);
  return before !== null && after !== null && after < before;
}

/** The members' own-capture window as typed: whole minutes 0–10080; null otherwise. */
export function memberWindowMinutes(raw: string): number | null {
  const value = raw.trim();
  if (!/^\d+$/.test(value)) return null;
  const minutes = Number(value);
  return minutes >= MEMBER_CAPTURE_DELETE_LIMITS.min && minutes <= MEMBER_CAPTURE_DELETE_LIMITS.max ? minutes : null;
}

/** "60 minutes", "2 hours", "1 day", "Not allowed": how the window reads. */
export function memberWindowLabel(minutes: number): string {
  if (minutes <= 0) return "Not allowed";
  if (minutes % 1440 === 0) return `${minutes / 1440} ${minutes === 1440 ? "day" : "days"}`;
  if (minutes % 60 === 0) return `${minutes / 60} ${minutes === 60 ? "hour" : "hours"}`;
  return `${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
}
