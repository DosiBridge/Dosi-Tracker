import { ApiError, ApiUnreachableError } from "@/hooks/useApi";

/* ============================================================================
 * One readable sentence for a failed request, for the inline error a form or
 * a dialog shows. The server's own words win (ABP's `error.message` and its
 * validation messages are written for people); a transport failure says the
 * server couldn't be reached; anything else falls back to the caller's
 * sentence plus the HTTP status, never a bare "API PUT Error: Forbidden".
 * ========================================================================== */

/** Shown when the request never reached the server (offline, DNS, TLS, server down). */
export const UNREACHABLE_TEXT = "Couldn't reach the server. Check your connection and try again.";

/** Shown for a 403 the server didn't explain. */
export const FORBIDDEN_TEXT = "You don't have permission to do that.";

/**
 * The sentence to show for `error`. `fallback` describes what failed
 * ("Couldn't save your profile.") and is used when the server said nothing
 * more useful.
 */
export function requestErrorText(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    const rules = error.validationErrors.map((v) => v.message.trim()).filter(Boolean);
    if (rules.length > 0) return rules.join(" ");
    if (error.serverMessage) return error.serverMessage;
    if (error.status === 403) return FORBIDDEN_TEXT;
    return `${fallback} (HTTP ${error.status})`;
  }
  // fetch() rejects with a TypeError when the network fails.
  if (error instanceof ApiUnreachableError || error instanceof TypeError) return UNREACHABLE_TEXT;
  return fallback;
}

/** The ABP business-error code of a failed request ("Tracker:SeatLimitReached"), when it carried one. */
export function requestErrorCode(error: unknown): string | null {
  return error instanceof ApiError ? error.code : null;
}

/** The HTTP status of a failed request; null when it never got an answer. */
export function requestErrorStatus(error: unknown): number | null {
  return error instanceof ApiError ? error.status : null;
}
