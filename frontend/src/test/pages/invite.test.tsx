// /invite/[token]: the invitation as the person opening the link sees it —
// signed in with the invited email (Accept), signed in as someone else
// (explain + sign out), or signed out (sign in / create an account).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { InviteView } from "@/components/invite-view";
import { resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const WS = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
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

function backend(opts: {
  info?: unknown;
  infoStatus?: number;
  email?: string | null;
  accept?: { body: unknown; status?: number };
  decline?: { body: unknown; status?: number };
} = {}) {
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url === "/api/app/invitations/tok-1" && method === "GET") {
      return json(opts.info ?? { workspaceName: "RealCo", email: "sam@realco.test", role: "member", expired: false }, opts.infoStatus ?? 200);
    }
    if (url === "/api/app/invitations/tok-1/accept" && method === "POST") {
      return json(opts.accept?.body ?? { workspaceId: WS, workspaceName: "RealCo", role: "member" }, opts.accept?.status ?? 200);
    }
    if (url === "/api/app/invitations/tok-1/decline" && method === "POST") {
      return json(opts.decline?.body ?? {}, opts.decline?.status ?? 200);
    }
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({ currentUser: { isAuthenticated: true, id: "u1", email: opts.email ?? "sam@realco.test", roles: [] }, auth: { grantedPolicies: {} } });
    }
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  assign = vi.fn();
  vi.stubGlobal("location", { ...window.location, pathname: "/invite/tok-1", origin: "http://localhost", assign });
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("invite page", () => {
  it("signed out: shows the workspace and email, and offers sign in (returning here) or a new account", async () => {
    backend();
    render(<InviteView token="tok-1" />);
    expect(await screen.findByRole("heading", { name: /join realco/i })).toBeInTheDocument();
    expect(screen.getByTestId("invite-email")).toHaveTextContent("sam@realco.test");
    expect(screen.getByRole("link", { name: /sign in to accept/i })).toHaveAttribute(
      "href",
      `/login?next=${encodeURIComponent("/invite/tok-1")}&email=${encodeURIComponent("sam@realco.test")}`,
    );
    expect(screen.getByRole("link", { name: /create an account/i })).toHaveAttribute(
      "href",
      `/login?mode=signup&invite=tok-1&email=${encodeURIComponent("sam@realco.test")}`,
    );
    // Anonymous: no token, no workspace header.
    const [, init] = fetchMock.mock.calls[0];
    expect((init?.headers as Record<string, string>).Authorization).toBeUndefined();
  });

  it("signed in with the invited email: Accept joins and opens that workspace", async () => {
    localStorage.setItem("dosi-token", "jwt");
    backend();
    const user = userEvent.setup();
    render(<InviteView token="tok-1" />);
    await user.click(await screen.findByRole("button", { name: /accept invitation/i }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/dashboard"));
    expect(localStorage.getItem("dosi-workspace")).toBe(WS);
  });

  it("signed in with the invited email: Decline says no, joins nothing and points back to the chooser", async () => {
    localStorage.setItem("dosi-token", "jwt");
    backend();
    const user = userEvent.setup();
    render(<InviteView token="tok-1" />);
    await user.click(await screen.findByRole("button", { name: /^decline$/i }));
    expect(await screen.findByRole("status")).toHaveTextContent(/you declined the invitation/i);
    expect(screen.getByRole("link", { name: /go to your workspaces/i })).toHaveAttribute("href", "/workspaces");
    expect(fetchMock.mock.calls.some(([u, i]) => String(u).endsWith("/tok-1/accept") && i?.method === "POST")).toBe(false);
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
  });

  it("a refused decline (already used) is explained", async () => {
    localStorage.setItem("dosi-token", "jwt");
    backend({ decline: { body: { error: { code: "Tracker:InvitationExpired", message: "gone" } }, status: 410 } });
    const user = userEvent.setup();
    render(<InviteView token="tok-1" />);
    await user.click(await screen.findByRole("button", { name: /^decline$/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/no longer valid/i);
  });

  it("signed in as someone else: explains the mismatch and offers to sign out", async () => {
    localStorage.setItem("dosi-token", "jwt");
    backend({ email: "other@x.test" });
    const user = userEvent.setup();
    render(<InviteView token="tok-1" />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/signed in as other@x\.test/i);
    expect(alert).toHaveTextContent(/sam@realco\.test/);
    expect(screen.queryByRole("button", { name: /accept/i })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /sign out/i }));
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(assign).toHaveBeenCalledWith("/invite/tok-1");
  });

  it("an expired invitation says so and offers nothing to accept", async () => {
    backend({ info: { workspaceName: "RealCo", email: "sam@realco.test", role: "member", expired: true } });
    render(<InviteView token="tok-1" />);
    expect(await screen.findByRole("alert")).toHaveTextContent(/expired/i);
    expect(screen.queryByRole("link", { name: /sign in/i })).not.toBeInTheDocument();
  });

  it("an unknown token is 'not found'", async () => {
    backend({ info: { error: { code: "Tracker:InvitationNotFound", message: "nope" } }, infoStatus: 404 });
    render(<InviteView token="tok-1" />);
    expect(await screen.findByRole("heading", { name: /invitation not found/i })).toBeInTheDocument();
  });

  it("a refused accept (expired meanwhile) is explained", async () => {
    localStorage.setItem("dosi-token", "jwt");
    backend({ accept: { body: { error: { code: "Tracker:InvitationExpired", message: "expired" } }, status: 410 } });
    const user = userEvent.setup();
    render(<InviteView token="tok-1" />);
    await user.click(await screen.findByRole("button", { name: /accept invitation/i }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/expired/i);
    expect(assign).not.toHaveBeenCalled();
  });
});
