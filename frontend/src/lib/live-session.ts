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
 *
 * Web storage can be BLOCKED (enterprise policy, browser privacy settings,
 * sandboxed frames): touching `window.localStorage` then throws a
 * SecurityError. Every read and write the session layer makes goes through
 * the safe wrappers below, so blocked storage degrades (nothing remembered,
 * no live session) instead of crashing the app or the sign-in page.
 * ========================================================================== */

/** localStorage key holding the backend bearer token. */
export const TOKEN_STORAGE_KEY = "dosi-token";

/** The current workspace id; every workspace-scoped request sends it as `X-Workspace-Id`. */
export const WORKSPACE_STORAGE_KEY = "dosi-workspace";

/** Where a signed-in person picks (or creates) a workspace. */
export const WORKSPACE_CHOOSER_PATH = "/workspaces";

/** When the access token expires, in epoch ms (written with every token). */
export const TOKEN_EXPIRES_STORAGE_KEY = "dosi-token-expires-at";

/** The refresh token, present when the server issued one (`offline_access`). */
export const REFRESH_TOKEN_STORAGE_KEY = "dosi-refresh-token";

/**
 * Raised (as a cancelable window event) by the API layer when a request was
 * refused and the session couldn't renew itself. A page that can ask for the
 * password in place cancels it (`preventDefault()`) and keeps the page; when
 * nobody does, the API layer signs out and goes to /login as before.
 */
export const SESSION_UNAUTHORIZED_EVENT = "dosi-session-unauthorized";

/* ----------------------------- Safe storage ----------------------------- */

function storage(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** A stored value, or null when absent or when storage is blocked. Never throws. */
export function readStorage(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

/** Store a value; false when the browser refused (blocked storage, full quota). Never throws. */
export function writeStorage(key: string, value: string): boolean {
  try {
    const store = storage();
    if (!store) return false;
    store.setItem(key, value);
    return true;
  } catch {
    return false;
  }
}

/** Forget a value. Never throws (blocked storage holds nothing to forget). */
export function removeStorage(key: string): void {
  try {
    storage()?.removeItem(key);
  } catch {
    // Nothing is stored where storage is blocked.
  }
}

const PROBE_KEY = "dosi-storage-probe";

/**
 * Whether this browser lets the app keep anything (a session included):
 * false on the server, and where site data is blocked or the quota is full.
 */
export function isStorageAvailable(): boolean {
  const store = storage();
  if (!store) return false;
  try {
    store.setItem(PROBE_KEY, "1");
    store.removeItem(PROBE_KEY);
    return true;
  } catch {
    return false;
  }
}

/* ----------------------------- The live probe ----------------------------- */

/** True when a real backend session token is present (browser only). */
export function isLiveSession(): boolean {
  // Storage can be blocked (privacy mode, sandboxed iframe): no token → demo.
  return !!readStorage(TOKEN_STORAGE_KEY);
}

/** When the stored access token expires (epoch ms), or null when unknown. */
export function tokenExpiresAt(): number | null {
  const at = Number(readStorage(TOKEN_EXPIRES_STORAGE_KEY));
  return Number.isFinite(at) && at > 0 ? at : null;
}

/** Whether the session holds a refresh token, so it can renew itself without asking for the password. */
export function canRenewSession(): boolean {
  return !!readStorage(REFRESH_TOKEN_STORAGE_KEY);
}

const GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The current workspace id (a GUID), or null when none is chosen. Demo workspace ids never qualify. */
export function currentWorkspaceId(): string | null {
  const id = readStorage(WORKSPACE_STORAGE_KEY)?.trim();
  return id && GUID.test(id) ? id : null;
}

/** Make `id` the current workspace (false when the browser refused to store it). */
export function setCurrentWorkspace(id: string): boolean {
  return writeStorage(WORKSPACE_STORAGE_KEY, id);
}

export function clearCurrentWorkspace(): void {
  removeStorage(WORKSPACE_STORAGE_KEY);
}

/* ----------------------------- Navigation ----------------------------- */

/**
 * Leave for the sign-in page with a FULL navigation: the page that unloads
 * takes every module-level copy of the tenant's data with it, and nothing of
 * the signed-out session (or the demo workspace) renders on the way.
 */
export function redirectToLogin(): void {
  if (typeof window === "undefined") return;
  try {
    window.location.assign("/login");
  } catch {
    // A navigation the environment refuses leaves the neutral signed-out screen up.
  }
}

const WORKSPACE_FREE_PATHS = [WORKSPACE_CHOOSER_PATH, "/invite", "/login"];

/** Pages that work without a chosen workspace (the chooser, invitations, sign-in). */
export function isWorkspaceFreePath(path: string): boolean {
  return WORKSPACE_FREE_PATHS.some((p) => path === p || path.startsWith(`${p}/`));
}

/** Leave for the workspace chooser with a full navigation (no-op on pages that need no workspace). */
export function redirectToWorkspaceChooser(): void {
  if (typeof window === "undefined") return;
  try {
    if (isWorkspaceFreePath(window.location.pathname)) return;
    window.location.assign(WORKSPACE_CHOOSER_PATH);
  } catch {
    // Keep the current screen.
  }
}

/** Switch to another workspace: store it and reload everything from the dashboard. */
export function switchToWorkspace(id: string, path = "/dashboard"): void {
  setCurrentWorkspace(id);
  if (typeof window === "undefined") return;
  try {
    window.location.assign(path);
  } catch {
    // Keep the current screen.
  }
}

/** Reload the page (another tab switched the browser to a different account). */
export function reloadPage(): void {
  if (typeof window === "undefined") return;
  try {
    window.location.reload();
  } catch {
    // Nothing better to do than keep the current screen.
  }
}

/* ----------------------------- Clocks ----------------------------- */

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
