import { ERASURE_REASON_LIMITS, MEMBER_CAPTURE_DELETE_LIMITS } from "./api-contract";

/* ============================================================================
 * ERASURE — who may delete what, and how it is confirmed (E2E-07). The server
 * decides (and records every deletion: who, when, what, why — never what it
 * contained); these rules only decide which controls a page offers, so a
 * button never invites a request that is bound to be refused.
 *
 *  - A time block with all its captures: owners and managers
 *    (Tracker.Activities.Delete), with a reason. Only an owner deletes an
 *    owner's data.
 *  - One capture: owners and managers, with a reason; a member their own,
 *    within the workspace's window after upload (memberCaptureDeleteMinutes,
 *    default 60, 0 = off), reason optional.
 *  - A workspace: its owner, typing its name (trimmed, any letter case, as
 *    the server compares it).
 *  - An account: its holder, with their current sign-in secret; not while they
 *    are a workspace's only owner, never a platform administrator's.
 * ========================================================================== */

/** The roles a deletion depends on. */
export interface ErasureViewer {
  /** The viewer holds Tracker.Activities.Delete (owners and managers). */
  canDeleteActivities: boolean;
  /** The viewer is an owner of the workspace. */
  isOwner: boolean;
  userId: string;
}

/** What the viewer may delete of one block. */
export interface BlockErasure {
  /** The block with every capture. */
  block: boolean;
  /** Single captures: "any" (with a reason), "own" (within the window, reason optional) or "none". */
  captures: "any" | "own" | "none";
  /** When "own": until when (epoch ms); null when there is no limit to show. */
  ownUntil: number | null;
}

const NOTHING: BlockErasure = { block: false, captures: "none", ownUntil: null };

/**
 * What the viewer may delete of a block owned by `blockUserId` (an owner's data
 * only by an owner). A member's own captures follow the workspace window,
 * counted from the block's upload; without a known upload time the member is
 * offered nothing (the server would refuse a capture outside its window).
 */
export function blockErasure(input: {
  viewer: ErasureViewer;
  blockUserId: string;
  /** The block's member is an owner of the workspace. */
  blockOwnerIsOwner: boolean;
  /** When the server received the block (ISO), when known. */
  uploadedAt?: string | null;
  /** The workspace's member window; null/undefined: the server's default. */
  windowMinutes?: number | null;
  now: Date;
}): BlockErasure {
  const { viewer } = input;
  if (viewer.canDeleteActivities && (viewer.isOwner || !input.blockOwnerIsOwner)) {
    return { block: true, captures: "any", ownUntil: null };
  }
  if (input.blockUserId !== viewer.userId) return NOTHING;
  const window = typeof input.windowMinutes === "number" && Number.isFinite(input.windowMinutes) ? input.windowMinutes : MEMBER_CAPTURE_DELETE_LIMITS.default;
  const uploaded = input.uploadedAt ? Date.parse(input.uploadedAt) : Number.NaN;
  if (window <= 0 || !Number.isFinite(uploaded)) return NOTHING;
  const until = uploaded + window * 60_000;
  return input.now.getTime() <= until ? { block: false, captures: "own", ownUntil: until } : NOTHING;
}

export type ReasonProblem = "missing" | "short" | "long";

/** What is wrong with a deletion's reason (the server keeps 3–512 characters); null when it can go. */
export function erasureReasonProblem(reason: string, required: boolean): ReasonProblem | null {
  const trimmed = reason.trim();
  if (!trimmed) return required ? "missing" : null;
  if (trimmed.length < ERASURE_REASON_LIMITS.min) return "short";
  if (trimmed.length > ERASURE_REASON_LIMITS.max) return "long";
  return null;
}

export const REASON_PROBLEM_TEXT: Record<ReasonProblem, string> = {
  missing: "Say why it is deleted. The reason is kept in the deletion record.",
  short: `Use at least ${ERASURE_REASON_LIMITS.min} characters.`,
  long: `Use at most ${ERASURE_REASON_LIMITS.max} characters.`,
};

/** Whether the typed confirmation matches the workspace's name, as the server compares it (trimmed, any case). */
export function workspaceNameConfirmed(typed: string, name: string): boolean {
  const want = name.trim();
  return want !== "" && typed.trim().toLocaleLowerCase("en") === want.toLocaleLowerCase("en");
}

interface ErrorLike {
  status?: unknown;
  code?: unknown;
  serverMessage?: unknown;
  data?: unknown;
}

/** Why a deletion (of a block, capture, workspace or account) was refused, in words. */
export function erasureErrorText(err: unknown, fallback: string): string {
  const e = (err ?? {}) as ErrorLike;
  const code = typeof e.code === "string" ? e.code : "";
  const serverMessage = typeof e.serverMessage === "string" && e.serverMessage.trim() ? e.serverMessage.trim() : null;
  switch (code) {
    case "Tracker:CaptureDeleteNotAllowed":
      return "The time to delete your own capture has passed. Ask a workspace owner or manager to delete it.";
    case "Tracker:RoleNotAllowed":
      return "Only a workspace owner can delete an owner's data.";
    case "Tracker:WorkspaceDeleteConfirmationMismatch":
      return "Type the workspace's name exactly to confirm.";
    case "Tracker:InvalidPassword":
      return "That isn't your current password.";
    case "Tracker:SoleWorkspaceOwner":
      return serverMessage ?? "You're the only owner of a workspace. Make someone else an owner, or delete the workspace, first.";
    case "Tracker:AccountDeletionNotAllowed":
      return "A platform administrator's account can't be deleted here.";
    default:
      break;
  }
  if (e.status === 404) return "It was already deleted.";
  if (e.status === 403) return "You aren't allowed to delete this.";
  return serverMessage ?? fallback;
}

/** "Nov 2, 2026" — when a deleted workspace's data is purged for good. */
export function purgeDateLabel(iso: string | null | undefined): string | null {
  const at = iso ? Date.parse(/([zZ]|[+-]\d{2}:?\d{2})$/.test(iso) ? iso : `${iso}Z`) : Number.NaN;
  return Number.isFinite(at) ? new Date(at).toLocaleDateString("en", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" }) : null;
}
