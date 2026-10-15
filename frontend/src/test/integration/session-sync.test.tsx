// The live session keeps itself honest while a page stays open — through the
// REAL SessionProvider and SessionGate, with only global fetch stubbed:
//  - other tabs share the browser's session: a sign-out there signs this tab
//    out, a sign-in to another account or workspace reloads it, a renewal of
//    the same account changes nothing;
//  - the data refreshes silently (on an interval, on returning to the tab, on
//    request) by re-reading only the newest rows;
//  - blocked site data never crashes the provider;
//  - a session renews itself before its token runs out when a refresh token
//    allows, and otherwise warns, then asks for the password IN PLACE.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/lib/live-session", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/live-session")>()),
  redirectToLogin: vi.fn(),
  reloadPage: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { useEffect } from "react";
import { SessionProvider, useSession } from "@/components/session-provider";
import { SessionGate } from "@/components/session-gate";
import { SESSION_UNAUTHORIZED_EVENT, redirectToLogin, reloadPage } from "@/lib/live-session";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

let captured: ReturnType<typeof useSession> | null = null;

function sess(): ReturnType<typeof useSession> {
  if (!captured) throw new Error("Probe is not mounted");
  return captured;
}

function Probe() {
  const session = useSession();
  useEffect(() => {
    captured = session;
  });
  return (
    <div>
      <span data-testid="status">{session.status}</span>
      <span data-testid="user-name">{session.user.name}</span>
      <span data-testid="workspace-name">{session.workspace.name}</span>
      <span data-testid="expiry">{session.expiry.phase}</span>
      <span data-testid="coverage">{session.liveCoverage ? `${session.liveCoverage.loaded}/${session.liveCoverage.total}` : "none"}</span>
    </div>
  );
}

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: { get: () => null },
    json: async () => JSON.parse(text),
    text: async () => text,
  } as unknown as Response;
}

const API = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";
const OWNER = "11111111-1111-1111-1111-111111111111";
const OTHER = "99999999-9999-9999-9999-999999999999";

function config(userId = OWNER, tenantId = "t-1") {
  return {
    currentUser: { isAuthenticated: true, id: userId, tenantId, userName: "mira", name: "Mira", surName: "Okafor", email: "mira@realco.test", roles: ["admin"] },
    currentTenant: { id: tenantId, name: "RealCo", isAvailable: true },
    auth: { grantedPolicies: {} },
  };
}

interface Stub {
  identity: { userId: string; tenantId: string };
  tokenResponses: Response[];
  activities: unknown[];
}

/** A healthy backend; tests steer the identity, the token endpoint and the rows through `stub`. */
function liveBackend(): { fetch: ReturnType<typeof vi.fn>; stub: Stub } {
  const stub: Stub = { identity: { userId: OWNER, tenantId: "t-1" }, tokenResponses: [], activities: [] };
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input).replace(API, "");
    const path = url.split("?")[0];
    if (path === "/connect/token") return stub.tokenResponses.shift() ?? json({ error: "invalid_grant" }, 400);
    if (path === "/api/abp/application-configuration") return json(config(stub.identity.userId, stub.identity.tenantId));
    if (path === "/api/app/team/members") return json({ items: [{ userId: OWNER, userName: "mira", name: "Mira", surname: "Okafor", isOwner: true, projectIds: ["p-1"] }] });
    if (path === "/api/app/project") return json({ items: [{ id: "p-1", title: "Apollo" }] });
    if (path === "/api/app/activity") return json({ items: stub.activities, totalCount: stub.activities.length });
    if (path === "/api/app/workspace/current-subscription") return json({ planId: "plan-s", status: "active" });
    if (path === "/api/app/workspace/available-plans") return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
    void init;
    return json({ error: { code: null, message: "Not found" } }, 404);
  });
  vi.stubGlobal("fetch", fetch);
  return { fetch, stub };
}

const tokenResponse = (accessToken: string, refreshToken?: string) =>
  json({ access_token: accessToken, expires_in: 3600, ...(refreshToken ? { refresh_token: refreshToken } : {}) });

function signIn(token = "jwt-live") {
  localStorage.setItem("dosi-token", token);
  localStorage.setItem("dosi-workspace", "RealCo");
}

