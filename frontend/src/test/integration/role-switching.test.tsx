// Role transitions through the REAL SessionProvider + RoleGuard: access must
// be granted and revoked LIVE when the session identity changes (not only at
// mount), landing destinations must follow the active role, and host
// impersonation must round-trip exactly (persisted across a simulated reload).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, screen } from "@testing-library/react";
import { useEffect } from "react";
import { RoleGuard } from "@/components/role-guard";
import { useSession } from "@/components/session-provider";
import { landingFor } from "@/lib/roles";
import { renderWithProviders, resetPrototypeState, seededUserId } from "@/test/harness";

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
      <span data-testid="landing">{landingFor(session.user.role)}</span>
      <RoleGuard>
        <div data-testid="guarded-content">guarded content</div>
      </RoleGuard>
    </div>
  );
}

beforeEach(() => {
  captured = null;
  resetPrototypeState();
});

describe("live role transitions against RoleGuard", () => {
  it("revokes /billing access the moment the session switches from owner to worker, and restores it on switch back", () => {
    renderWithProviders(<Probe />, { role: "owner", route: "/billing" });
    expect(screen.getByTestId("guarded-content")).toBeInTheDocument();

    act(() => sess().setUserById(seededUserId("worker")));

    expect(screen.queryByTestId("guarded-content")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /restricted area/i })).toBeInTheDocument();

    act(() => sess().setUserById(seededUserId("owner")));

    expect(screen.getByTestId("guarded-content")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /restricted area/i })).not.toBeInTheDocument();
  });

  it("the landing destination follows the active role: owner lands on /dashboard, host on /host", () => {
    renderWithProviders(<Probe />, { role: "owner" });
    expect(screen.getByTestId("landing")).toHaveTextContent("/dashboard");

    act(() => sess().loginAsHost());

    expect(screen.getByTestId("user-role")).toHaveTextContent("host");
    expect(screen.getByTestId("landing")).toHaveTextContent("/host");
  });
});

describe("host impersonation round-trip", () => {
  it("impersonating w2 makes the host that tenant's owner, persists across reload, and stopping returns to the host user exactly", () => {
    const view = renderWithProviders(<Probe />, { role: "host", route: "/host" });
    expect(screen.getByTestId("user-role")).toHaveTextContent("host");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("false");

    act(() => sess().impersonate("w2"));

    expect(screen.getByTestId("user-name")).toHaveTextContent("Diego Alvarez");
    expect(screen.getByTestId("user-role")).toHaveTextContent("owner");
    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Studio");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("true");
    expect(localStorage.getItem("dosi-impersonating")).toBe("1");
    expect(localStorage.getItem("dosi-user")).toBe("w2-u1");
    expect(localStorage.getItem("dosi-workspace")).toBe("w2");

    // Simulated reload: a fresh provider must restore the impersonated session.
    view.unmount();
    captured = null;
    renderWithProviders(<Probe />);
    expect(screen.getByTestId("user-name")).toHaveTextContent("Diego Alvarez");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("true");

    act(() => sess().stopImpersonating());

    expect(screen.getByTestId("user-name")).toHaveTextContent("Platform Admin");
    expect(screen.getByTestId("user-role")).toHaveTextContent("host");
    expect(screen.getByTestId("impersonating")).toHaveTextContent("false");
    expect(localStorage.getItem("dosi-user")).toBe("host");
    expect(localStorage.getItem("dosi-impersonating")).toBe("0");
  });

  it("impersonation drops host-console access live: /host is restricted while impersonating and restored after stopping", () => {
    renderWithProviders(<Probe />, { role: "host", route: "/host" });
    expect(screen.getByTestId("guarded-content")).toBeInTheDocument();

    act(() => sess().impersonate("w2"));

    expect(screen.queryByTestId("guarded-content")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /restricted area/i })).toBeInTheDocument();

    act(() => sess().stopImpersonating());

    expect(screen.getByTestId("guarded-content")).toBeInTheDocument();
  });
});
