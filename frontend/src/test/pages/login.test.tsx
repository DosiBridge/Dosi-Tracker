// LoginPage behavior: the real OpenIddict password flow from the user's side.
// Only the network boundary (global fetch) is stubbed — the page, loginApi,
// registerWorkspaceApi, localStorage/cookie session mirror, the SessionProvider
// (which now loads the real identity BEFORE the page navigates) and the router
// stub are all exercised for real.
//
// The fields carry real labels (htmlFor/id); the older tests below still find
// them by placeholder/type, the newer ones by label.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import LoginPage from "@/app/login/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";
import { __router } from "@/test/next-navigation-stub";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";

type UserSession = ReturnType<typeof userEvent.setup>;

function jsonResponse(body: unknown, init: { status?: number; statusText?: string; headers?: Record<string, string> } = {}): Response {
  const { status = 200, statusText = "OK", headers = {} } = init;
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    headers: { get: (name: string) => headers[name] ?? null },
    json: async () => JSON.parse(text),
    text: async () => text,
  } as unknown as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

const ACME = "11111111-1111-1111-1111-111111111111";
const BETA = "22222222-2222-2222-2222-222222222222";
const ws = (id: string, name: string, role = "owner") => ({ id, name, role, planName: "Free", isOwner: role === "owner" });

/** ABP application-configuration: a global account (no tenant); `roles` carries platform-admin for the operator. */
function appConfig(opts: { roles?: string[]; policies?: Record<string, boolean> } = {}) {
  const { roles = [], policies = {} } = opts;
  return {
    currentUser: {
      isAuthenticated: true,
      id: "user-1",
      tenantId: null,
      userName: "ayesha",
      name: "Ayesha",
      surName: "Rahman",
      email: "ayesha@dosi.dev",
      roles,
    },
    currentTenant: { id: null, name: null, isAvailable: false },
    auth: { grantedPolicies: policies },
  };
}

/**
 * A backend that accepts the token request and then serves what follows it:
 * the account's workspaces, its identity and empty workspace lists.
 */
function stubBackend(token: string, opts: { mine?: unknown[]; config?: unknown } = {}): void {
  const { mine = [ws(ACME, "Acme")], config = appConfig() } = opts;
  fetchMock.mockImplementation(async (input) => {
    const url = String(input);
    if (url.includes("/connect/token")) return jsonResponse({ access_token: token, expires_in: 3600 });
    if (url.includes("/api/app/workspaces/mine")) return jsonResponse(mine);
    if (url.includes("/api/abp/application-configuration")) return jsonResponse(config);
    if (url.includes("/api/app/workspace/current-subscription")) return jsonResponse({});
    return jsonResponse({ items: [] });
  });
}

const tokenCalls = () => fetchMock.mock.calls.filter(([u]) => String(u).includes("/connect/token"));

