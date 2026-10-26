// /workspaces journeys (E2E-01, E2E-07): invitations addressed to the signed-in
// person are answered here (nobody joins without accepting), an unconfirmed
// email explains why the list is empty, and an owner deletes a workspace by
// typing its name.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import WorkspacesPage from "@/app/workspaces/page";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";
import type { WorkspaceMembership } from "@/lib/sign-in";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ACME = "11111111-1111-1111-1111-111111111111";
const BETA = "22222222-2222-2222-2222-222222222222";

function json(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
let assign: ReturnType<typeof vi.fn>;
const calls = (path: string, method = "GET") =>
  fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "") === path && (i?.method ?? "GET") === method);

interface Backend {
  mine?: WorkspaceMembership[];
  invitations?: unknown;
  accept?: { body: unknown; status?: number };
  confirmation?: unknown;
  remove?: { body: unknown; status?: number };
}

function backend(opts: Backend = {}) {
  let mine = [...(opts.mine ?? [{ id: ACME, name: "Acme", role: "owner", planName: "Free" }])];
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url === "/api/app/workspaces/mine") return json(mine);
    if (url === "/api/app/invitations/mine") {
      return json(opts.invitations ?? { emailConfirmed: true, items: [] });
    }
    if (url === "/api/app/invitations/mine/inv-1/accept" && method === "POST") {
      return json(opts.accept?.body ?? { workspaceId: BETA, workspaceName: "Beta", role: "member" }, opts.accept?.status ?? 200);
    }
    if (url === "/api/app/invitations/mine/inv-1/decline" && method === "POST") return json(undefined, 204);
    if (url === "/api/app/my-account/send-email-confirmation" && method === "POST") {
      return json(opts.confirmation ?? { alreadyConfirmed: false, emailSent: true });
    }
    if (url === `/api/app/workspaces/${ACME}/delete` && method === "POST") {
      if (opts.remove) return json(opts.remove.body, opts.remove.status ?? 200);
      mine = mine.filter((w) => w.id !== ACME);
      return json({ id: ACME, deletionTime: "2026-10-26T08:00:00Z", purgeAfter: "2026-11-02T08:00:00Z" });
    }
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [] });
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({ currentUser: { isAuthenticated: true, id: "u1", email: "a@b.test", roles: [] }, auth: { grantedPolicies: {} } });
    }
    return json({ items: [] });
  });
}

const PENDING = {
  emailConfirmed: true,
  items: [{ id: "inv-1", workspaceId: BETA, workspaceName: "Beta", role: "manager", expiresAt: "2026-11-01T00:00:00Z" }],
};

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, pathname: "/workspaces", assign });
  localStorage.setItem("dosi-token", "jwt");
});

afterEach(() => vi.unstubAllGlobals());