/** Another tab changing storage (the event never fires in the tab that wrote). */
function fromAnotherTab(key: string, oldValue: string | null, newValue: string | null) {
  if (newValue === null) localStorage.removeItem(key);
  else localStorage.setItem(key, newValue);
  act(() => {
    window.dispatchEvent(new StorageEvent("storage", { key, oldValue, newValue }));
  });
}

const expiresIn = (ms: number) => localStorage.setItem("dosi-token-expires-at", String(Date.now() + ms));

const calls = (fetch: ReturnType<typeof vi.fn>, path: string) => fetch.mock.calls.filter(([u]) => String(u).replace(API, "").startsWith(path));

beforeEach(() => {
  captured = null;
  resetPrototypeState();
  vi.mocked(redirectToLogin).mockClear();
  vi.mocked(reloadPage).mockClear();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("other tabs", () => {
  it("a sign-out in another tab signs this one out too — no stale workspace, no demo", async () => {
    liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    fromAnotherTab("dosi-token", "jwt-live", null);

    expect(screen.getByTestId("status")).toHaveTextContent("signedout");
    expect(screen.getByTestId("workspace-name")).not.toHaveTextContent("RealCo");
    expect(screen.getByTestId("user-name")).not.toHaveTextContent("Ayesha Rahman");
    expect(redirectToLogin).toHaveBeenCalledTimes(1);
  });

  it("switching workspace in another tab reloads this one", async () => {
    liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    fromAnotherTab("dosi-workspace", "RealCo", "OtherCo");

    expect(reloadPage).toHaveBeenCalledTimes(1);
  });

  it("a renewal of the same account changes nothing; another account's token reloads the tab", async () => {
    const { stub } = liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    fromAnotherTab("dosi-token", "jwt-live", "jwt-renewed");
    await act(async () => {});
    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("ok"));
    expect(reloadPage).not.toHaveBeenCalled();

    stub.identity = { userId: OTHER, tenantId: "t-1" };
    fromAnotherTab("dosi-token", "jwt-renewed", "jwt-someone-else");
    await waitFor(() => expect(reloadPage).toHaveBeenCalledTimes(1));
    expect(screen.getByTestId("user-name")).toHaveTextContent("Mira Okafor"); // never mixes the two accounts
  });

  it("a tab showing the demo reloads when another tab signs in", async () => {
    liveBackend();
    renderWithProviders(<Probe />);
    expect(screen.getByTestId("status")).toHaveTextContent("demo");

    fromAnotherTab("dosi-token", null, "jwt-live");

    expect(reloadPage).toHaveBeenCalledTimes(1);
  });

  it("coming back to a tab whose session ended elsewhere (event missed) signs it out", async () => {
    liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    localStorage.removeItem("dosi-token"); // no event reached this tab
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });

    expect(screen.getByTestId("status")).toHaveTextContent("signedout");
    expect(redirectToLogin).toHaveBeenCalled();
  });
});

describe("silent refresh", () => {
  const row = (id: string, startedAgoMs: number) => {
    const start = Date.now() - startedAgoMs;
    return { id, userId: OWNER, projectId: "p-1", startedAt: new Date(start).toISOString(), endedAt: new Date(start + 10 * 60_000).toISOString(), productivity: 60 };
  };

  it("re-reads only the newest rows, merges them, and never shows the loading screen", async () => {
    const { fetch, stub } = liveBackend();
    stub.activities = [row("old", 2 * 86_400_000)];
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("coverage")).toHaveTextContent("1/1"));

    stub.activities = [row("fresh", 60_000)];
    const before = calls(fetch, "/api/app/activity").length;
    let pulse: Promise<unknown> | null = null;
    act(() => {
      pulse = sess().refreshSession({ silent: true });
    });
    expect(screen.getByTestId("status")).toHaveTextContent("ready"); // no loading flash
    await act(async () => {
      await pulse;
    });

    const reads = calls(fetch, "/api/app/activity").slice(before);
    expect(reads).toHaveLength(1);
    const from = Date.parse(new URL(String(reads[0][0])).searchParams.get("From") ?? "");
    expect(Date.now() - from).toBeLessThanOrEqual(3 * 3_600_000 + 1000); // the last few hours, not 30 days
    expect(screen.getByTestId("coverage")).toHaveTextContent("2/2"); // "old" kept, "fresh" added
  });

  it("a refresh after the token vanished in this tab signs out — it never falls back to the demo", async () => {
    liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    localStorage.removeItem("dosi-token"); // e.g. the API layer ended the session on a refused token
    await act(async () => {
      await sess().refreshSession({ silent: true });
    });

    expect(screen.getByTestId("status")).toHaveTextContent("signedout");
    expect(screen.getByTestId("user-name")).not.toHaveTextContent("Ayesha Rahman");
    expect(redirectToLogin).toHaveBeenCalledTimes(1);
  });

  it("refreshes on an interval while the tab is visible", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const { fetch } = liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));
    const before = calls(fetch, "/api/abp/application-configuration").length;

    act(() => {
      vi.advanceTimersByTime(2 * 60_000);
    });

    await waitFor(() => expect(calls(fetch, "/api/abp/application-configuration").length).toBe(before + 1));
    expect(screen.getByTestId("status")).toHaveTextContent("ready");
  });
});