beforeEach(() => {
  resetPrototypeState();
  // The harness clears localStorage but not cookies; expire the auth flag so
  // one test's successful login can never satisfy another's assertion.
  document.cookie = "dosi-token=; Path=/; Max-Age=0";
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

/** The page's labels are not wired to inputs, so locate fields by type. */
function field(type: "email" | "password"): HTMLInputElement {
  const el = document.querySelector(`input[type="${type}"]`);
  if (!(el instanceof HTMLInputElement)) throw new Error(`no ${type} input rendered`);
  return el;
}

const signInButton = () => screen.getByRole("button", { name: /^sign in$/i });

async function fillSignIn(
  user: UserSession,
  opts: { workspace?: string; email?: string; password?: string } = {},
): Promise<void> {
  const { email = "ayesha@dosi.dev", password = "hunter22" } = opts;
  await user.type(field("email"), email);
  await user.type(field("password"), password);
}

describe("login page — sign-in form", () => {
  it("asks only for email and password — no workspace name", () => {
    renderWithProviders(<LoginPage />, { route: "/login" });

    expect(screen.queryByText("Workspace")).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/workspace name/i)).not.toBeInTheDocument();
    expect(screen.getByText("Email")).toBeInTheDocument();
    expect(screen.getByText("Password")).toBeInTheDocument();
    expect(field("email")).toBeRequired();
    expect(field("password")).toBeRequired();
    expect(signInButton()).toBeEnabled();
  });

  it("signs in with no workspace named, opens the only workspace, stores the session and lands on /dashboard", async () => {
    stubBackend("tok-123");
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user);
    await user.click(signInButton());

    await waitFor(() => expect(localStorage.getItem("dosi-token")).toBe("tok-123"));
    expect(tokenCalls()).toHaveLength(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(String(url)).toBe(`${API_BASE}/connect/token`);
    const body = new URLSearchParams(String(init?.body));
    expect(body.get("grant_type")).toBe("password");
    expect(body.get("username")).toBe("ayesha@dosi.dev");
    expect(body.get("password")).toBe("hunter22");
    expect(body.get("scope")).toBe("openid profile email roles offline_access Tracker");
    expect(body.has("__tenant")).toBe(false);
    await waitFor(() => expect(localStorage.getItem("dosi-workspace")).toBe(ACME));
    expect(document.cookie).toContain("dosi-token=1");
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
  });

  it("loads the real identity before navigating, so the first page shows the signed-in account", async () => {
    stubBackend("tok-123");
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme" });
    await user.click(signInButton());

    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
    const urls = fetchMock.mock.calls.map(([u]) => String(u));
    const configAt = urls.findIndex((u) => u.includes("/api/abp/application-configuration"));
    expect(configAt).toBeGreaterThan(0); // after the token request, before navigation resolved
  });

  it("the platform admin (global platform-admin role) lands on the platform console with no workspace", async () => {
    stubBackend("tok-host", { mine: [], config: appConfig({ roles: ["platform-admin"], policies: { "Tracker.Platform": true } }) });
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { email: "ops@dositracker.app" });
    await user.click(signInButton());

    await waitFor(() => expect(tokenCalls()).toHaveLength(1));
    expect(String(fetchMock.mock.calls[0][0])).toBe(`${API_BASE}/connect/token`);
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/host"), { timeout: 2000 });
  });

  it("a rejected login shows a human error message and re-enables the form instead of navigating", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ error: "invalid_grant" }, { status: 400, statusText: "Bad Request" }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme", password: "wrong-pass" });
    await user.click(signInButton());

    // Names the fields the form actually has (email, not "username") and
    // blames the credentials only because the server really did reject them.
    expect(await screen.findByText(/that email or password didn't match/i)).toBeInTheDocument();
    expect(signInButton()).toBeEnabled(); // no longer stuck in "Signing in…"
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(document.cookie).not.toContain("dosi-token=1");
    expect(__router.push).not.toHaveBeenCalled();
  });

  it("says the server is unreachable rather than blaming the password when the request never lands", async () => {
    // A network failure previously produced "Invalid username, password or
    // workspace", sending users to re-check credentials that were fine.
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme", password: "correct-horse" });
    await user.click(signInButton());

    expect(await screen.findByText(/can't reach the server/i)).toBeInTheDocument();
    expect(screen.queryByText(/didn't match/i)).not.toBeInTheDocument();
    expect(signInButton()).toBeEnabled();
    expect(__router.push).not.toHaveBeenCalled();
  });

  it("pressing Enter in the password field submits the form", async () => {
    stubBackend("tok-kbd");
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.type(field("email"), "ayesha@dosi.dev");
    await user.type(field("password"), "hunter22{Enter}");

    await waitFor(() => expect(tokenCalls()).toHaveLength(1));
    expect(String(fetchMock.mock.calls[0][0])).toContain("/connect/token");
  });

  it("switching to signup and back clears a stale error", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ error: "invalid_grant" }, { status: 400, statusText: "Bad Request" }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme", password: "wrong-pass" });
    await user.click(signInButton());
    expect(await screen.findByText(/didn't match/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    expect(screen.queryByText(/didn't match/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /back to sign in/i }));
    expect(screen.queryByText(/didn't match/i)).not.toBeInTheDocument();
  });

  it("'Forgot password?' explains who can reset it (owner, or support for owners) without calling it an error", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /forgot password/i }));
    const note = screen.getByRole("status");
    expect(note).toHaveTextContent(/contact your workspace owner/i);
    expect(note).toHaveTextContent(/support@/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("the workspace field hint is neutral — no host-only instructions", () => {
    renderWithProviders(<LoginPage />, { route: "/login" });
    expect(screen.queryByText(/host sign-in/i)).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/host/i)).not.toBeInTheDocument();
  });

  it("rapid duplicate submits fire only one token request (button disables while in flight)", async () => {
    // Never-settling response keeps the request in flight for the whole test.
    fetchMock.mockImplementation(() => new Promise<Response>(() => {}));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme" });
    const button = signInButton();
    await user.click(button);
    expect(button).toBeDisabled();
    await user.click(button); // second click lands on a disabled button
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe("login page — create-workspace mode", () => {
  it("refuses to submit until email, password and workspace name are all provided", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    expect(screen.getByRole("heading", { name: /create your workspace/i })).toBeInTheDocument();

    const submit = screen.getByRole("button", { name: /^create workspace$/i });
    expect(submit).toBeDisabled();

    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    expect(submit).toBeDisabled(); // email + password still missing
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("does not invent a subdomain or ask people to remember the workspace name", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");

    expect(screen.queryByText(/sign in with the workspace name/i)).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain(".dositracker.app");
  });

  it("?mode=signup opens the create-workspace form with the email filled in", async () => {
    window.history.pushState({}, "", "/login?mode=signup&email=a%40b.dev");
    try {
      renderWithProviders(<LoginPage />, { route: "/login" });
      expect(await screen.findByRole("heading", { name: /create your workspace/i })).toBeInTheDocument();
      expect(field("email")).toHaveValue("a@b.dev");
    } finally {
      window.history.pushState({}, "", "/");
    }
  });

  it("creates the account and workspace, signs in automatically and opens the new workspace", async () => {
    stubBackend("tok-signup", { mine: [ws(ACME, "Acme Corp")] });
    const base = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) =>
      String(input).includes("/api/app/workspace/register")
        ? jsonResponse({ workspaceId: ACME, workspaceName: "Acme Corp", userName: "founder" })
        : base(input, init),
    );
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(screen.getByLabelText("First name"), "Fara");
    await user.type(screen.getByLabelText("Surname"), "Haque");
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
    const [regUrl, regInit] = fetchMock.mock.calls[0];
    expect(String(regUrl)).toBe(`${API_BASE}/api/app/workspace/register`);
    expect(JSON.parse(String(regInit?.body))).toEqual({
      firstName: "Fara",
      surname: "Haque",
      email: "founder@acme.dev",
      password: "hunter22",
      workspaceName: "Acme Corp",
      planName: "Free",
    });
    expect(String(fetchMock.mock.calls[1][0])).toBe(`${API_BASE}/connect/token`);
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("an email that already has an account says to sign in and create the workspace from the sidebar", async () => {
    fetchMock.mockResolvedValue(
      jsonResponse({ error: { code: "Tracker:EmailAlreadyRegistered", message: "taken" } }, { status: 409, statusText: "Conflict" }),
    );
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    expect(await screen.findByRole("heading", { name: /welcome back/i })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "You already have an account — sign in and create a new workspace from the sidebar.",
    );
    expect(tokenCalls()).toHaveLength(0);
  });

  it("surfaces the backend's registration error message instead of a blank screen", async () => {
    fetchMock.mockResolvedValue(
      jsonResponse(
        { error: { message: "Workspace name is already taken." } },
        { status: 400, statusText: "Bad Request" },
      ),
    );
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    expect(await screen.findByText("Workspace name is already taken.")).toBeInTheDocument();
    expect(__router.push).not.toHaveBeenCalled();
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });
});

describe("login page — every way a sign-in fails, told apart", () => {
  async function signInWith(response: Response | Error, opts: { workspace?: string } = {}) {
    if (response instanceof Error) fetchMock.mockRejectedValue(response);
    else fetchMock.mockResolvedValue(response);
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });
    await fillSignIn(user, { workspace: opts.workspace ?? "acme" });
    await user.click(signInButton());
    return user;
  }

  it("a rate limit says how long to wait — not that the password is wrong — and holds the button until then", async () => {
    await signInWith(jsonResponse({}, { status: 429, statusText: "Too Many Requests", headers: { "Retry-After": "30" } }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/too many sign-in attempts/i);
    expect(alert).toHaveTextContent(/try again in 30 seconds/i);
    expect(alert).not.toHaveTextContent(/didn't match/i);
    const button = await screen.findByRole("button", { name: /try again in 0:[0-3]\d/i });
    expect(button).toBeDisabled();
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("a rate limit without Retry-After waits out the server's one-minute window", async () => {
    await signInWith(jsonResponse({}, { status: 429, statusText: "Too Many Requests" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/try again in 1 minute/i);
  });

  it("the sign-in comes back once the wait is over", async () => {
    await signInWith(jsonResponse({}, { status: 429, statusText: "Too Many Requests", headers: { "Retry-After": "1" } }));
    expect(await screen.findByRole("button", { name: /try again in/i })).toBeDisabled();
    await waitFor(() => expect(signInButton()).toBeEnabled(), { timeout: 3000 });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("a locked account says so", async () => {
    await signInWith(
      jsonResponse(
        { error: "account_locked", error_description: "The user account has been locked out due to invalid login attempts. Please wait a while and try again." },
        { status: 400, statusText: "Bad Request" },
      ),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(/locked after too many failed sign-in attempts/i);
  });

  it("wrong credentials are announced as an alert and mark the email and password fields", async () => {
    await signInWith(jsonResponse({ error: "invalid_grant", error_description: "Invalid username or password!" }, { status: 400, statusText: "Bad Request" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/that email or password didn't match/i);
    expect(screen.getByLabelText("Email")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByLabelText("Password")).toHaveAttribute("aria-invalid", "true");
  });

  it("a server failure is the server's, with what it said", async () => {
    await signInWith(jsonResponse({ error: "server_error", error_description: "Database offline" }, { status: 503, statusText: "Service Unavailable" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/server couldn't sign you in/i);
    expect(alert).toHaveTextContent(/Database offline/);
    expect(alert).not.toHaveTextContent(/didn't match/i);
  });

  it("a deactivated account says so", async () => {
    await signInWith(jsonResponse({ error: "account_inactive", error_description: "You are not allowed to login!" }, { status: 400, statusText: "Bad Request" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/deactivated/i);
  });
});

describe("login page — blocked site data", () => {
  it("warns up front and keeps the form from pretending it can sign in", () => {
    // Every write is refused — the theme's included: nothing around the page may crash on it either.
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    });
    renderWithProviders(<LoginPage />, { route: "/login" });

    expect(screen.getByRole("alert")).toHaveTextContent(/blocking site data/i);
    expect(signInButton()).toBeDisabled();
  });
});

describe("login page — a new workspace whose first sign-in fails", () => {
  it("keeps the workspace it just created: switches to sign-in, filled in, and says so", async () => {
    fetchMock.mockImplementation(async (input) => {
      const url = String(input);
      if (url.includes("/api/app/workspace/register")) return jsonResponse({ workspaceId: ACME, workspaceName: "Acme Corp", userName: "f" });
      if (url.includes("/connect/token")) return jsonResponse({}, { status: 429, statusText: "Too Many Requests", headers: { "Retry-After": "20" } });
      return jsonResponse({ items: [] });
    });
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    expect(await screen.findByRole("heading", { name: /welcome back/i })).toBeInTheDocument();
    expect(field("email")).toHaveValue("founder@acme.dev");
    expect(screen.getByRole("status")).toHaveTextContent(/your account was created\. sign in to continue/i);
    expect(screen.getByRole("alert")).toHaveTextContent(/too many sign-in attempts/i);
    expect(fetchMock.mock.calls.filter(([u]) => String(u).includes("/api/app/workspace/register"))).toHaveLength(1);
  });
});

describe("login page — a one-time password must be changed first", () => {
  const challenge = () =>
    jsonResponse(
      { error: "invalid_grant", error_description: "ShouldChangePasswordOnNextLogin", userId: "u1", changePasswordToken: "cpt-1" },
      { status: 400, statusText: "Bad Request" },
    );

  async function reachChangeStep() {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });
    await fillSignIn(user, { workspace: "acme", password: "OneTime#1" });
    await user.click(signInButton());
    expect(await screen.findByRole("heading", { name: /choose a new password/i })).toBeInTheDocument();
    return user;
  }

  it("asks for a new password, changes it with the server, then signs in with it", async () => {
    let tokenCalls = 0;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      if (url.includes("/connect/token")) {
        tokenCalls++;
        const body = new URLSearchParams(String(init?.body));
        if (body.get("NewPassword")) return jsonResponse({ access_token: "ignored", expires_in: 3600 });
        if (body.get("password") === "Mine#2026") return jsonResponse({ access_token: "tok-new", expires_in: 3600 });
        return challenge();
      }
      if (url.includes("/api/app/workspaces/mine")) return jsonResponse([ws(ACME, "Acme")]);
      if (url.includes("/api/abp/application-configuration")) return jsonResponse(appConfig());
      if (url.includes("/api/app/workspace/current-subscription")) return jsonResponse({});
      return jsonResponse({ items: [] });
    });
    const user = await reachChangeStep();

    await user.type(screen.getByLabelText("New password"), "Mine#2026");
    await user.type(screen.getByLabelText("Confirm new password"), "Mine#2026");
    await user.click(screen.getByRole("button", { name: /save and sign in/i }));

    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-token")).toBe("tok-new");
    // login (challenge) → challenge again for its token → the change → sign in with the new password
    expect(tokenCalls).toBe(4);
    const change = fetchMock.mock.calls.find(([, i]) => String(i?.body).includes("NewPassword"));
    expect(String(change?.[0])).toBe(`${API_BASE}/connect/token`);
    const body = new URLSearchParams(String(change?.[1]?.body));
    expect(body.get("ChangePasswordToken")).toBe("cpt-1");
    expect(body.get("password")).toBe("OneTime#1");
  });

  it("checks the two entries match and are long enough before asking the server", async () => {
    fetchMock.mockResolvedValue(challenge());
    const user = await reachChangeStep();
    const calls = fetchMock.mock.calls.length;

    await user.type(screen.getByLabelText("New password"), "abc");
    await user.type(screen.getByLabelText("Confirm new password"), "abc");
    await user.click(screen.getByRole("button", { name: /save and sign in/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/at least 6 characters/i);

    await user.clear(screen.getByLabelText("New password"));
    await user.type(screen.getByLabelText("New password"), "abcdef1");
    await user.click(screen.getByRole("button", { name: /save and sign in/i }));
    expect(screen.getByRole("alert")).toHaveTextContent(/don't match/i);
    expect(fetchMock.mock.calls.length).toBe(calls);
  });

  it("shows the server's own rules when it refuses the new password", async () => {
    fetchMock.mockImplementation(async (_input, init) => {
      const body = new URLSearchParams(String(init?.body));
      if (body.get("NewPassword")) {
        return jsonResponse({ error: "invalid_grant", error_description: "Passwords must have at least one digit ('0'-'9')." }, { status: 400, statusText: "Bad Request" });
      }
      return challenge();
    });
    const user = await reachChangeStep();
    await user.type(screen.getByLabelText("New password"), "NoDigits#");
    await user.type(screen.getByLabelText("Confirm new password"), "NoDigits#");
    await user.click(screen.getByRole("button", { name: /save and sign in/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Passwords must have at least one digit ('0'-'9').");
    expect(__router.push).not.toHaveBeenCalled();
  });

  it("'Back to sign in' leaves the step", async () => {
    fetchMock.mockResolvedValue(challenge());
    const user = await reachChangeStep();
    await user.click(screen.getByRole("button", { name: /back to sign in/i }));
    const form = screen.getByRole("heading", { name: /welcome back/i }).parentElement as HTMLElement;
    expect(within(form).getByRole("button", { name: /^sign in$/i })).toBeInTheDocument();
  });
});

describe("login page — which workspace opens after sign-in", () => {
  async function signIn() {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });
    await fillSignIn(user);
    await user.click(signInButton());
    return user;
  }

  it("no workspace yet: goes to the chooser to create the first one", async () => {
    stubBackend("tok-0", { mine: [] });
    await signIn();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/workspaces"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("several workspaces: reopens the last used one while still a member", async () => {
    localStorage.setItem("dosi-workspace", BETA);
    stubBackend("tok-2", { mine: [ws(ACME, "Acme"), ws(BETA, "Beta", "member")] });
    await signIn();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
  });

  it("several workspaces and no (valid) last one: goes to the chooser", async () => {
    localStorage.setItem("dosi-workspace", "33333333-3333-3333-3333-333333333333");
    stubBackend("tok-2", { mine: [ws(ACME, "Acme"), ws(BETA, "Beta", "member")] });
    await signIn();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/workspaces"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("when all workspaces are archived, goes to the chooser and does not auto-select", async () => {
    stubBackend("tok-arch", { mine: [{ ...ws(ACME, "Acme"), isArchived: true }] });
    await signIn();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/workspaces"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("never auto-selects archived workspace even if last-used was that workspace", async () => {
    localStorage.setItem("dosi-workspace", ACME);
    stubBackend("tok-arch2", {
      mine: [{ ...ws(ACME, "Acme"), isArchived: true }, ws(BETA, "Beta", "member")],
    });
    await signIn();
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
  });

  it("?next= (an invitation) is where a sign-in returns to", async () => {
    window.history.pushState({}, "", "/login?next=%2Finvite%2Ftok-1");
    try {
      stubBackend("tok-n");
      await signIn();
      await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/invite/tok-1"), { timeout: 2000 });
    } finally {
      window.history.pushState({}, "", "/");
    }
  });
});
