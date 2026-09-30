// Multi-tenant workspace switching through the REAL SessionProvider: switching
// workspaces must swap BOTH the workspace identity (name/plan/status) and the
// tenant dataset (roster), never leak one tenant's data into another, relocate
// users that don't exist in the target tenant, and persist workspace
// create/update/delete through localStorage so they survive a reload.
import { describe, it, expect, beforeEach } from "vitest";
import { act, screen, within } from "@testing-library/react";
import { useEffect } from "react";
import { useSession } from "@/components/session-provider";
import { datasetFor } from "@/lib/tenant-data";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";
import { buildUser } from "@/test/factories";

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
      <span data-testid="workspace-plan">{session.workspace.planId}</span>
      <span data-testid="workspace-status">{session.workspace.status}</span>
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

describe("workspace switching", () => {
  it("switching w1 → w2 swaps the workspace identity AND the tenant dataset", () => {
    renderWithProviders(<Probe />, { role: "owner" });
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
    expect(screen.getByTestId("workspace-plan")).toHaveTextContent("business");
    const w1Roster = screen.getByTestId("roster");
    expect(within(w1Roster).getByText("Ayesha Rahman")).toBeInTheDocument();
    expect(within(w1Roster).queryByText("Diego Alvarez")).not.toBeInTheDocument();

    act(() => sess().setWorkspaceById("w2"));

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Studio");
    expect(screen.getByTestId("workspace-plan")).toHaveTextContent("starter");
    const w2Roster = screen.getByTestId("roster");
    expect(within(w2Roster).getByText("Diego Alvarez")).toBeInTheDocument();
    expect(within(w2Roster).getByText("Lucia Romero")).toBeInTheDocument();
    expect(within(w2Roster).queryByText("Ayesha Rahman")).not.toBeInTheDocument();
    expect(within(w2Roster).queryByText("Tanvir Hasan")).not.toBeInTheDocument();
  });

  it("a user added to w2 never leaks into w1 when switching back and forth (tenant isolation)", () => {
    renderWithProviders(<Probe />, { role: "owner" });
    const w1Baseline = datasetFor("w1").users.map((u) => u.name);

    const intruder = buildUser({ name: "Mallory Intruder", role: "worker" });
    datasetFor("w2").users.push(intruder);

    act(() => sess().setWorkspaceById("w2"));
    expect(within(screen.getByTestId("roster")).getByText("Mallory Intruder")).toBeInTheDocument();

    act(() => sess().setWorkspaceById("w1"));
    const w1Roster = screen.getByTestId("roster");
    expect(within(w1Roster).queryByText("Mallory Intruder")).not.toBeInTheDocument();
    // w1's dataset is byte-for-byte the same roster as before the mutation…
    expect(datasetFor("w1").users.map((u) => u.name)).toEqual(w1Baseline);
    // …while w2 keeps its own mutation (it moved with the tenant, not the session).
    expect(datasetFor("w2").users.some((u) => u.id === intruder.id)).toBe(true);
  });

  it("switching into a tenant that doesn't know the current user lands them as that tenant's owner", () => {
    renderWithProviders(<Probe />, { role: "worker" }); // Tanvir Hasan (u2), w1 only
    expect(screen.getByTestId("user-name")).toHaveTextContent("Tanvir Hasan");

    act(() => sess().setWorkspaceById("w3"));

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Nimbus Co");
    expect(screen.getByTestId("user-name")).toHaveTextContent("Kwame Mensah");
    expect(screen.getByTestId("user-role")).toHaveTextContent("owner");
    // The relocation is persisted exactly like a normal sign-in.
    expect(localStorage.getItem("dosi-user")).toBe("w3-u1");
    expect(localStorage.getItem("dosi-workspace")).toBe("w3");
  });

  it("the host keeps their platform identity when browsing another tenant", () => {
    renderWithProviders(<Probe />, { role: "host" });

    act(() => sess().setWorkspaceById("w2"));

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Studio");
    expect(screen.getByTestId("user-name")).toHaveTextContent("Platform Admin");
    expect(screen.getByTestId("user-role")).toHaveTextContent("host");
  });
});

describe("workspace lifecycle (create / update / delete)", () => {
  it("creating a paid workspace starts a 14-day trial, becomes active and persists to localStorage", () => {
    renderWithProviders(<Probe />, { role: "owner" });

    act(() => {
      sess().createWorkspace("Orbit Studio", "starter");
    });

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Orbit Studio");
    expect(screen.getByTestId("workspace-status")).toHaveTextContent("trialing");

    const stored: unknown = JSON.parse(localStorage.getItem("dosi-workspaces-created") ?? "null");
    expect(stored).toEqual([
      expect.objectContaining({
        name: "Orbit Studio",
        planId: "starter",
        status: "trialing",
        cycleDays: 14,
      }),
    ]);
  });

  it("creating a free workspace activates immediately with no trial cycle", () => {
    renderWithProviders(<Probe />, { role: "owner" });

    act(() => {
      sess().createWorkspace("Side Project", "free");
    });

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Side Project");
    expect(screen.getByTestId("workspace-status")).toHaveTextContent("active");

    const stored: unknown = JSON.parse(localStorage.getItem("dosi-workspaces-created") ?? "null");
    expect(stored).toEqual([
      expect.objectContaining({ name: "Side Project", planId: "free", status: "active", cycleDays: 0 }),
    ]);
  });

  it("a workspace rename survives a simulated reload (unmount + fresh provider)", () => {
    const view = renderWithProviders(<Probe />, { role: "owner", workspaceId: "w2" });
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Studio");

    act(() => sess().updateWorkspace("w2", { name: "Acme Rebranded" }));
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Rebranded");

    view.unmount();
    captured = null;
    renderWithProviders(<Probe />); // reads only what localStorage kept

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Rebranded");
    expect(screen.getByTestId("workspace-plan")).toHaveTextContent("starter"); // untouched fields survive
  });

  it("deleting the active workspace removes it and falls back to the primary workspace", () => {
    renderWithProviders(<Probe />, { role: "owner", workspaceId: "w2" }); // Diego, w2

    act(() => sess().deleteWorkspace("w2"));

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
    expect(sess().workspaces.map((w) => w.id)).not.toContain("w2");
    expect(JSON.parse(localStorage.getItem("dosi-workspaces-deleted") ?? "[]")).toContain("w2");
    // The displaced w2 owner is relocated onto the primary tenant's owner.
    expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");
  });
});
