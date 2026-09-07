// Session teardown + backend-hydration gating through the REAL SessionProvider.
//
// Contracts under test:
//  - logout wipes the stored identity (dosi-user, dosi-token), turns
//    impersonation off, and lands the session back on the default owner;
//  - without a dosi-token the provider must not touch the network at all;
//  - with a token, a dead backend (every fetch rejects) must never crash the
//    tree — the mock tenant data keeps rendering (graceful degradation) AND
//    stays intact: a failed hydration must not replace the active tenant's
//    projects/activities with empty arrays.
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
import { useSession } from "@/components/session-provider";
import { datasetFor } from "@/lib/tenant-data";
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
      <span data-testid="user-name">{session.user.name}</span>
      <span data-testid="user-role">{session.user.role}</span>
      <span data-testid="workspace-name">{session.workspace.name}</span>
      <span data-testid="impersonating">{String(session.isImpersonating)}</span>
      <ul data-testid="roster">
        {datasetFor(session.workspace.id).users.map((u) => (
          <li key={u.id}>{u.name}</li>
        ))}
      </ul>
    </div>
  );
}

beforeEach(() => {
  captured = null;
  resetPrototypeState();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("logout", () => {
  it("clears the stored session and token, ends impersonation, and lands on the default owner", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network disabled in tests"))));
    localStorage.setItem("dosi-token", "jwt-test-token");
    localStorage.setItem("dosi-impersonating", "1"); // stale flag from a previous session

    renderWithProviders(<Probe />, { role: "worker" }); // Tanvir Hasan (u2)
    await act(async () => {});
    expect(screen.getByTestId("user-name")).toHaveTextContent("Tanvir Hasan");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("true");

    act(() => sess().logout());

    expect(localStorage.getItem("dosi-user")).toBeNull();
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(localStorage.getItem("dosi-impersonating")).toBe("0");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("false");
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
  });

  it("keeps the demo tenant data intact when every hydration fetch fails with a token present", async () => {
    localStorage.setItem("dosi-token", "jwt-test-token");
    const fetchSpy = vi.fn(() => Promise.reject(new Error("backend down")));
    vi.stubGlobal("fetch", fetchSpy);

    renderWithProviders(<Probe />, { role: "owner" });

    await waitFor(() => expect(fetchSpy).toHaveBeenCalled());
    await act(async () => {});

    // Graceful degradation: the tree survived and the demo tenant still renders.
    expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
    const roster = screen.getByTestId("roster");
    expect(within(roster).getByText("Tanvir Hasan")).toBeInTheDocument();
    expect(within(roster).getByText("David Chen")).toBeInTheDocument();

    // Data integrity: a merely-unreachable backend must NOT hydrate empty
    // arrays over the active tenant — the seeded data survives untouched.
    const ds = datasetFor("w1");
    expect(ds.projects.length).toBeGreaterThan(0);
    expect(ds.activities.length).toBeGreaterThan(0);
    expect(ds.projects.some((p) => p.title === "Dosi Web Platform")).toBe(true);
  });
});