describe("blocked site data", () => {
  it("renders the demo without crashing, and switching people still works for this page view", () => {
    const denied = () => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    };
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(denied);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(denied);
    vi.spyOn(Storage.prototype, "removeItem").mockImplementation(denied);

    // The session provider alone: every storage call it makes goes through the safe wrappers.
    expect(() =>
      render(
        <SessionProvider>
          <Probe />
        </SessionProvider>,
      ),
    ).not.toThrow();
    expect(screen.getByTestId("status")).toHaveTextContent("demo");
    expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");

    act(() => sess().setUserById("u2"));
    expect(screen.getByTestId("user-name")).toHaveTextContent("Tanvir Hasan");
  });
});

describe("session expiry", () => {
  it("renews ahead of expiry with the refresh token, without asking anyone anything", async () => {
    const { fetch, stub } = liveBackend();
    signIn();
    localStorage.setItem("dosi-refresh-token", "rt-1");
    expiresIn(30_000); // inside the renewal lead
    stub.tokenResponses.push(tokenResponse("jwt-renewed", "rt-2"));

    renderWithProviders(<Probe />);

    await waitFor(() => expect(localStorage.getItem("dosi-token")).toBe("jwt-renewed"));
    const renewal = calls(fetch, "/connect/token").find(([, init]) => String(init?.body).includes("grant_type=refresh_token"));
    expect(renewal).toBeDefined();
    expect(String(renewal?.[0])).toBe(`${API}/connect/token`);
    expect(localStorage.getItem("dosi-refresh-token")).toBe("rt-2");
    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("ok"));
  });

  it("warns ahead of the end when the session can't renew itself, then calls it expired", async () => {
    liveBackend();
    signIn();
    expiresIn(3 * 60_000); // no refresh token: inside the warning lead
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("expiring"));

    expiresIn(-1000);
    act(() => {
      window.dispatchEvent(new StorageEvent("storage", { key: "dosi-token-expires-at", newValue: "x" }));
    });
    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("expired"));
    expect(redirectToLogin).not.toHaveBeenCalled(); // the page stays
  });

  it("a refused refresh token falls back to warning and asking", async () => {
    const { stub } = liveBackend();
    signIn();
    localStorage.setItem("dosi-refresh-token", "rt-revoked");
    expiresIn(30_000);
    stub.tokenResponses.push(json({ error: "invalid_grant" }, 400));

    renderWithProviders(<Probe />);

    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("expiring"));
    expect(localStorage.getItem("dosi-refresh-token")).toBeNull();
  });

  it("a refused request the API layer can't renew for asks for the password in place instead of leaving the page", async () => {
    liveBackend();
    signIn();
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));

    const event = new Event(SESSION_UNAUTHORIZED_EVENT, { cancelable: true });
    act(() => {
      window.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true); // handled here: the API layer keeps the page
    expect(screen.getByTestId("expiry")).toHaveTextContent("expired");
    expect(redirectToLogin).not.toHaveBeenCalled();
  });

  it("the unauthorized notice is left alone outside a loaded live session (the API layer signs out as before)", () => {
    renderWithProviders(<Probe />);
    const event = new Event(SESSION_UNAUTHORIZED_EVENT, { cancelable: true });
    act(() => {
      window.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(false);
  });

  it("reauthenticate signs the same account in again to the same workspace and clears the expiry", async () => {
    const { fetch, stub } = liveBackend();
    signIn();
    expiresIn(-1000);
    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("expiry")).toHaveTextContent("expired"));

    stub.tokenResponses.push(json({ error: "invalid_grant", error_description: "Invalid username or password!" }, 400));
    let result: Awaited<ReturnType<ReturnType<typeof useSession>["reauthenticate"]>> | null = null;
    await act(async () => {
      result = await sess().reauthenticate("wrong");
    });
    expect(result).toEqual({ ok: false, failure: { kind: "invalid", status: 400 } });

    stub.tokenResponses.push(tokenResponse("jwt-again"));
    await act(async () => {
      result = await sess().reauthenticate("right");
    });
    expect(result).toEqual({ ok: true });
    const attempt = calls(fetch, "/connect/token").at(-1);
    expect(String(attempt?.[0])).toBe(`${API}/connect/token`);
    const body = new URLSearchParams(String(attempt?.[1]?.body));
    expect(body.get("username")).toBe("mira");
    expect(body.get("password")).toBe("right");
    expect(localStorage.getItem("dosi-token")).toBe("jwt-again");
    await waitFor(() => expect(screen.getByTestId("expiry")).not.toHaveTextContent("expired"));
  });
});

