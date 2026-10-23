import { describe, expect, it, vi } from "vitest";
import {
  API_BASE_URL,
  isWorkspaceOwner,
  pickWorkspace,
  workspaceInitial,
  workspaceRoleLabel,
  SIGN_IN_RETRY_DEFAULT_SECONDS,
  WEB_SIGN_IN_CLIENT,
  changePasswordAtSignIn,
  classifySignInError,
  failureField,
  formatWait,
  signInFailureMessage,
  tokenEndpoint,
  type SignInFailure,
} from "./sign-in";

/** An error shaped like the ones hooks/useApi throws (recognised by name). */
function thrown(name: string, fields: Record<string, unknown> = {}): Error {
  return Object.assign(new Error(name), { name, ...fields });
}

const invalid = (status: number, error: string | null, errorDescription: string | null) =>
  thrown("InvalidCredentialsError", { status, error, errorDescription });

describe("classifySignInError — every way a sign-in fails, told apart", () => {
  it.each<[string, Error, SignInFailure]>([
    ["wrong email or password", invalid(400, "invalid_grant", "Invalid username or password!"), { kind: "invalid", status: 400 }],
    ["no workspace by that name (ABP tenant resolution: 404)", invalid(404, null, null), { kind: "unknown-workspace", status: 404 }],
    ["locked after failed attempts", invalid(400, "account_locked", "The user account has been locked out…"), { kind: "locked", status: 400 }],
    ["deactivated", invalid(400, "account_inactive", "You are not allowed to login!"), { kind: "inactive", status: 400 }],
    ["one-time password must be changed", invalid(400, "invalid_grant", "ShouldChangePasswordOnNextLogin"), { kind: "change-required", status: 400 }],
    ["expired password must be changed", invalid(400, "invalid_grant", "PeriodicallyChangePassword"), { kind: "change-required", status: 400 }],
    ["second factor required", invalid(400, "invalid_grant", "RequiresTwoFactor"), { kind: "two-factor", status: 400 }],
    ["email not confirmed", invalid(400, "invalid_grant", "RequiresConfirmUser"), { kind: "confirm", status: 400 }],
    ["a misconfigured client is the server's problem", invalid(400, "invalid_client", null), { kind: "server", status: 400, detail: "invalid_client" }],
    ["an odd status is the server's problem", invalid(418, null, "teapot"), { kind: "server", status: 418, detail: "teapot" }],
    ["no server", thrown("ApiUnreachableError"), { kind: "unreachable" }],
    ["a 5xx", thrown("SignInServerError", { status: 503, errorDescription: "Database offline" }), { kind: "server", status: 503, detail: "Database offline" }],
    ["blocked site data", thrown("StorageUnavailableError"), { kind: "storage" }],
    ["rate limited, Retry-After known", thrown("RateLimitedError", { retryAfterSeconds: 42.2 }), { kind: "rate-limited", retryAfterSeconds: 43 }],
    ["rate limited, no Retry-After (the window is a minute)", thrown("RateLimitedError", { retryAfterSeconds: null }), { kind: "rate-limited", retryAfterSeconds: SIGN_IN_RETRY_DEFAULT_SECONDS }],
  ])("%s", (_label, err, expected) => {
    expect(classifySignInError(err)).toEqual(expected);
  });

  it("anything unrecognised is a server problem, carrying its message", () => {
    expect(classifySignInError(new Error("Workspace registration failed (HTTP 500)"))).toEqual({
      kind: "server",
      detail: "Workspace registration failed (HTTP 500)",
    });
    expect(classifySignInError("boom")).toEqual({ kind: "server", detail: null });
    expect(classifySignInError(null)).toEqual({ kind: "server", detail: null });
  });
});

