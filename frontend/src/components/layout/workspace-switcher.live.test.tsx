// Live WorkspaceSwitcher (Notion-style): lists /workspaces/mine with role
// badges; switching only changes the current workspace (no password, no new
// token) and reloads the dashboard; create and leave work in place.
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const api = vi.hoisted(() => ({
  getMyWorkspacesApi: vi.fn(),
  createWorkspaceApi: vi.fn(),
  leaveWorkspaceApi: vi.fn(),
  archiveWorkspaceApi: vi.fn(),
  getApi: vi.fn(),
}));
const toasts = vi.hoisted(() => ({ toast: vi.fn() }));
const session = vi.hoisted(() => ({
  workspace: { id: "live-tenant-a", name: "Acme", color: "#333", planId: "free", status: "active" },
  workspaces: [],
  setWorkspaceById: () => {},
  createWorkspace: () => ({}),
  setUserById: () => {},
  isLive: true,
  user: { email: "ayesha@dosi.dev", name: "admin" },
}));

vi.mock("@/components/session-provider", () => ({ useSession: () => session }));
vi.mock("@/hooks/useApi", () => api);
vi.mock("@/components/toast", () => toasts);

import { WorkspaceSwitcher } from "@/components/layout/workspace-switcher";

const ACME = "11111111-1111-1111-1111-111111111111";
const BETA = "22222222-2222-2222-2222-222222222222";
const items = [
  { id: ACME, name: "Acme", role: "owner", planName: "Free", isOwner: true },
  { id: BETA, name: "Beta", role: "member", planName: "Business", isOwner: false },
];

let assign: ReturnType<typeof vi.fn>;

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  toasts.toast.mockReset();
  localStorage.clear();
  localStorage.setItem("dosi-workspace", ACME);
  api.getMyWorkspacesApi.mockResolvedValue(items);
  api.getApi.mockResolvedValue({
    items: [
      { id: "plan-b", name: "Business", pricePerUser: 12 },
      { id: "plan-f", name: "Free", pricePerUser: 0 },
    ],
  });
  assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, pathname: "/dashboard", assign });
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

async function openMenu() {
  const user = userEvent.setup();
  render(<WorkspaceSwitcher collapsed={false} />);
  await user.click(screen.getByRole("button", { name: /acme/i }));
  await screen.findByRole("button", { current: true });
  return user;
}

