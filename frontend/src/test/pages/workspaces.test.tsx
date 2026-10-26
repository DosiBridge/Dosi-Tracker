// /workspaces: the Notion-like chooser after sign-in — pick one, or create the first.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import WorkspacesPage from "@/app/workspaces/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

import type { WorkspaceMembership } from "@/lib/sign-in";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ACME = "11111111-1111-1111-1111-111111111111";
const BETA = "22222222-2222-2222-2222-222222222222";
const GAMMA = "33333333-3333-3333-3333-333333333333";
const DELTA = "44444444-4444-4444-4444-444444444444";

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status === 200 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
let assign: ReturnType<typeof vi.fn>;
let currentMine: WorkspaceMembership[] = [];

function backend(mine: WorkspaceMembership[]) {
  currentMine = [...mine];
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url === "/api/app/workspaces/mine") return json(currentMine);
    if (url === "/api/app/workspaces" && method === "POST") return json({ id: BETA, name: JSON.parse(String(init?.body)).name });
    const archiveMatch = url.match(/^\/api\/app\/workspaces\/([^/]+)\/archive$/);
    if (archiveMatch && method === "POST") {
      const id = decodeURIComponent(archiveMatch[1]);
      currentMine = currentMine.map((w) => (w.id.toLowerCase() === id.toLowerCase() ? { ...w, isArchived: true } : w));
      return json({});
    }
    const restoreMatch = url.match(/^\/api\/app\/workspaces\/([^/]+)\/restore$/);
    if (restoreMatch && method === "POST") {
      const id = decodeURIComponent(restoreMatch[1]);
      currentMine = currentMine.map((w) => (w.id.toLowerCase() === id.toLowerCase() ? { ...w, isArchived: false } : w));
      return json({});
    }
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-f", name: "Free", pricePerUser: 0 }] });
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({ currentUser: { isAuthenticated: true, id: "u1", email: "a@b.test", roles: [] }, auth: { grantedPolicies: {} } });
    }
    return json({ items: [] });
  });
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, pathname: "/workspaces", assign });
  localStorage.setItem("dosi-token", "jwt");
});

afterEach(() => vi.unstubAllGlobals());

