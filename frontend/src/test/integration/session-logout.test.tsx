// Session teardown + backend hydration through the REAL SessionProvider.
//
// Contracts under test:
//  - without a dosi-token the provider must not touch the network at all, and
//    the demo dataset renders (demo mode);
//  - with a token, the session is built ONLY from the backend: the real user
//    (name/email/role), the real tenant as the one and only workspace, the real
//    roster — never a demo person or demo company;
//  - with a token and a dead backend (every fetch rejects) the tree must not
//    crash, must show an ERROR state rather than demo data, and must leave the
//    demo dataset untouched (a failed load never writes empty arrays over it);
//  - logout wipes the stored identity (dosi-user, dosi-token), drops the live
//    tenant's data, turns impersonation off and returns to the demo default;
//  - refreshSession() re-hydrates after a client-side sign-in (no reload).
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
import { useSession } from "@/components/session-provider";
import { datasetFor, isLiveDataset } from "@/lib/tenant-data";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

let captured: ReturnType<typeof useSession> | null = null;

/** The session context value captured by the currently mounted probe. */
function sess(): ReturnType<typeof useSession> {
  if (!captured) throw new Error("Probe is not mounted");
  return captured;
}

function Probe() {
  const session = useSession();
  // Capture outside the render phase so tests can drive the real context API.
  useEffect(() => {
    captured = session;
  });
  return (
    <div>
      <span data-testid="status">{session.status}</span>
      <span data-testid="user-name">{session.user.name}</span>
      <span data-testid="user-email">{session.user.email}</span>
      <span data-testid="user-role">{session.user.role}</span>
      <span data-testid="workspace-name">{session.workspace.name}</span>
      <span data-testid="workspace-count">{session.workspaces.length}</span>
      <span data-testid="impersonating">{String(session.isImpersonating)}</span>
      <ul data-testid="roster">
        {datasetFor(session.workspace.id).users.map((u) => (
          <li key={u.id}>{u.name}</li>
        ))}
      </ul>
    </div>
  );
}

function json(body: unknown): Response {
  const text = JSON.stringify(body);
  return { ok: true, status: 200, statusText: "OK", json: async () => JSON.parse(text), text: async () => text } as Response;
}

const OWNER_ID = "11111111-1111-1111-1111-111111111111";
const WORKER_ID = "22222222-2222-2222-2222-222222222222";

function tenantConfig(overrides: { roles?: string[]; policies?: Record<string, boolean> } = {}) {
  return {
    currentUser: {
      isAuthenticated: true,
      id: OWNER_ID,
      tenantId: "t-1",
      userName: "mira",
      name: "Mira",
      surName: "Okafor",
      email: "mira@realco.test",
      roles: overrides.roles ?? ["admin"],
    },
    currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
    auth: { grantedPolicies: overrides.policies ?? {} },
  };
}

