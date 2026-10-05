import { describe, expect, it, vi } from "vitest";
import {
  API_BASE_URL,
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
    expect(signInFailureMessage({ kind: "invalid" })).toBe("That email, password or workspace didn't match. Check them and try again.");
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

  it("tokenEndpoint scopes a workspace account with __tenant (a query parameter, as ABP needs)", () => {
    expect(tokenEndpoint("Acme Corp", "https://api.test")).toBe("https://api.test/connect/token?__tenant=Acme%20Corp");
    expect(tokenEndpoint(null, "https://api.test")).toBe("https://api.test/connect/token");
    expect(tokenEndpoint(undefined)).toBe(`${API_BASE_URL}/connect/token`);
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
  tokenUrl: "https://api.test/connect/token?__tenant=acme",
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
    expect(url).toBe("https://api.test/connect/token?__tenant=acme");
    const firstBody = new URLSearchParams(String(first.body));
    expect(Object.fromEntries(firstBody)).toEqual({
      grant_type: "password",
      username: "sam@realco.test",
      password: "OneTime#1",
      client_id: "Tracker_App",
      scope: "Tracker offline_access",
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
