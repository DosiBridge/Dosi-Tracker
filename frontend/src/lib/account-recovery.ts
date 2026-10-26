import { safeRedirectPath } from "./safe-redirect";

/* ============================================================================
 * ACCOUNT RECOVERY — the rules behind "forgot", "reset" and "confirm email"
 * (FE-09, E2E-04). Pure functions; the pages do the fetching.
 *
 *  - Asking for a reset never says whether the address has an account: the
 *    server answers the same for every address (no account enumeration), and
 *    so does the page — one confirmation message whatever happened, unless
 *    the request itself could not be made (unreachable, rate-limited).
 *  - The emailed link opens /reset-password?userId=&resetToken=[&returnUrl=]
 *    (backend WebAppLinks); the return path is followed only when it is a
 *    path on this site (lib/safe-redirect).
 *  - The email confirmation link opens /confirm-email?userId=&token=.
 * ========================================================================== */

/** ABP Identity's default minimum length; the server's own policy is reported when it asks for more. */
export const MIN_NEW_CREDENTIAL_LENGTH = 6;

/** The longest value ABP accepts for a new sign-in secret. */
export const MAX_NEW_CREDENTIAL_LENGTH = 128;

/** Shown after every reset request that reached the server, whether or not the address has an account. */
export function recoveryRequestedMessage(email: string): string {
  const shown = email.trim() || "that address";
  return `If ${shown} belongs to an account here, we've emailed it a link to choose a new one. The link works once and expires; check your spam folder if it doesn't arrive in a few minutes.`;
}

/** A pragmatic address check (one @, a dotted domain): catches typos, not exotic-but-legal addresses. */
export function looksLikeEmail(value: string): boolean {
  return /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(value.trim());
}

const GUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/** The parts of an emailed reset link; null when the link is incomplete or malformed. */
export interface RecoveryLink {
  userId: string;
  resetToken: string;
  /** A path on this site to continue to afterwards, or null. */
  returnTo: string | null;
}

/** Read /reset-password's query string (the backend's WebAppLinks.ResetPassword). */
export function parseRecoveryLink(search: string | URLSearchParams): RecoveryLink | null {
  const params = typeof search === "string" ? new URLSearchParams(search) : search;
  const userId = (params.get("userId") ?? "").trim();
  const resetToken = params.get("resetToken") ?? "";
  if (!GUID_RE.test(userId) || resetToken.trim() === "" || resetToken.length > 2048) return null;
  return { userId, resetToken, returnTo: safeRedirectPath(params.get("returnUrl")) };
}

/** The parts of an emailed email-confirmation link; null when incomplete. */
export interface ConfirmationLink {
  userId: string;
  token: string;
}

/** Read /confirm-email's query string (the backend's WebAppLinks.ConfirmEmail). */
export function parseConfirmationLink(search: string | URLSearchParams): ConfirmationLink | null {
  const params = typeof search === "string" ? new URLSearchParams(search) : search;
  const userId = (params.get("userId") ?? "").trim();
  const token = params.get("token") ?? "";
  if (!GUID_RE.test(userId) || token.trim() === "" || token.length > 2048) return null;
  return { userId, token };
}

export type NewCredentialProblem = "missing" | "short" | "long" | "mismatch";

/** What is wrong with a new sign-in secret and its confirmation, before anything is sent; null when it can go. */
export function newCredentialProblem(value: string, confirmation: string): NewCredentialProblem | null {
  if (!value || !confirmation) return "missing";
  if (value.length < MIN_NEW_CREDENTIAL_LENGTH) return "short";
  if (value.length > MAX_NEW_CREDENTIAL_LENGTH) return "long";
  if (value !== confirmation) return "mismatch";
  return null;
}

/** The words for each problem (the field names stay in the page's own labels). */
export const NEW_CREDENTIAL_PROBLEM_TEXT: Record<NewCredentialProblem, string> = {
  missing: "Fill in both fields.",
  short: `Use at least ${MIN_NEW_CREDENTIAL_LENGTH} characters.`,
  long: `Use at most ${MAX_NEW_CREDENTIAL_LENGTH} characters.`,
  mismatch: "The two entries don't match.",
};

/** What a refused reset said, as the page shows it. */
export interface RecoveryFailure {
  /** The link can't be used again (expired, already used, or for another account): ask for a new one. */
  linkDead: boolean;
  message: string;
}

interface ErrorLike {
  status?: unknown;
  code?: unknown;
  serverMessage?: unknown;
  validationErrors?: unknown;
}

/**
 * Why a reset (or an email confirmation) was refused. ABP rejects a wrong or
 * expired token with a business error; a weak new value comes back as the
 * identity policy's own words, which are shown as they are.
 */
export function recoveryFailure(err: unknown, fallback: string): RecoveryFailure {
  const e = (err ?? {}) as ErrorLike;
  const code = typeof e.code === "string" ? e.code : "";
  const message = typeof e.serverMessage === "string" && e.serverMessage.trim() ? e.serverMessage.trim() : null;
  const validation = Array.isArray(e.validationErrors)
    ? (e.validationErrors as { message?: unknown }[]).map((v) => (typeof v?.message === "string" ? v.message : "")).filter(Boolean)
    : [];
  if (code === "Tracker:EmailConfirmationInvalid" || /invalidtoken/i.test(code) || (message && /invalid token|expired/i.test(message))) {
    return { linkDead: true, message: "This link has expired or was already used. Ask for a new one." };
  }
  if (validation.length > 0) return { linkDead: false, message: validation.join(" ") };
  if (e.status === 429) return { linkDead: false, message: "Too many attempts. Wait a minute and try again." };
  return { linkDead: false, message: message ?? fallback };
}
