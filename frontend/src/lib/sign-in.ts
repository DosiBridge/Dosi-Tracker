import { parseApiErrorBody, parseRetryAfter } from "./api-contract";
import { apiUrl } from "./runtime-config";

/* ============================================================================
 * SIGN-IN FAILURES — what went wrong, in words that send people the right way.
 *
 * "Wrong password" is the right answer only when the server actually rejected
 * the credentials. A rate limit, a locked or deactivated account, a server
 * address that is set up wrong, an outage, a browser that blocks site data and
 * an account that must change its password each need a different next step, and
 * blaming the password for them sends people to reset passwords that were
 * fine.
 *
 * The sign-in calls (hooks/useApi) throw typed errors; they are recognised
 * here by name so this module stays in the pure layer. The OpenIddict token
 * endpoint (ABP 10, TokenController.Password) reports:
 *   - wrong email or password ............ 400 invalid_grant
 *   - locked out after failed attempts ... 400 account_locked
 *   - deactivated ........................ 400 account_inactive
 *   - unconfirmed email / phone .......... 400 invalid_grant, "RequiresConfirmUser"
 *   - two-factor required ................ 400 invalid_grant, "RequiresTwoFactor"
 *   - must change the password ........... 400 invalid_grant, "ShouldChangePasswordOnNextLogin"
 *                                          or "PeriodicallyChangePassword", plus a
 *                                          one-time `changePasswordToken`
 * Accounts are global (no workspace is named at sign-in), so a 404 can only
 * mean the configured API address doesn't point at a Dosi-Tracker server.
 * ========================================================================== */

export type SignInFailureKind =
  /** The email or password didn't match. */
  | "invalid"
  /** This site's API address is missing or doesn't lead to a Dosi-Tracker server (an operator's fix). */
  | "misconfigured"
  /** Locked after too many failed attempts. */
  | "locked"
  /** Deactivated by the workspace. */
  | "inactive"
  /** The email or phone number must be confirmed first. */
  | "confirm"
  /** The account must replace its one-time (or expired) password before signing in. */
  | "change-required"
  /** The account needs a second factor, which the web sign-in doesn't offer. */
  | "two-factor"
  /** Too many attempts from this network: wait, nothing is wrong with the details. */
  | "rate-limited"
  /** The request never reached the server. */
  | "unreachable"
  /** The server failed or is misconfigured. */
  | "server"
  /** The browser blocks site data, so no session can be kept. */
  | "storage";

export interface SignInFailure {
  kind: SignInFailureKind;
  /** Seconds to wait before trying again (rate-limited only). */
  retryAfterSeconds?: number;
  /** What the server said, when that helps (a server error's description). */
  detail?: string | null;
  /** The HTTP status, when the server answered. */
  status?: number;
}

/**
 * The sign-in rate limit is a fixed one-minute window (the API host's
 * limiter); browsers can't read a Retry-After header the server doesn't
 * expose, so a rate-limited answer without one waits out a full window.
 */
export const SIGN_IN_RETRY_DEFAULT_SECONDS = 60;

/** OAuth errors that mean the sign-in request itself is misconfigured — not the user's details. */
const PROTOCOL_ERRORS = new Set(["invalid_client", "unauthorized_client", "unsupported_grant_type", "invalid_scope", "invalid_request"]);

/** ABP's error descriptions for an account that must replace its password (ChangePasswordType). */
const CHANGE_REQUIRED = new Set(["ShouldChangePasswordOnNextLogin", "PeriodicallyChangePassword"]);

interface ErrorFields {
  name?: unknown;
  status?: unknown;
  error?: unknown;
  errorDescription?: unknown;
  retryAfterSeconds?: unknown;
}

const asText = (value: unknown): string | null => (typeof value === "string" && value.trim() !== "" ? value.trim() : null);

