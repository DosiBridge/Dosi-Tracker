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

/** ABP application-configuration for a signed-in tenant owner (or the host when tenantId is null). */
function appConfig(opts: { tenantId?: string | null; tenantName?: string | null; roles?: string[] } = {}) {
  const { tenantId = "tenant-1", tenantName = "acme", roles = ["admin"] } = opts;
  return {
    currentUser: {
      isAuthenticated: true,
      id: "user-1",
      tenantId,
      userName: "ayesha",
      name: "Ayesha",
      surName: "Rahman",
      email: "ayesha@dosi.dev",
      roles,
    },
    currentTenant: tenantId ? { id: tenantId, name: tenantName, isAvailable: true } : { id: null, name: null, isAvailable: false },
    auth: { grantedPolicies: {} },
  };
}

/**
 * A backend that accepts the token request and then serves the session
 * load that follows it: identity (tenant or host) + empty tenant lists.
 */
function stubBackend(token: string, config = appConfig()): void {
  fetchMock.mockImplementation(async (input) => {
    const url = String(input);
    if (url.includes("/connect/token")) return jsonResponse({ access_token: token, expires_in: 3600 });
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

const workspaceField = () => screen.getByPlaceholderText(/your workspace name/i);
const signInButton = () => screen.getByRole("button", { name: /^sign in$/i });

async function fillSignIn(
  user: UserSession,
  opts: { workspace?: string; email?: string; password?: string } = {},
): Promise<void> {
  const { workspace = "", email = "ayesha@dosi.dev", password = "hunter22" } = opts;
  if (workspace) await user.type(workspaceField(), workspace);
  await user.type(field("email"), email);
  await user.type(field("password"), password);
}

describe("login page — sign-in form", () => {
  it("renders workspace, email and password fields with visible labels and a submit button", () => {
    renderWithProviders(<LoginPage />, { route: "/login" });

    expect(screen.getByText("Workspace")).toBeInTheDocument();
    expect(screen.getByText("Email")).toBeInTheDocument();
    expect(screen.getByText("Password")).toBeInTheDocument();
    expect(workspaceField()).toBeInTheDocument();
    expect(field("email")).toBeRequired();
    expect(field("password")).toBeRequired();
    expect(signInButton()).toBeEnabled();
  });

  it("tenant sign-in sends __tenant on /connect/token, stores the token + cookie flag, and lands on /dashboard", async () => {
    stubBackend("tok-123");
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme" });
    await user.click(signInButton());

    await waitFor(() => expect(localStorage.getItem("dosi-token")).toBe("tok-123"));
    expect(tokenCalls()).toHaveLength(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(String(url)).toBe(`${API_BASE}/connect/token?__tenant=acme`);
    const body = new URLSearchParams(String(init?.body));
    expect(body.get("grant_type")).toBe("password");
    expect(body.get("username")).toBe("ayesha@dosi.dev");
    expect(body.get("password")).toBe("hunter22");
    expect(localStorage.getItem("dosi-tenant")).toBe("acme");
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

  it("host sign-in (empty workspace) omits __tenant and lands on the platform console", async () => {
    stubBackend("tok-host", appConfig({ tenantId: null, tenantName: null }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { email: "ops@dositracker.app" });
    await user.click(signInButton());

    await waitFor(() => expect(tokenCalls()).toHaveLength(1));
    expect(String(fetchMock.mock.calls[0][0])).toBe(`${API_BASE}/connect/token`);
    expect(localStorage.getItem("dosi-tenant")).toBeNull();
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
    expect(await screen.findByText(/that email, password or workspace didn't match/i)).toBeInTheDocument();
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

    await user.type(workspaceField(), "acme");
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

  it("previews the exact workspace name to sign in with — not an invented subdomain", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");

    expect(screen.getByText(/sign in with the workspace name/i)).toHaveTextContent("Acme Corp");
    expect(document.body.textContent).not.toContain(".dositracker.app");
  });

  it("registers the tenant, signs into it, shows the exact workspace name to use, then lands on /dashboard", async () => {
    fetchMock.mockImplementation(async (input) => {
      const url = String(input);
      if (url.includes("/api/app/workspace/register")) {
        return jsonResponse({ tenantId: "t-1", name: "Acme Corp" });
      }
      if (url.includes("/connect/token")) {
        return jsonResponse({ access_token: "tok-signup", expires_in: 60 });
      }
      if (url.includes("/api/abp/application-configuration")) {
        return jsonResponse(appConfig({ tenantId: "t-1", tenantName: "Acme Corp" }));
      }
      if (url.includes("/api/app/workspace/current-subscription")) return jsonResponse({});
      return jsonResponse({ items: [] });
    });
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    // The success screen names the workspace exactly as it must be typed at sign-in.
    expect(await screen.findByTestId("created-workspace-name")).toHaveTextContent("Acme Corp");
    const [regUrl, regInit] = fetchMock.mock.calls[0];
    expect(String(regUrl)).toBe(`${API_BASE}/api/app/workspace/register`);
    expect(JSON.parse(String(regInit?.body))).toEqual({
      name: "Acme Corp",
      adminEmail: "founder@acme.dev",
      adminPassword: "hunter22",
      planName: "Free", // default plan selection maps to the seeded backend plan name
    });
    // The follow-up login is scoped to the tenant the backend just created.
    expect(String(fetchMock.mock.calls[1][0])).toBe(
      `${API_BASE}/connect/token?__tenant=${encodeURIComponent("Acme Corp")}`,
    );
    expect(__router.push).not.toHaveBeenCalled(); // the owner reads the name first
    await user.click(screen.getByRole("button", { name: /continue to your dashboard/i }));
    expect(__router.push).toHaveBeenCalledWith("/dashboard");
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

  it("an unknown workspace names it, marks the field and puts the cursor there", async () => {
    await signInWith(jsonResponse("<html>Tenant not found!</html>", { status: 404, statusText: "Not Found" }), { workspace: "acmee" });

    expect(await screen.findByRole("alert")).toHaveTextContent(/couldn't find an active workspace named “acmee”/i);
    const ws = workspaceField();
    expect(ws).toHaveAttribute("aria-invalid", "true");
    expect(ws).toHaveFocus();
    expect(ws.getAttribute("aria-describedby")).toContain("login-error");
  });

  it("wrong credentials are announced as an alert and mark the email and password fields", async () => {
    await signInWith(jsonResponse({ error: "invalid_grant", error_description: "Invalid username or password!" }, { status: 400, statusText: "Bad Request" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/that email, password or workspace didn't match/i);
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
      if (url.includes("/api/app/workspace/register")) return jsonResponse({ tenantId: "t-1", name: "Acme Corp" });
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
    expect(workspaceField()).toHaveValue("Acme Corp");
    expect(field("email")).toHaveValue("founder@acme.dev");
    expect(screen.getByRole("status")).toHaveTextContent(/your workspace “Acme Corp” was created\. sign in to continue/i);
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
    expect(String(change?.[0])).toContain("__tenant=acme");
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