describe("SessionGate", () => {
  it("shows a neutral screen after signing out — never the demo workspace", async () => {
    liveBackend();
    signIn();
    renderWithProviders(
      <SessionGate area="tenant">
        <Probe />
        <div>page body</div>
      </SessionGate>,
      { route: "/dashboard" },
    );
    expect(await screen.findByText("page body")).toBeInTheDocument();

    act(() => sess().logout());

    expect(screen.getByText(/signing you out/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /continue to sign in/i })).toHaveAttribute("href", "/login");
    expect(screen.queryByText("page body")).not.toBeInTheDocument();
    expect(screen.queryByText("Ayesha Rahman")).not.toBeInTheDocument();
  });

  it("when the session has run out, asks for the password in place — the page stays rendered", async () => {
    const { stub } = liveBackend();
    signIn();
    expiresIn(-1000);
    const user = userEvent.setup();
    renderWithProviders(
      <SessionGate area="tenant">
        <div>unsaved form</div>
      </SessionGate>,
      { route: "/dashboard" },
    );

    const dialog = await screen.findByRole("dialog", { name: /your session has ended/i });
    expect(screen.getByText("unsaved form")).toBeInTheDocument();
    expect(within(dialog).getByText("RealCo")).toBeInTheDocument();
    expect(within(dialog).getByText("mira")).toBeInTheDocument();
    // An expired session can't be waved away: no "Not now", only signing in or out.
    expect(within(dialog).queryByRole("button", { name: /not now/i })).not.toBeInTheDocument();
    expect(await axe(dialog)).toHaveNoViolations();

    stub.tokenResponses.push(json({ error: "account_locked", error_description: "The user account has been locked out" }, 400));
    await user.type(within(dialog).getByLabelText("Password"), "nope");
    await user.click(within(dialog).getByRole("button", { name: /^sign in$/i }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/locked after too many failed sign-in attempts/i);

    stub.tokenResponses.push(tokenResponse("jwt-again"));
    await user.clear(within(dialog).getByLabelText("Password"));
    await user.type(within(dialog).getByLabelText("Password"), "right");
    await user.click(within(dialog).getByRole("button", { name: /^sign in$/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(screen.getByText("unsaved form")).toBeInTheDocument();
  });

  it("before the end, a notice offers to sign in again; the dialog can be put away until it's needed", async () => {
    liveBackend();
    signIn();
    expiresIn(3 * 60_000);
    renderWithProviders(
      <SessionGate area="tenant">
        <div>page body</div>
      </SessionGate>,
      { route: "/dashboard" },
    );

    const notice = await screen.findByText(/your session ends/i);
    fireEvent.click(within(notice.closest("[role=status]") as HTMLElement).getByRole("button", { name: /sign in again/i }));
    const dialog = screen.getByRole("dialog", { name: /sign in again/i });
    expect(within(dialog).getByLabelText("Password")).toHaveFocus();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText(/your session ends/i)).toBeInTheDocument();
  });
});
