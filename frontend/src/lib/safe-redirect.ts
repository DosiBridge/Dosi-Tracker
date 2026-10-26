/* ============================================================================
 * SAFE REDIRECTS — where `?next=` may send someone after signing in.
 *
 * The route gate (src/proxy.ts) and the sign-in page both follow `?next=`, so
 * one check guards both. A same-site path is the only acceptable answer, and
 * "starts with one slash" is not enough: WHATWG URL parsing treats `\` like
 * `/` and drops tabs and newlines, so `/\evil.example` and `/<TAB>/evil.example`
 * both resolve to https://evil.example/. The target is therefore parsed the
 * way the browser will parse it and accepted only when it stays on this
 * origin; anything odd (control characters, backslashes, encoded slashes that
 * decode to a second leading slash) is refused rather than repaired.
 * ========================================================================== */

/** A placeholder origin: only "did it stay on the same origin" matters, never which one. */
const PROBE_ORIGIN = "https://app.invalid";

/** Longer than any real app path; refusing it keeps the check cheap. */
const MAX_LENGTH = 2048;

/** C0 controls, DEL and backslash: never part of a path this app links to. */
const FORBIDDEN = /[\u0000-\u001f\u007f\\]/;

/** Paths a sign-in must never "return" to (the sign-in page itself would loop). */
const NEVER = ["/login"];

function decodedOrNull(value: string): string | null {
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
}

/**
 * The same-site path (`pathname?search#hash`) that `raw` names, or null when
 * it is missing, malformed or could leave this site.
 */
export function safeRedirectPath(raw: string | null | undefined): string | null {
  if (typeof raw !== "string" || raw.length === 0 || raw.length > MAX_LENGTH) return null;
  if (FORBIDDEN.test(raw)) return null;
  if (!raw.startsWith("/") || raw.startsWith("//")) return null;

  let url: URL;
  try {
    url = new URL(raw, PROBE_ORIGIN);
  } catch {
    return null;
  }
  if (url.origin !== PROBE_ORIGIN) return null;

  // A path that only becomes "//host" or gains a backslash once decoded is
  // refused too: some router or proxy along the way may decode it.
  const decoded = decodedOrNull(url.pathname);
  if (decoded === null || decoded.startsWith("//") || FORBIDDEN.test(decoded)) return null;
  if (NEVER.some((p) => url.pathname === p || url.pathname.startsWith(`${p}/`))) return null;

  return `${url.pathname}${url.search}${url.hash}`;
}