describe("invitations for you", () => {
  it("lists them by workspace and role; accepting joins and opens that workspace", async () => {
    backend({ invitations: PENDING });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: "Invitations for you" });
    expect(within(list).getByText("Beta")).toBeInTheDocument();
    expect(within(list).getByText(/as manager/i)).toBeInTheDocument();
    // Nothing is joined until the person says so.
    expect(calls("/api/app/invitations/mine/inv-1/accept", "POST")).toHaveLength(0);
    await user.click(within(list).getByRole("button", { name: /accept the invitation to beta/i }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(localStorage.getItem("dosi-workspace")).toBe(BETA);
    const [, init] = calls("/api/app/invitations/mine/inv-1/accept", "POST")[0];
    expect((init?.headers as Record<string, string>)["X-Workspace-Id"]).toBeUndefined();
  });

  it("declining removes it and joins nothing", async () => {
    backend({ invitations: PENDING });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: "Invitations for you" });
    await user.click(within(list).getByRole("button", { name: /decline the invitation to beta/i }));
    await waitFor(() => expect(screen.queryByRole("list", { name: "Invitations for you" })).not.toBeInTheDocument());
    expect(calls("/api/app/invitations/mine/inv-1/decline", "POST")).toHaveLength(1);
    expect(assign).not.toHaveBeenCalled();
  });

  it("an invitation that expired meanwhile drops out with the reason", async () => {
    backend({ invitations: PENDING, accept: { body: { error: { code: "Tracker:InvitationExpired", message: "gone" } }, status: 410 } });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces", withToasts: true });
    const list = await screen.findByRole("list", { name: "Invitations for you" });
    await user.click(within(list).getByRole("button", { name: /accept the invitation to beta/i }));
    expect(await screen.findByText(/expired or was withdrawn/i)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("list", { name: "Invitations for you" })).not.toBeInTheDocument());
    expect(assign).not.toHaveBeenCalled();
  });

  it("an unconfirmed email explains the empty list and sends the confirmation again", async () => {
    backend({ invitations: { emailConfirmed: false, items: [] } });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    expect(await screen.findByText(/confirm your email address to see invitations/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /email me a confirmation link/i }));
    expect(await screen.findByText(/we emailed you a confirmation link/i)).toBeInTheDocument();
    expect(calls("/api/app/my-account/send-email-confirmation", "POST")).toHaveLength(1);
  });

  it("says so when the server sends no email", async () => {
    backend({ invitations: { emailConfirmed: false, items: [] }, confirmation: { alreadyConfirmed: false, emailSent: false } });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    await user.click(await screen.findByRole("button", { name: /email me a confirmation link/i }));
    expect(await screen.findByText(/this server doesn't send email/i)).toBeInTheDocument();
  });

  it("shows invitations above 'create your first workspace' when the person has none yet", async () => {
    backend({ mine: [], invitations: PENDING });
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    expect(await screen.findByRole("heading", { name: /create your first workspace/i })).toBeInTheDocument();
    expect(await screen.findByRole("list", { name: "Invitations for you" })).toBeInTheDocument();
  });

  it("stays out of the way on an older server without the endpoint", async () => {
    backend({});
    fetchMock.mockImplementationOnce(async () => json([{ id: ACME, name: "Acme", role: "owner" }]));
    const original = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) =>
      String(input).endsWith("/api/app/invitations/mine") ? json({}, 404) : original(input, init),
    );
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    await screen.findByRole("list", { name: /your workspaces/i });
    await waitFor(() => expect(calls("/api/app/invitations/mine")).toHaveLength(1));
    expect(screen.queryByText(/invitations for you/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/couldn't check for invitations/i)).not.toBeInTheDocument();
  });

  it("a11y: invitations list has no violations", async () => {
    backend({ invitations: PENDING });
    const { container } = renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    await screen.findByRole("list", { name: "Invitations for you" });
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("deleting a workspace (owners)", () => {
  it("unlocks only when the name is typed, then removes it for good and says when its data goes", async () => {
    backend();
    localStorage.setItem("dosi-workspace", ACME);
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces", withToasts: true });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    await user.click(within(list).getByRole("button", { name: /^delete$/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete Acme?" });
    expect(dialog).toHaveTextContent(/members lose access and their desktop agents stop/i);
    const confirm = within(dialog).getByRole("button", { name: /delete workspace/i });
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByLabelText(/type acme to confirm/i), "acm");
    expect(confirm).toBeDisabled();
    await user.type(within(dialog).getByLabelText(/type acme to confirm/i), "e ");
    expect(confirm).toBeEnabled();
    await user.click(confirm);

    await waitFor(() => expect(calls(`/api/app/workspaces/${ACME}/delete`, "POST")).toHaveLength(1));
    const [, init] = calls(`/api/app/workspaces/${ACME}/delete`, "POST")[0];
    expect(JSON.parse(String(init?.body))).toEqual({ confirmationName: "acme" });
    expect(await screen.findByText(/erased for good on nov 2, 2026/i)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
    expect(screen.queryByText("Acme")).not.toBeInTheDocument();
  });

  it("keeps the dialog open with the server's reason when it is refused", async () => {
    backend({ remove: { body: { error: { code: "Tracker:WorkspaceDeleteConfirmationMismatch", message: "mismatch" } }, status: 400 } });
    const user = userEvent.setup();
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    await user.click(within(list).getByRole("button", { name: /^delete$/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete Acme?" });
    await user.type(within(dialog).getByLabelText(/type acme to confirm/i), "Acme");
    await user.click(within(dialog).getByRole("button", { name: /delete workspace/i }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/type the workspace's name exactly/i);
  });

  it("is offered to owners only", async () => {
    backend({ mine: [{ id: BETA, name: "Beta", role: "member" }] });
    renderWithProviders(<WorkspacesPage />, { route: "/workspaces" });
    const list = await screen.findByRole("list", { name: /your workspaces/i });
    expect(within(list).queryByRole("button", { name: /^delete$/i })).not.toBeInTheDocument();
  });
});
