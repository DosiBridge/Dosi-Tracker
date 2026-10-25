// Demo mode is a deployment's explicit choice (DOSI_DEMO_MODE), off by default.
//
// Contracts under test (FE-05):
//  - without demo mode, a visitor with no token is signed OUT: the session
//    cookie alone (stale, or set by hand) never shows the demo dataset; the
//    gate clears the cookie and leaves for /login;
//  - with demo mode on, the demo tenants arrive through a dynamic import
//    (their own chunk) the first time a token-less visitor needs them, and the
//    shell waits for them instead of flashing an empty workspace;
//  - a demo chunk that fails to load is an honest error with Try again;
//  - a live session is unaffected by the switch.
vi.mock("@/lib/live-session", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/live-session")>()),
  redirectToLogin: vi.fn(),
  reloadPage: vi.fn(),
}));
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useSession } from "@/components/session-provider";
import { redirectToLogin } from "@/lib/live-session";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

function Status() {
  const { status, user, workspaces } = useSession();
  return (
    <div>
      <span data-testid="status">{status}</span>
      <span data-testid="user">{user.name}</span>
      <span data-testid="workspaces">{workspaces.map((w) => w.name).join(",")}</span>
    </div>
  );
}

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(redirectToLogin).mockClear();
  document.cookie = "dosi-token=1; Path=/";
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      throw new Error("a token-less visitor must never call the API");
    }),
  );
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.resetModules();
  vi.doUnmock("@/lib/demo-tenants");
  document.cookie = "dosi-token=; Path=/; Max-Age=0";
});

describe("without demo mode", () => {
  it("a session cookie with no token is a signed-out visitor: no demo data, cookie cleared, sent to /login", async () => {
    vi.stubEnv("DOSI_DEMO_MODE", "0");
    const { SessionGate } = await import("@/components/session-gate");
    renderWithProviders(
      <SessionGate area="tenant">
        <Status />
      </SessionGate>,
      { route: "/dashboard" },
    );

    expect(await screen.findByText(/signing you out/i)).toBeInTheDocument();
    await waitFor(() => expect(redirectToLogin).toHaveBeenCalled());
    expect(document.cookie).not.toMatch(/dosi-token=1/);
    expect(screen.queryByTestId("status")).not.toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/Ayesha Rahman|Dosi Labs/);
    expect(fetch).not.toHaveBeenCalled();
  });

  it("the session says anonymous and offers no demo workspaces (even ones a demo visit created here)", async () => {
    vi.stubEnv("DOSI_DEMO_MODE", "");
    localStorage.setItem("dosi-workspaces-created", JSON.stringify([{ id: "w-1", name: "Leftover", planId: "free", status: "active" }]));
    renderWithProviders(<Status />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("anonymous"));
    expect(screen.getByTestId("workspaces")).toHaveTextContent("");
    expect(screen.getByTestId("user")).toHaveTextContent("Signed out");
  });
});

describe("with demo mode", () => {
  it("renders the seeded demo for a token-less visitor", async () => {
    renderWithProviders(<Status />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("demo"));
    expect(screen.getByTestId("workspaces")).toHaveTextContent("Dosi Labs,Acme Studio,Nimbus Co");
    expect(screen.getByTestId("user")).toHaveTextContent("Ayesha Rahman");
  });

  it("loads the demo tenants on demand, holding the shell until they arrive", async () => {
    // A fresh module registry: nothing has installed the demo tenants yet, as in a browser.
    vi.resetModules();
    const { SessionProvider, useSession: useFreshSession } = await import("@/components/session-provider");
    const { SessionGate } = await import("@/components/session-gate");
    const tenantData = await import("@/lib/tenant-data");
    expect(tenantData.demoDatasetsInstalled()).toBe(false);

    function FreshStatus() {
      const { status, workspace } = useFreshSession();
      return <span data-testid="fresh">{`${status}:${workspace.name}`}</span>;
    }

    render(
      <SessionProvider>
        <SessionGate area="tenant">
          <FreshStatus />
        </SessionGate>
      </SessionProvider>,
    );
    // The demo chunk is on its way: a loading screen, never an empty workspace.
    expect(screen.getByText(/loading your workspace/i)).toBeInTheDocument();
    expect(await screen.findByTestId("fresh")).toHaveTextContent("demo:Dosi Labs");
    expect(tenantData.demoDatasetsInstalled()).toBe(true);
    expect(tenantData.datasetFor("w1").users.length).toBeGreaterThan(0);
  });

  it("says so, with Try again, when the demo chunk can't be loaded", async () => {
    vi.resetModules();
    vi.doMock("@/lib/demo-tenants", () => {
      throw new Error("ChunkLoadError: Loading chunk failed");
    });
    const { SessionProvider } = await import("@/components/session-provider");
    const { SessionGate } = await import("@/components/session-gate");

    render(
      <SessionProvider>
        <SessionGate area="tenant">
          <span>app</span>
        </SessionGate>
      </SessionProvider>,
    );
    expect(await screen.findByText(/the demo couldn't be loaded/i)).toBeInTheDocument();
    expect(screen.queryByText("app")).not.toBeInTheDocument();

    // The network is back: Try again loads it.
    vi.doUnmock("@/lib/demo-tenants");
    await userEvent.setup().click(screen.getByRole("button", { name: /try again/i }));
    expect(await screen.findByText("app")).toBeInTheDocument();
  });
});