describe("workspace chooser", () => {
  it("lists the account's workspaces with their roles; choosing one opens it", async () => {
    backend([
      { id: ACME, name: "Acme", role: "owner", planName: "Free" },
      { id: BETA, name: "Beta", role: "member", planName: "Business" },
    ]);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    expect(within(list).getByRole("button", { name: /acme/i })).toHaveTextContent("Owner");
    expect(within(list).getByRole("button", { name: /beta/i })).toHaveTextContent("Member");
    await user.click(within(list).getByRole("button", { name: /beta/i }));
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
    expect(assign).toHaveBeenCalledWith("/dashboard");
  });

  it("with no workspace yet, asks to create the first one and opens it", async () => {
    backend([]);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    expect(await screen.findByRole("heading", { name: /create your first workspace/i })).toBeInTheDocument();
    await user.type(screen.getByLabelText(/workspace name/i), "NewCo");
    await user.click(screen.getByRole("button", { name: /^create workspace$/i }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
    const create = fetchMock.mock.calls.find(([u, i]) => String(u).endsWith("/api/app/workspaces") && i?.method === "POST");
    expect((create?.[1]?.headers as Record<string, string>)["X-Workspace-Id"]).toBeUndefined();
  });

  it("mixed active/archived workspaces show role controls and archived status; archived row is not switchable", async () => {
    backend([
      { id: ACME, name: "Acme", role: "owner", planName: "Free" },
      { id: BETA, name: "Beta", role: "member", planName: "Business" },
      { id: GAMMA, name: "Gamma", role: "owner", planName: "Starter", isArchived: true },
      { id: DELTA, name: "Delta", role: "member", planName: "Free", isArchived: true },
    ]);
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });

    const acmeLi = within(list).getByText("Acme").closest("li")!;
    const betaLi = within(list).getByText("Beta").closest("li")!;
    const gammaLi = within(list).getByText("Gamma").closest("li")!;
    const deltaLi = within(list).getByText("Delta").closest("li")!;

    // Active Acme (owner): switch button present, Archive button present
    expect(within(acmeLi).getByRole("button", { name: /acme/i })).toBeInTheDocument();
    expect(within(acmeLi).getByRole("button", { name: /^archive$/i })).toHaveAttribute("title", "Archive Acme");

    // Active Beta (member): switch button present, NO Archive button
    expect(within(betaLi).getByRole("button", { name: /beta/i })).toBeInTheDocument();
    expect(within(betaLi).queryByRole("button", { name: /^archive$/i })).not.toBeInTheDocument();

    // Archived Gamma (owner): Archived badge, Restore button, NO switch button
    expect(within(gammaLi).getByText("Archived")).toBeInTheDocument();
    expect(within(gammaLi).getByRole("button", { name: /^restore$/i })).toHaveAttribute("title", "Restore Gamma");
    expect(within(gammaLi).queryByRole("button", { name: /^gamma$/i })).not.toBeInTheDocument();

    // Archived Delta (member): Archived badge, NO Restore button, NO switch button
    expect(within(deltaLi).getByText("Archived")).toBeInTheDocument();
    expect(within(deltaLi).queryByRole("button", { name: /^restore$/i })).not.toBeInTheDocument();
    expect(within(deltaLi).queryByRole("button", { name: /^delta$/i })).not.toBeInTheDocument();
  });

  it("cancelling owner archive confirmation dialog makes zero API calls and preserves state", async () => {
    backend([
      { id: ACME, name: "Acme", role: "owner", planName: "Free" },
    ]);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    const acmeLi = within(list).getByText("Acme").closest("li")!;

    await user.click(within(acmeLi).getByRole("button", { name: /^archive$/i }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(/archive acme\?/i);
    expect(dialog).toHaveTextContent(/restore it at any time/i);
    expect(dialog).toHaveTextContent(/archived workspaces are not invoiced/i);

    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    const archiveCalls = fetchMock.mock.calls.filter(([u]) => String(u).includes("/archive"));
    expect(archiveCalls).toHaveLength(0);
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("confirming owner archive calls exact production route once, handles busy state, updates list and clears current workspace if matching", async () => {
    localStorage.setItem("dosi-workspace", ACME);
    backend([
      { id: ACME, name: "Acme", role: "owner", planName: "Free" },
      { id: BETA, name: "Beta", role: "member", planName: "Business" },
    ]);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    const acmeLi = within(list).getByText("Acme").closest("li")!;

    await user.click(within(acmeLi).getByRole("button", { name: /^archive$/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /archive workspace/i }));

    await waitFor(() => {
      const archiveCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).endsWith(`/api/app/workspaces/${ACME}/archive`) && i?.method === "POST"
      );
      expect(archiveCalls).toHaveLength(1);
    });

    // Current workspace matching archived workspace was cleared
    expect(localStorage.getItem("dosi-workspace")).toBeNull();

    // List updated to show Acme as archived with Restore button
    await waitFor(() => {
      const updatedAcmeLi = within(list).getByText("Acme").closest("li")!;
      expect(within(updatedAcmeLi).getByText("Archived")).toBeInTheDocument();
      expect(within(updatedAcmeLi).getByRole("button", { name: /^restore$/i })).toHaveAttribute("title", "Restore Acme");
    });
  });

  it("handles archive 403/rejection without modifying state, showing accessible error alert", async () => {
    localStorage.setItem("dosi-workspace", ACME);
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url === "/api/app/workspaces/mine") {
        return json([{ id: ACME, name: "Acme", role: "owner", planName: "Free" }]);
      }
      if (url.includes("/archive") && method === "POST") {
        return json({ error: { message: "Only owners may archive this workspace." } }, 403);
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    const acmeLi = within(list).getByText("Acme").closest("li")!;

    await user.click(within(acmeLi).getByRole("button", { name: /^archive$/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /archive workspace/i }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/only owners may archive this workspace/i);
    // Preserves current workspace
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("when all memberships are archived, shows chooser (not mandatory creation) and owner can restore", async () => {
    backend([
      { id: GAMMA, name: "Gamma", role: "owner", planName: "Starter", isArchived: true },
      { id: DELTA, name: "Delta", role: "member", planName: "Free", isArchived: true },
    ]);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });

    // Does NOT show "Create your first workspace"
    expect(await screen.findByRole("heading", { name: /choose a workspace/i })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /create your first workspace/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /create a new workspace/i })).toBeInTheDocument();

    const list = screen.getByRole("list", { name: /your workspaces/i });
    const gammaLi = within(list).getByText("Gamma").closest("li")!;
    const deltaLi = within(list).getByText("Delta").closest("li")!;

    // Non-owner cannot restore
    expect(within(deltaLi).queryByRole("button", { name: /^restore$/i })).not.toBeInTheDocument();

    // Owner can restore
    const restoreBtn = within(gammaLi).getByRole("button", { name: /^restore$/i });
    await user.click(restoreBtn);

    await waitFor(() => {
      const restoreCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).endsWith(`/api/app/workspaces/${GAMMA}/restore`) && i?.method === "POST"
      );
      expect(restoreCalls).toHaveLength(1);
    });

    // Refreshes options: Gamma is now active with switch button
    await waitFor(() => {
      expect(within(list).getByRole("button", { name: /gamma/i })).toBeInTheDocument();
    });
  });

  it("handles restore error gracefully with accessible error alert", async () => {
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url === "/api/app/workspaces/mine") {
        return json([{ id: GAMMA, name: "Gamma", role: "owner", planName: "Starter", isArchived: true }]);
      }
      if (url.includes("/restore") && method === "POST") {
        return json({ error: { message: "Server error restoring workspace" } }, 500);
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });

    const gammaLi = within(list).getByText("Gamma").closest("li")!;
    await user.click(within(gammaLi).getByRole("button", { name: /^restore$/i }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/server error restoring workspace/i);
    // Still shows as archived
    expect(within(gammaLi).getByText("Archived")).toBeInTheDocument();
  });

  it("archive POST succeeds but subsequent mine GET rejects: reconciles target locally as archived, clears matching context, shows refresh retry, and explicit retry reloads list", async () => {
    localStorage.setItem("dosi-workspace", ACME);
    let mineGetCount = 0;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url === "/api/app/workspaces/mine") {
        mineGetCount++;
        if (mineGetCount === 1) {
          return json([
            { id: ACME, name: "Acme", role: "owner", planName: "Free", isArchived: false },
            { id: BETA, name: "Beta", role: "member", planName: "Business", isArchived: false },
          ]);
        }
        if (mineGetCount === 2) {
          // Refresh call after archive POST rejects with 500
          return json({ error: { message: "Internal server error" } }, 500);
        }
        // Explicit retry (mineGetCount === 3) succeeds with updated server list
        return json([
          { id: ACME, name: "Acme", role: "owner", planName: "Free", isArchived: true },
          { id: BETA, name: "Beta", role: "member", planName: "Business", isArchived: false },
        ]);
      }
      if (url.endsWith(`/api/app/workspaces/${ACME}/archive`) && method === "POST") {
        return json({});
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    const acmeLi = within(list).getByText("Acme").closest("li")!;

    await user.click(within(acmeLi).getByRole("button", { name: /^archive$/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /archive workspace/i }));

    // Archive POST was sent once
    await waitFor(() => {
      const postCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).endsWith(`/api/app/workspaces/${ACME}/archive`) && i?.method === "POST"
      );
      expect(postCalls).toHaveLength(1);
    });

    // Matching context was cleared
    expect(localStorage.getItem("dosi-workspace")).toBeNull();

    // Target is reconciled locally as archived even though subsequent GET rejected
    await waitFor(() => {
      const updatedAcmeLi = within(list).getByText("Acme").closest("li")!;
      expect(within(updatedAcmeLi).getByText("Archived")).toBeInTheDocument();
      // Cannot switch or archive again from stale row
      expect(within(updatedAcmeLi).queryByRole("button", { name: /acme/i })).not.toBeInTheDocument();
      expect(within(updatedAcmeLi).queryByRole("button", { name: /^archive$/i })).not.toBeInTheDocument();
    });

    // Other active workspace Beta remains usable
    const betaLi = within(list).getByText("Beta").closest("li")!;
    expect(within(betaLi).getByRole("button", { name: /beta/i })).toBeInTheDocument();

    // Accessible refresh warning appears with explicit retry button
    const retryBtn = await screen.findByRole("button", { name: /retry/i });
    expect(screen.getByRole("alert")).toHaveTextContent(/couldn't refresh/i);

    // Clicking retry sends ONLY GET /mine (no extra POST calls)
    await user.click(retryBtn);

    await waitFor(() => {
      expect(mineGetCount).toBe(3);
    });
    const postCallsTotal = fetchMock.mock.calls.filter(([, i]) => i?.method === "POST");
    expect(postCallsTotal).toHaveLength(1); // exactly the initial archive POST, zero repeats
  });

  it("restore POST succeeds but subsequent mine GET rejects: reconciles target locally as active, prevents repeated restore POST, shows retry, and explicit retry reloads list", async () => {
    let mineGetCount = 0;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url === "/api/app/workspaces/mine") {
        mineGetCount++;
        if (mineGetCount === 1) {
          // All workspaces archived
          return json([
            { id: GAMMA, name: "Gamma", role: "owner", planName: "Starter", isArchived: true },
            { id: DELTA, name: "Delta", role: "member", planName: "Free", isArchived: true },
          ]);
        }
        if (mineGetCount === 2) {
          // Refresh call after restore POST rejects
          return json({ error: { message: "Internal server error" } }, 500);
        }
        // Explicit retry (mineGetCount === 3) succeeds
        return json([
          { id: GAMMA, name: "Gamma", role: "owner", planName: "Starter", isArchived: false },
          { id: DELTA, name: "Delta", role: "member", planName: "Free", isArchived: true },
        ]);
      }
      if (url.endsWith(`/api/app/workspaces/${GAMMA}/restore`) && method === "POST") {
        return json({});
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });

    const gammaLi = within(list).getByText("Gamma").closest("li")!;
    await user.click(within(gammaLi).getByRole("button", { name: /^restore$/i }));

    // Restore POST was sent once
    await waitFor(() => {
      const postCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).endsWith(`/api/app/workspaces/${GAMMA}/restore`) && i?.method === "POST"
      );
      expect(postCalls).toHaveLength(1);
    });

    // Target is reconciled locally as active: no repeated Restore button, switch button is available
    await waitFor(() => {
      const updatedGammaLi = within(list).getByText("Gamma").closest("li")!;
      expect(within(updatedGammaLi).queryByRole("button", { name: /^restore$/i })).not.toBeInTheDocument();
      expect(within(updatedGammaLi).getByRole("button", { name: /gamma/i })).toBeInTheDocument();
    });

    // Accessible refresh warning appears with explicit retry button
    const retryBtn = await screen.findByRole("button", { name: /retry/i });
    expect(screen.getByRole("alert")).toHaveTextContent(/couldn't refresh/i);

    // Clicking retry sends ONLY GET /mine (no extra POST calls)
    await user.click(retryBtn);

    await waitFor(() => {
      expect(mineGetCount).toBe(3);
    });
    const postCallsTotal = fetchMock.mock.calls.filter(([, i]) => i?.method === "POST");
    expect(postCallsTotal).toHaveLength(1); // exactly the initial restore POST, zero repeats
  });
});
