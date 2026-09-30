import { NOW } from "./mock-data";

/* ============================================================================
 * LIVE vs DEMO — the one probe every surface uses.
 *
 * A session is LIVE when the browser holds a real backend bearer token (written
 * by `loginApi`). In a live session the app must show ONLY real data or honest
 * empty / error states — never the built-in demo dataset (demo people, demo
 * companies, demo cards, demo notifications). Demo mode (no token) keeps the
 * seeded dataset so the product can be explored offline and tested.
 *
 * Pages used to re-implement `localStorage.getItem("dosi-token")` inline; call
 * `isLiveSession()` instead so the rule lives in exactly one place.
 * ========================================================================== */

/** localStorage key holding the backend bearer token. */
export const TOKEN_STORAGE_KEY = "dosi-token";

/** True when a real backend session token is present (browser only). */
export function isLiveSession(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return !!window.localStorage.getItem(TOKEN_STORAGE_KEY);
  } catch {
    // Storage can be blocked (privacy mode, sandboxed iframe): no token → demo.
    return false;
  }
}

/**
 * The clock pages pivot on. Demo data is generated around a FIXED instant (so
 * SSR and client render identically); real data happens now. Using the demo
 * instant in a live session would put every "today"/"last 7 days" window
 * months away from the real rows.
 */
export function referenceNow(): Date {
  return isLiveSession() ? new Date() : new Date(NOW);
}

/** "5m ago" / "3h ago" / "2d ago" relative to the reference clock. Future instants read "just now". */
export function agoLabel(iso: string, now: Date = referenceNow()): string {
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return "—";
  const mins = Math.max(0, Math.floor((now.getTime() - at) / 60000));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}