/** Classify whatever a sign-in call threw. */
export function classifySignInError(err: unknown): SignInFailure {
  const e = (err && typeof err === "object" ? err : {}) as ErrorFields;
  const status = typeof e.status === "number" ? e.status : undefined;
  switch (e.name) {
    case "RateLimitedError": {
      const wait = typeof e.retryAfterSeconds === "number" && e.retryAfterSeconds > 0 ? Math.ceil(e.retryAfterSeconds) : SIGN_IN_RETRY_DEFAULT_SECONDS;
      return { kind: "rate-limited", retryAfterSeconds: wait };
    }
    case "ApiUnreachableError":
      return { kind: "unreachable" };
    case "StorageUnavailableError":
      return { kind: "storage" };
    case "ApiNotConfiguredError":
      return { kind: "misconfigured" };
    case "SignInServerError":
      return { kind: "server", status, detail: asText(e.errorDescription) };
    case "InvalidCredentialsError": {
      const code = asText(e.error);
      const description = asText(e.errorDescription);
      if (status === 404) return { kind: "misconfigured", status };
      if (code === "account_locked") return { kind: "locked", status };
      if (code === "account_inactive") return { kind: "inactive", status };
      if (description && CHANGE_REQUIRED.has(description)) return { kind: "change-required", status };
      if (description === "RequiresTwoFactor") return { kind: "two-factor", status };
      if (description === "RequiresConfirmUser") return { kind: "confirm", status };
      if (code && PROTOCOL_ERRORS.has(code)) return { kind: "server", status, detail: code };
      if (status === 400 || status === 401 || status === 403) return { kind: "invalid", status };
      return { kind: "server", status, detail: description };
    }
    default:
      return { kind: "server", detail: err instanceof Error ? asText(err.message) : null };
  }
}

/** "45 seconds", "1 minute", "3 minutes". */
export function formatWait(seconds: number): string {
  const s = Math.max(1, Math.ceil(seconds));
  if (s < 60) return `${s} second${s === 1 ? "" : "s"}`;
  const m = Math.ceil(s / 60);
  return `${m} minute${m === 1 ? "" : "s"}`;
}

/**
 * The sentence to show for a failure. `secondsLeft` overrides the wait of a
 * rate-limited failure while a countdown runs.
 */
export function signInFailureMessage(failure: SignInFailure, options: { secondsLeft?: number } = {}): string {
  switch (failure.kind) {
    case "invalid":
      return "That email or password didn't match. Check them and try again.";
    case "misconfigured":
      return failure.status
        ? `This site's server address doesn't lead to a Dosi-Tracker server (HTTP ${failure.status}). Ask whoever runs this site to check its API address.`
        : "This site isn't connected to a Dosi-Tracker server yet. Ask whoever runs it to set its API address.";
    case "locked":
      return "This account is locked after too many failed sign-in attempts. Wait a few minutes and try again, or ask your workspace owner to unlock it.";
    case "inactive":
      return "This account has been deactivated. Ask your workspace owner to reactivate it.";
    case "confirm":
      return "This account's email address hasn't been confirmed yet. Ask your workspace owner to confirm it, then sign in again.";
    case "change-required":
      return "This account has to finish setting up its sign-in on the sign-in page before it can continue.";
    case "two-factor":
      return "This account uses two-step sign-in, which the web app doesn't support yet. Ask your workspace owner for help.";
    case "rate-limited":
      return `Too many sign-in attempts from this network. Nothing is wrong with your details — try again in ${formatWait(options.secondsLeft ?? failure.retryAfterSeconds ?? SIGN_IN_RETRY_DEFAULT_SECONDS)}.`;
    case "unreachable":
      return "Can't reach the server right now. Check your connection, then try again.";
    case "storage":
      return "This browser is blocking site data, so you can't stay signed in. Allow cookies and site data for this site, then try again.";
    case "server":
    default:
      return failure.detail
        ? `The server couldn't sign you in (${failure.detail}). Try again in a few minutes.`
        : `The server couldn't sign you in${failure.status ? ` (HTTP ${failure.status})` : ""}. Try again in a few minutes.`;
  }
}

/** Which form field a failure points at, so the page can move focus there. */
export function failureField(failure: SignInFailure): "credentials" | null {
  return failure.kind === "invalid" ? "credentials" : null;
}

/* ----------------------------- Change at sign-in ----------------------------- */

/** The OAuth client the web signs in as and the scopes it asks for (as hooks/useApi's loginApi does). */
export const WEB_SIGN_IN_CLIENT = { clientId: "Tracker_App", scope: "openid profile email roles offline_access Tracker" } as const;

/**
 * The token endpoint on this deployment's API (lib/runtime-config), or on
 * `baseUrl`. Accounts are global: no workspace is named at sign-in.
 */
export function tokenEndpoint(baseUrl?: string): string {
  return baseUrl ? `${baseUrl.replace(/\/+$/, "")}/connect/token` : apiUrl("/connect/token");
}

export interface ChangeAtSignInRequest {
  /** fetch, injected so the exchange is testable. */
  fetch: (input: string, init: RequestInit) => Promise<Response>;
  tokenUrl: string;
  clientId: string;
  scope: string;
  username: string;
  currentPassword: string;
  newPassword: string;
}

export type ChangeAtSignInResult =
  /** Changed: sign in with the new password. */
  | { ok: true }
  /** The server refused; `message` is its reason (e.g. the password rules), when it gave one. */
  | { ok: false; failure: SignInFailure; message: string | null };