/** A healthy backend for a tenant owner with one invited worker. */
function stubLiveBackend(config = tenantConfig()) {
  const fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/api/abp/application-configuration")) return json(config);
    if (url.includes("/api/app/team/members")) {
      return json({
        items: [
          { userId: OWNER_ID, userName: "mira", name: "Mira", surname: "Okafor", email: "mira@realco.test", isActive: true, isOwner: true, isManager: true, projectIds: ["p-real"] },
          { userId: WORKER_ID, userName: "sam", name: "Sam", surname: "Lee", email: "sam@realco.test", isActive: true, isOwner: false, isManager: false, projectIds: ["p-real"] },
        ],
      });
    }
    if (url.includes("/api/app/project")) return json({ items: [{ id: "p-real", title: "Real Project", isArchived: false }] });
    if (url.includes("/api/app/activity")) return json({ items: [] });
    if (url.includes("/api/app/workspace/current-subscription")) return json({ planId: "plan-starter", status: "trialing", trialEndsAt: null });
    if (url.includes("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-starter", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchSpy);
  return fetchSpy;
}

beforeEach(() => {
  captured = null;
  resetPrototypeState();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("live session identity (token present)", () => {
  it("builds the user and the one workspace from the backend — no demo person or company", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "jwt-live");
    localStorage.setItem("dosi-user", "u2"); // a stale demo id must be ignored in a live session

    renderWithProviders(<Probe />);

    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));
    expect(screen.getByTestId("user-name")).toHaveTextContent("Mira Okafor");
    expect(screen.getByTestId("user-email")).toHaveTextContent("mira@realco.test");
    expect(screen.getByTestId("user-role")).toHaveTextContent("owner"); // tenant "admin" role → owner
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("RealCo");
    expect(screen.getByTestId("workspace-count")).toHaveTextContent("1"); // no demo tenants in the switcher

    const roster = within(screen.getByTestId("roster"));
    expect(roster.getByText("Mira Okafor")).toBeInTheDocument();
    expect(roster.getByText("Sam Lee")).toBeInTheDocument();
    expect(roster.queryByText("Ayesha Rahman")).not.toBeInTheDocument();
    expect(isLiveDataset()).toBe(true);

    // The live session wrote into ITS tenant, not the demo workspace (the old
    // stale-closure bug pushed the real user into w1's roster).
    expect(datasetFor("w1").users.some((u) => u.id === OWNER_ID)).toBe(false);
  });

  it("maps a team manager (no tenant admin role) to the admin role", async () => {
    stubLiveBackend(tenantConfig({ roles: [], policies: { "Tracker.Team.Manage": true } }));
    localStorage.setItem("dosi-token", "jwt-live");

    renderWithProviders(<Probe />);

    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));
    expect(screen.getByTestId("user-role")).toHaveTextContent("admin");
  });

  it("ignores demo impersonation and workspace switching in a live session", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "jwt-live");
    localStorage.setItem("dosi-impersonating", "1");

    renderWithProviders(<Probe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));
    expect(screen.getByTestId("impersonating")).toHaveTextContent("false");

    act(() => sess().impersonate("w2"));
    act(() => sess().setWorkspaceById("w3"));
    act(() => sess().setUserById("u1"));
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("RealCo");
    expect(screen.getByTestId("user-name")).toHaveTextContent("Mira Okafor");
  });

  it("refreshSession() hydrates after a client-side sign-in, without a reload", async () => {
    const fetchSpy = stubLiveBackend();
    renderWithProviders(<Probe />);
    await act(async () => {});
    expect(screen.getByTestId("status")).toHaveTextContent("demo");
    expect(fetchSpy).not.toHaveBeenCalled();

    localStorage.setItem("dosi-token", "jwt-fresh"); // what loginApi does
    let signedIn: Awaited<ReturnType<ReturnType<typeof useSession>["refreshSession"]>> = null;
    await act(async () => {
      signedIn = await sess().refreshSession();
    });

    expect(signedIn).not.toBeNull();
    expect(screen.getByTestId("status")).toHaveTextContent("ready");
    expect(screen.getByTestId("user-name")).toHaveTextContent("Mira Okafor");
  });

  it("a token the backend no longer accepts ends the session instead of showing anyone", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => json({ currentUser: { isAuthenticated: false } })));
    localStorage.setItem("dosi-token", "jwt-expired");

    renderWithProviders(<Probe />);

    await waitFor(() => expect(localStorage.getItem("dosi-token")).toBeNull());
    expect(screen.getByTestId("status")).toHaveTextContent("demo");
  });
});

describe("logout", () => {
  it("clears the stored session and token, drops the live data, ends impersonation, and returns to the demo default", async () => {
    stubLiveBackend();
    localStorage.setItem("dosi-token", "jwt-test-token");
    localStorage.setItem("dosi-impersonating", "1"); // stale flag from a previous session

    renderWithProviders(<Probe />, { role: "worker" });
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("ready"));
    expect(screen.getByTestId("user-name")).toHaveTextContent("Mira Okafor");

    act(() => sess().logout());

    expect(localStorage.getItem("dosi-user")).toBeNull();
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(localStorage.getItem("dosi-impersonating")).toBe("0");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("false");
    expect(screen.getByTestId("status")).toHaveTextContent("demo");
    expect(isLiveDataset()).toBe(false);
    expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");
    expect(screen.getByTestId("user-role")).toHaveTextContent("owner");
  });
});

describe("backend hydration gating", () => {
  it("makes zero network requests when no dosi-token is stored", async () => {
    const fetchSpy = vi.fn(() => Promise.reject(new Error("must never be called")));
    vi.stubGlobal("fetch", fetchSpy);

    renderWithProviders(<Probe />, { role: "owner" });
    await act(async () => {});
    await act(async () => {});

    expect(fetchSpy).not.toHaveBeenCalled();
    expect(screen.getByTestId("status")).toHaveTextContent("demo");
  });

  it("shows an error state — never the demo tenant — when the backend is unreachable with a token present", async () => {
    localStorage.setItem("dosi-token", "jwt-test-token");
    const fetchSpy = vi.fn(() => Promise.reject(new Error("backend down")));
    vi.stubGlobal("fetch", fetchSpy);

    renderWithProviders(<Probe />, { role: "owner" });

    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("error"));
    expect(fetchSpy).toHaveBeenCalled();
    expect(sess().liveError).toMatch(/couldn't load your workspace/i);

    // No demo identity, company or roster leaks into a live session.
    expect(screen.getByTestId("user-name")).not.toHaveTextContent("Ayesha Rahman");
    expect(screen.getByTestId("workspace-name")).not.toHaveTextContent("Dosi Labs");
    expect(within(screen.getByTestId("roster")).queryAllByRole("listitem")).toHaveLength(0);

    // Data integrity: the demo dataset itself is untouched by the failed load.
    const ds = datasetFor("w1");
    expect(ds.projects.length).toBeGreaterThan(0);
    expect(ds.activities.length).toBeGreaterThan(0);
    expect(ds.projects.some((p) => p.title === "Dosi Web Platform")).toBe(true);
  });
});
