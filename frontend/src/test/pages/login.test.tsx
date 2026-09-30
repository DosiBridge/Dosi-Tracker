// LoginPage behavior: the real OpenIddict password flow from the user's side.
// Only the network boundary (global fetch) is stubbed — the page, loginApi,
// registerWorkspaceApi, localStorage/cookie session mirror, the SessionProvider
// (which now loads the real identity BEFORE the page navigates) and the router
// stub are all exercised for real.
//
// KNOWN GAP (reported, not asserted): none of the field labels on this page are
// programmatically associated with their inputs (no htmlFor/id), so screen
// readers announce bare inputs and getByLabelText cannot find them. Tests below
// query by placeholder/type instead of by label because of this.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import LoginPage from "@/app/login/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";
import { __router } from "@/test/next-navigation-stub";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";

type UserSession = ReturnType<typeof userEvent.setup>;

function jsonResponse(body: unknown, init: { status?: number; statusText?: string } = {}): Response {
  const { status = 200, statusText = "OK" } = init;
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
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
