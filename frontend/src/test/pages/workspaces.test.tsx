// /workspaces: the Notion-like chooser after sign-in — pick one, or create the first.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import WorkspacesPage from "@/app/workspaces/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ACME = "11111111-1111-1111-1111-111111111111";
const BETA = "22222222-2222-2222-2222-222222222222";

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: "OK", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
let assign: ReturnType<typeof vi.fn>;

function backend(mine: unknown[]) {
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url === "/api/app/workspaces/mine") return json(mine);
    if (url === "/api/app/workspaces" && method === "POST") return json({ id: BETA, name: JSON.parse(String(init?.body)).name });
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
});
