// LoginPage behavior: the real OpenIddict password flow from the user's side.
// Only the network boundary (global fetch) is stubbed — the page, loginApi,
// registerWorkspaceApi, localStorage/cookie session mirror and the router stub
// are all exercised for real.
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

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:44342";

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

const workspaceField = () => screen.getByPlaceholderText(/leave empty for host sign-in/i);
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
    fetchMock.mockResolvedValue(jsonResponse({ access_token: "tok-123", expires_in: 3600 }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { workspace: "acme" });
    await user.click(signInButton());

    await waitFor(() => expect(localStorage.getItem("dosi-token")).toBe("tok-123"));
    expect(fetchMock).toHaveBeenCalledTimes(1);
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

  it("host sign-in (empty workspace) omits __tenant and lands on the platform console", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ access_token: "tok-host", expires_in: 3600 }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await fillSignIn(user, { email: "ops@dositracker.app" });
    await user.click(signInButton());

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
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

    expect(await screen.findByText("Invalid username, password or workspace")).toBeInTheDocument();
    expect(signInButton()).toBeEnabled(); // no longer stuck in "Signing in…"
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(document.cookie).not.toContain("dosi-token=1");
    expect(__router.push).not.toHaveBeenCalled();
  });

  it("pressing Enter in the password field submits the form", async () => {
    fetchMock.mockResolvedValue(jsonResponse({ access_token: "tok-kbd", expires_in: 60 }));
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.type(workspaceField(), "acme");
    await user.type(field("email"), "ayesha@dosi.dev");
    await user.type(field("password"), "hunter22{Enter}");

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(String(fetchMock.mock.calls[0][0])).toContain("/connect/token");
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

  it("registers the tenant, then signs into it and lands on /dashboard", async () => {
    fetchMock.mockImplementation(async (input) => {
      const url = String(input);
      if (url.includes("/api/app/workspace/register")) {
        return jsonResponse({ tenantId: "t-1", name: "Acme Corp" });
      }
      if (url.includes("/connect/token")) {
        return jsonResponse({ access_token: "tok-signup", expires_in: 60 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    const user = userEvent.setup();
    renderWithProviders(<LoginPage />, { route: "/login" });

    await user.click(screen.getByRole("button", { name: /create a workspace/i }));
    await user.type(field("email"), "founder@acme.dev");
    await user.type(field("password"), "hunter22");
    await user.type(screen.getByPlaceholderText("Acme Corp"), "Acme Corp");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
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
    await waitFor(() => expect(__router.push).toHaveBeenCalledWith("/dashboard"), { timeout: 2000 });
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