describe("WorkspaceSwitcher (live)", () => {
  it("loads the workspaces when opened, with role badges, and checks the current one", async () => {
    const user = userEvent.setup();
    render(<WorkspaceSwitcher collapsed={false} />);
    expect(api.getMyWorkspacesApi).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /acme/i }));
    const current = await screen.findByRole("button", { current: true });
    expect(current).toHaveTextContent("Acme");
    expect(current).toHaveTextContent("Owner");
    expect(screen.getByRole("button", { name: /beta/i })).toHaveTextContent("Member");
    expect(api.getMyWorkspacesApi).toHaveBeenCalledTimes(1);
  });

  it("switches instantly: stores the workspace and reloads the dashboard — no password", async () => {
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /beta/i }));
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
    expect(assign).toHaveBeenCalledWith("/dashboard");
    expect(document.querySelector('input[type="password"]')).toBeNull();
  });

  it("clicking the current workspace stays put", async () => {
    const user = await openMenu();
    await user.click(screen.getByRole("button", { current: true }));
    expect(assign).not.toHaveBeenCalled();
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("creates a workspace (name + plan) and switches into it", async () => {
    api.createWorkspaceApi.mockResolvedValue({ id: BETA, name: "NewCo" });
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /create workspace/i }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText(/workspace name/i), "NewCo");
    await waitFor(() => expect(within(dialog).getByLabelText(/plan/i)).toHaveValue("plan-f"));
    await user.click(within(dialog).getByRole("button", { name: /^create workspace$/i }));
    await waitFor(() => expect(api.createWorkspaceApi).toHaveBeenCalledWith("NewCo", "plan-f"));
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
    expect(assign).toHaveBeenCalledWith("/dashboard");
  });

  it("a duplicate workspace name is shown inline", async () => {
    api.createWorkspaceApi.mockRejectedValue(Object.assign(new Error("x"), { status: 409, code: "Tracker:DuplicateWorkspaceName" }));
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /create workspace/i }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText(/workspace name/i), "Acme");
    await user.click(within(dialog).getByRole("button", { name: /^create workspace$/i }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/already exists/i);
    expect(assign).not.toHaveBeenCalled();
  });

  it("leaves the current workspace after confirming, then goes to the chooser", async () => {
    api.leaveWorkspaceApi.mockResolvedValue(undefined);
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /leave workspace/i }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(/leave acme/i);
    await user.click(within(dialog).getByRole("button", { name: /leave workspace/i }));
    await waitFor(() => expect(api.leaveWorkspaceApi).toHaveBeenCalledWith(ACME));
    expect(assign).toHaveBeenCalledWith("/workspaces");
  });

  it("the last owner can't leave: says so and stays", async () => {
    api.leaveWorkspaceApi.mockRejectedValue(Object.assign(new Error("x"), { status: 409, code: "Tracker:LastOwnerCannotLeave" }));
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /leave workspace/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /leave workspace/i }));
    await waitFor(() => expect(toasts.toast).toHaveBeenCalled());
    expect(toasts.toast.mock.calls[0][0].description).toMatch(/last owner/i);
    expect(assign).not.toHaveBeenCalled();
  });

  it("renders archived workspaces with Archived badge and disabled; clicking does not switch", async () => {
    const GAMMA = "33333333-3333-3333-3333-333333333333";
    api.getMyWorkspacesApi.mockResolvedValue([
      ...items,
      { id: GAMMA, name: "Gamma", role: "member", planName: "Free", isOwner: false, isArchived: true },
    ]);
    const user = await openMenu();
    const archivedItem = screen.getByText("Gamma").closest("div");
    expect(archivedItem).toHaveAttribute("aria-disabled", "true");
    expect(within(archivedItem!).getByText("Archived")).toBeInTheDocument();

    await user.click(within(archivedItem!).getByText("Gamma"));
    expect(assign).not.toHaveBeenCalled();
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("owner sees archive option; cancelling makes zero requests and stays put", async () => {
    const user = await openMenu();
    const archiveBtn = screen.getByRole("button", { name: /archive workspace/i });
    expect(archiveBtn).toBeInTheDocument();

    await user.click(archiveBtn);
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(/archive acme\?/i);
    expect(dialog).toHaveTextContent(/subscription billing is not cancelled/i);

    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));
    expect(api.archiveWorkspaceApi).not.toHaveBeenCalled();
    expect(assign).not.toHaveBeenCalled();
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("owner confirms archive: calls archiveWorkspaceApi, clears current workspace and goes to chooser", async () => {
    api.archiveWorkspaceApi.mockResolvedValue(undefined);
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /archive workspace/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^archive workspace$/i }));

    await waitFor(() => expect(api.archiveWorkspaceApi).toHaveBeenCalledWith(ACME));
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
    expect(assign).toHaveBeenCalledWith("/workspaces");
  });

  it("archive failure displays error toast and does not redirect or clear workspace", async () => {
    api.archiveWorkspaceApi.mockRejectedValue({
      serverMessage: "Forbidden: Only owners may archive",
      status: 403,
    });
    const user = await openMenu();
    await user.click(screen.getByRole("button", { name: /archive workspace/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^archive workspace$/i }));

    await waitFor(() => expect(toasts.toast).toHaveBeenCalled());
    expect(toasts.toast).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Couldn't archive the workspace",
        description: "Forbidden: Only owners may archive",
        tone: "danger",
      })
    );
    expect(assign).not.toHaveBeenCalled();
    expect(localStorage.getItem("dosi-workspace")).toBe(ACME);
  });

  it("non-owner does not see archive option", async () => {
    localStorage.setItem("dosi-workspace", BETA);
    const user = userEvent.setup();
    render(<WorkspaceSwitcher collapsed={false} />);
    await user.click(screen.getByRole("button", { name: /acme/i }));
    await screen.findByRole("button", { current: true });

    expect(screen.queryByRole("button", { name: /archive workspace/i })).not.toBeInTheDocument();
  });
});