async function postToken(req: ChangeAtSignInRequest, extra: Record<string, string>): Promise<Response> {
  const body = new URLSearchParams({
    grant_type: "password",
    username: req.username,
    password: req.currentPassword,
    client_id: req.clientId,
    scope: req.scope,
    ...extra,
  });
  return req.fetch(req.tokenUrl, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: body.toString(),
  });
}

async function readJson(res: Response): Promise<Record<string, unknown>> {
  try {
    const text = await res.text();
    const parsed: unknown = text ? JSON.parse(text) : {};
    return parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

function failureOf(res: Response, body: Record<string, unknown>): SignInFailure {
  if (res.status === 429) return { kind: "rate-limited", retryAfterSeconds: parseRetryAfter(res.headers?.get?.("Retry-After") ?? null) ?? SIGN_IN_RETRY_DEFAULT_SECONDS };
  if (res.status >= 500) return { kind: "server", status: res.status, detail: asText(body.error_description) };
  const info = parseApiErrorBody(JSON.stringify(body));
  return classifySignInError({ name: "InvalidCredentialsError", status: res.status, error: info.code, errorDescription: info.message });
}

/**
 * Replace a password the account must change, the way ABP's token endpoint
 * asks for it: the password grant answers with a one-time
 * `changePasswordToken`, and the same grant sent again with that token and the
 * new password changes it. The tokens that come back are not kept — the
 * caller signs in with the new password through its usual path, so the
 * session is stored exactly as for any other sign-in.
 */
export async function changePasswordAtSignIn(req: ChangeAtSignInRequest): Promise<ChangeAtSignInResult> {
  let challenge: Response;
  try {
    challenge = await postToken(req, {});
  } catch {
    return { ok: false, failure: { kind: "unreachable" }, message: null };
  }
  // Already changed (in another tab, say): the current password signs in as it is.
  if (challenge.ok) return { ok: true };
  const body = await readJson(challenge);
  const token = asText(body.changePasswordToken);
  if (!token) return { ok: false, failure: failureOf(challenge, body), message: null };

  let changed: Response;
  try {
    changed = await postToken(req, { ChangePasswordToken: token, NewPassword: req.newPassword });
  } catch {
    return { ok: false, failure: { kind: "unreachable" }, message: null };
  }
  if (changed.ok) return { ok: true };
  const refused = await readJson(changed);
  const failure = failureOf(changed, refused);
  // A refused change is the server's password rules talking: show its words.
  return { ok: false, failure, message: failure.kind === "invalid" ? asText(refused.error_description) : null };
}

/* ----------------------------- Workspaces ----------------------------- */

export type WorkspaceRole = "owner" | "manager" | "member";

/** GET /api/app/workspaces/mine item. */
export interface WorkspaceMembership {
  id: string;
  name: string;
  role?: WorkspaceRole | string | null;
  planName?: string | null;
  isOwner?: boolean;
  isArchived?: boolean;
  archivedTime?: string | null;
}

export function isWorkspaceOwner(w: WorkspaceMembership): boolean {
  return w.isOwner === true || (typeof w.role === "string" && w.role.toLowerCase() === "owner");
}

export const WORKSPACE_ROLE_LABELS: Record<WorkspaceRole, string> = { owner: "Owner", manager: "Manager", member: "Member" };

export function workspaceRoleLabel(role: string | null | undefined): string {
  const r = (role ?? "").toLowerCase() as WorkspaceRole;
  return WORKSPACE_ROLE_LABELS[r] ?? "Member";
}

export function workspaceInitial(w: Pick<WorkspaceMembership, "name">): string {
  return (w.name.trim().charAt(0) || "?").toUpperCase();
}

export type WorkspaceLanding =
  | { kind: "create" }
  | { kind: "select"; id: string }
  | { kind: "choose" };

/** After sign-in: none → create one; one active → open it; several → the last used active one if still a member, else choose. Archived workspaces are never auto-selected. If all memberships are archived, choose so the owner can restore. */
export function pickWorkspace(mine: readonly WorkspaceMembership[], lastUsed: string | null): WorkspaceLanding {
  if (mine.length === 0) return { kind: "create" };
  const active = mine.filter((w) => !w.isArchived);
  if (active.length === 0) return { kind: "choose" };
  if (active.length === 1) return { kind: "select", id: active[0].id };
  const last = lastUsed ? active.find((w) => w.id.toLowerCase() === lastUsed.toLowerCase()) : undefined;
  return last ? { kind: "select", id: last.id } : { kind: "choose" };
}

export const EMAIL_ALREADY_REGISTERED_MESSAGE =
  "You already have an account — sign in and create a new workspace from the sidebar.";