describe("signInFailureMessage", () => {
  it("blames the credentials only when the server rejected them", () => {
    expect(signInFailureMessage({ kind: "invalid" })).toBe("That email or password didn't match. Check them and try again.");
    for (const kind of ["rate-limited", "unreachable", "server", "locked", "unknown-workspace", "storage"] as const) {
      expect(signInFailureMessage({ kind }), kind).not.toMatch(/didn't match/);
    }
  });

  it("names the workspace that wasn't found", () => {
    expect(signInFailureMessage({ kind: "unknown-workspace" }, { workspace: "acme" })).toMatch(/workspace named “acme”/);
    expect(signInFailureMessage({ kind: "unknown-workspace" })).toMatch(/couldn't find that workspace/);
  });

  it("says how long to wait out a rate limit, and that nothing is wrong with the details", () => {
    expect(signInFailureMessage({ kind: "rate-limited", retryAfterSeconds: 45 })).toBe(
      "Too many sign-in attempts from this network. Nothing is wrong with your details — try again in 45 seconds.",
    );
    expect(signInFailureMessage({ kind: "rate-limited", retryAfterSeconds: 45 }, { secondsLeft: 3 })).toMatch(/in 3 seconds\.$/);
    expect(signInFailureMessage({ kind: "rate-limited" })).toMatch(/in 1 minute\.$/);
  });

  it.each<[SignInFailure, RegExp]>([
    [{ kind: "locked" }, /locked after too many failed sign-in attempts/],
    [{ kind: "inactive" }, /deactivated/],
    [{ kind: "confirm" }, /hasn't been confirmed/],
    [{ kind: "two-factor" }, /two-step sign-in/],
    [{ kind: "change-required" }, /sign-in page/],
    [{ kind: "unreachable" }, /Can't reach the server/],
    [{ kind: "storage" }, /blocking site data/],
    [{ kind: "server", status: 503 }, /\(HTTP 503\)/],
    [{ kind: "server", detail: "Database offline" }, /\(Database offline\)/],
    [{ kind: "server" }, /couldn't sign you in\. Try again/],
  ])("%j → %s", (failure, pattern) => {
    expect(signInFailureMessage(failure)).toMatch(pattern);
  });
});

describe("small helpers", () => {
  it.each([
    [1, "1 second"],
    [45, "45 seconds"],
    [59.2, "1 minute"],
    [60, "1 minute"],
    [61, "2 minutes"],
    [0, "1 second"],
  ])("formatWait(%d) → %s", (seconds, text) => {
    expect(formatWait(seconds)).toBe(text);
  });

  it("failureField points at the field to fix", () => {
    expect(failureField({ kind: "unknown-workspace" })).toBe("workspace");
    expect(failureField({ kind: "invalid" })).toBe("credentials");
    expect(failureField({ kind: "locked" })).toBeNull();
  });

  it("tokenEndpoint names no workspace (accounts are global)", () => {
    expect(tokenEndpoint("https://api.test")).toBe("https://api.test/connect/token");
    expect(tokenEndpoint()).toBe(`${API_BASE_URL}/connect/token`);
  });
});

/* ----------------------------- Change at sign-in ----------------------------- */

function response(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name: string) => headers[name] ?? null },
    text: async () => text,
    json: async () => JSON.parse(text),
  } as unknown as Response;
}

const request = (fetch: (input: string, init: RequestInit) => Promise<Response>) => ({
  fetch,
  tokenUrl: "https://api.test/connect/token",
  clientId: WEB_SIGN_IN_CLIENT.clientId,
  scope: WEB_SIGN_IN_CLIENT.scope,
  username: "sam@realco.test",
  currentPassword: "OneTime#1",
  newPassword: "Mine#2026",
});

const challenge = response(400, { error: "invalid_grant", error_description: "ShouldChangePasswordOnNextLogin", userId: "u1", changePasswordToken: "cpt-1" });

describe("changePasswordAtSignIn — ABP's change-at-sign-in exchange", () => {
  it("asks for the one-time token, then sends it with the new password", async () => {
    const fetch = vi.fn().mockResolvedValueOnce(challenge).mockResolvedValueOnce(response(200, { access_token: "tok" }));
    expect(await changePasswordAtSignIn(request(fetch))).toEqual({ ok: true });

    expect(fetch).toHaveBeenCalledTimes(2);
    const [url, first] = fetch.mock.calls[0];
    expect(url).toBe("https://api.test/connect/token");
    const firstBody = new URLSearchParams(String(first.body));
    expect(Object.fromEntries(firstBody)).toEqual({
      grant_type: "password",
      username: "sam@realco.test",
      password: "OneTime#1",
      client_id: "Tracker_App",
      scope: "openid profile email roles offline_access Tracker",
    });
    const second = new URLSearchParams(String(fetch.mock.calls[1][1].body));
    expect(second.get("ChangePasswordToken")).toBe("cpt-1");
    expect(second.get("NewPassword")).toBe("Mine#2026");
    expect(second.get("password")).toBe("OneTime#1");
  });

  it("is done at once when the current password already signs in (changed meanwhile)", async () => {
    const fetch = vi.fn().mockResolvedValueOnce(response(200, { access_token: "tok" }));
    expect(await changePasswordAtSignIn(request(fetch))).toEqual({ ok: true });
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("shows the server's own words when it refuses the new password (its rules)", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(challenge)
      .mockResolvedValueOnce(response(400, { error: "invalid_grant", error_description: "Passwords must have at least one digit ('0'-'9')." }));
    const result = await changePasswordAtSignIn(request(fetch));
    expect(result).toEqual({ ok: false, failure: { kind: "invalid", status: 400 }, message: "Passwords must have at least one digit ('0'-'9')." });
  });

  it("reports a rejected current password, a rate limit, a server error and no network as such", async () => {
    const wrong = vi.fn().mockResolvedValueOnce(response(400, { error: "invalid_grant", error_description: "Invalid username or password!" }));
    expect(await changePasswordAtSignIn(request(wrong))).toEqual({ ok: false, failure: { kind: "invalid", status: 400 }, message: null });

    const limited = vi.fn().mockResolvedValueOnce(response(429, {}, { "Retry-After": "30" }));
    expect(await changePasswordAtSignIn(request(limited))).toEqual({ ok: false, failure: { kind: "rate-limited", retryAfterSeconds: 30 }, message: null });

    const down = vi.fn().mockResolvedValueOnce(challenge).mockResolvedValueOnce(response(503, { error_description: "maintenance" }));
    expect(await changePasswordAtSignIn(request(down))).toEqual({ ok: false, failure: { kind: "server", status: 503, detail: "maintenance" }, message: null });

    const offline = vi.fn().mockRejectedValueOnce(new TypeError("Failed to fetch"));
    expect(await changePasswordAtSignIn(request(offline))).toEqual({ ok: false, failure: { kind: "unreachable" }, message: null });

    const dropped = vi.fn().mockResolvedValueOnce(challenge).mockRejectedValueOnce(new TypeError("Failed to fetch"));
    expect(await changePasswordAtSignIn(request(dropped))).toEqual({ ok: false, failure: { kind: "unreachable" }, message: null });
  });
});

describe("workspace choices", () => {
  const a = { id: "11111111-1111-1111-1111-111111111111", name: "Acme", role: "owner" };
  const b = { id: "22222222-2222-2222-2222-222222222222", name: "Beta", role: "member" };
  const c = { id: "33333333-3333-3333-3333-333333333333", name: "Gamma", role: "member" };
  const aArchived = { ...a, isArchived: true };
  const bArchived = { ...b, isArchived: true };

  it("none → create, one → open it, several → the last used one while still a member, else choose", () => {
    expect(pickWorkspace([], null)).toEqual({ kind: "create" });
    expect(pickWorkspace([a], null)).toEqual({ kind: "select", id: a.id });
    expect(pickWorkspace([a], b.id)).toEqual({ kind: "select", id: a.id });
    expect(pickWorkspace([a, b], b.id.toUpperCase())).toEqual({ kind: "select", id: b.id });
    expect(pickWorkspace([a, b], "44444444-4444-4444-4444-444444444444")).toEqual({ kind: "choose" });
    expect(pickWorkspace([a, b], null)).toEqual({ kind: "choose" });
  });

  it("never auto-selects archived workspaces; all-archived leads to chooser (not create)", () => {
    // An account with only archived memberships must NOT be misclassified as no memberships (create)
    expect(pickWorkspace([aArchived], null)).toEqual({ kind: "choose" });
    expect(pickWorkspace([aArchived, bArchived], null)).toEqual({ kind: "choose" });

    // Active single workspace is selected even if last-used was an archived one
    expect(pickWorkspace([aArchived, b], aArchived.id)).toEqual({ kind: "select", id: b.id });
    expect(pickWorkspace([aArchived, b], null)).toEqual({ kind: "select", id: b.id });

    // When multiple active exist, last-used to an archived workspace does not select it
    expect(pickWorkspace([aArchived, b, c], aArchived.id)).toEqual({ kind: "choose" });
    expect(pickWorkspace([aArchived, b, c], b.id)).toEqual({ kind: "select", id: b.id });
  });

  it("identifies owners by role string or isOwner flag", () => {
    expect(isWorkspaceOwner(a)).toBe(true);
    expect(isWorkspaceOwner({ id: "x", name: "X", role: "OWNER" })).toBe(true);
    expect(isWorkspaceOwner({ id: "x", name: "X", role: "member", isOwner: true })).toBe(true);
    expect(isWorkspaceOwner(b)).toBe(false);
    expect(isWorkspaceOwner({ id: "x", name: "X", role: "manager" })).toBe(false);
    expect(isWorkspaceOwner({ id: "x", name: "X", role: null })).toBe(false);
  });

  it("labels roles and initials", () => {
    expect(workspaceRoleLabel("owner")).toBe("Owner");
    expect(workspaceRoleLabel("MANAGER")).toBe("Manager");
    expect(workspaceRoleLabel(null)).toBe("Member");
    expect(workspaceInitial(a)).toBe("A");
  });
});
