// TeamPage invite behavior in demo mode: who gets the invite affordance and
// exactly which roles each inviter may assign — verified against the
// inviteableRoles matrix in src/lib/roles.ts, not against a copy of it.
//
// Email-format validation is enforced before anything is sent (in BOTH demo
// and live paths) — the "rejects malformed addresses" tests below were written
// against the unvalidated implementation, observed failing, and now pin the fix.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import TeamPage from "@/app/(dashboard)/team/page";
import { renderAsRole, resetPrototypeState } from "@/test/harness";
import { inviteableRoles } from "@/lib/roles";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetPrototypeState();
  // Demo mode (no dosi-token) must never touch the network; a never-settling
  // stub makes any accidental request hang the assertion instead of passing.
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openInviteModal(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: /invite member/i }));
  return screen.getByRole("dialog");
}

/** The invite dialog has two selects (Role, Project); the Role one is the one offering "Member". */
function roleSelect(dialog: HTMLElement): HTMLSelectElement {
  const match = within(dialog)
    .getAllByRole("combobox")
    .find((s) => within(s).queryByRole("option", { name: "Member" }));
  if (!(match instanceof HTMLSelectElement)) throw new Error("role select not found in invite dialog");
  return match;
}

describe("team page — invite permissions", () => {
  it("owner can invite and is offered exactly worker/admin/client (never owner)", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    const options = within(roleSelect(dialog)).getAllByRole("option");

    expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual(
      inviteableRoles({ role: "owner" }), // ["worker", "admin", "client"]
    );
    // Named as roleLabels names them everywhere else in the app.
    expect(options.map((o) => o.textContent?.trim())).toEqual(["Member", "Administrator", "Client"]);
    expect(within(dialog).queryByRole("option", { name: /owner/i })).not.toBeInTheDocument();
  });

  it("admin can invite but is never offered the Admin or Owner role", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "admin");

    const dialog = await openInviteModal(user);
    const options = within(roleSelect(dialog)).getAllByRole("option");

    expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual(
      inviteableRoles({ role: "admin" }), // ["worker", "client"]
    );
    expect(within(dialog).queryByRole("option", { name: /admin/i })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("option", { name: /owner/i })).not.toBeInTheDocument();
  });

  it("a worker gets no invite affordance at all", () => {
    renderAsRole(<TeamPage />, "worker");
    expect(screen.queryByRole("button", { name: /invite member/i })).not.toBeInTheDocument();
  });

  it("a client gets no invite affordance at all", () => {
    renderAsRole(<TeamPage />, "client");
    expect(screen.queryByRole("button", { name: /invite member/i })).not.toBeInTheDocument();
  });
});

describe("team page — sending invites (demo mode)", () => {
  it("send stays disabled until at least one email is entered", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    expect(within(dialog).getByRole("button", { name: /send invites/i })).toBeDisabled();
  });

  it("submitting emails shows the sent confirmation without hitting the network", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByPlaceholderText("jane@company.com, john@company.com"),
      "jane@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await screen.findByText("Invitations sent!")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects a malformed address, names the offender, and sends nothing", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "not-an-email",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await within(dialog).findByText(/not a valid email address: not-an-email/i)).toBeInTheDocument();
    expect(screen.queryByText("Invitations sent!")).not.toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects a batch when only one address of several is malformed", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "jane@company.dev, broken@, john@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await within(dialog).findByText(/broken@/)).toBeInTheDocument();
    expect(screen.queryByText("Invitations sent!")).not.toBeInTheDocument();
  });

  it("accepts a comma-separated batch of valid addresses", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    await user.type(
      within(dialog).getByLabelText(/email addresses/i),
      "jane@company.dev, john@company.dev",
    );
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await screen.findByText("Invitations sent!")).toBeInTheDocument();
  });

  it("labels every invite field for assistive tech", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openInviteModal(user);
    expect(within(dialog).getByLabelText(/email addresses/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^role$/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^project$/i)).toBeInTheDocument();
  });
});

/* ============================================================================
 * LIVE MODE — the Team page against a small in-memory backend: every project
 * in the invite dialog, sign-in details after an invite, the owner/manager
 * rules on editing a membership, removing someone from a project or the
 * workspace, a new one-time password, adding an existing person to a
 * project, live status from heartbeats, and the Devices view.
 * ========================================================================== */

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const OWNER = "11111111-1111-1111-1111-111111111111";
const SAM = "22222222-2222-2222-2222-222222222222";
const LEE = "33333333-3333-3333-3333-333333333333";
/** Twelve projects: more than ABP's default page of ten. Newest first, like the API. */
const PROJECTS = Array.from({ length: 12 }, (_, i) => ({
  id: `p-${String(i + 1).padStart(2, "0")}`,
  title: `Project ${String.fromCharCode(76 - i)}`, // "Project L" … "Project A"
  isArchived: i === 3,
}));

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

interface LiveOptions {
  /** Identity roles of the signed-in user: ["admin"] = workspace owner; [] with Team.Manage = manager. */
  roles?: string[];
  inviteError?: unknown;
  /** Fail the next project-members read for this project id. */
  failMembersOnce?: string;
}

function liveBackend(opts: LiveOptions = {}) {
  let failMembers = opts.failMembersOnce ?? null;
  const roles = opts.roles ?? ["admin"];
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: OWNER, tenantId: "t-1", userName: "mira", name: "Mira", surName: "Okafor", email: "mira@realco.test", roles },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } },
      });
    }
    if (url.startsWith("/api/app/team/members")) {
      return json({
        items: [
          { userId: OWNER, userName: "mira", name: "Mira", surname: "Okafor", email: "mira@realco.test", isActive: true, isOwner: roles.includes("admin"), isManager: true, projectIds: ["p-01"] },
          { userId: SAM, userName: "sam", name: "Sam", surname: "Lee", email: "sam@realco.test", isActive: true, isOwner: false, isManager: false, projectIds: ["p-01", "p-02"] },
          { userId: LEE, userName: "lee", name: "Lee", surname: "Park", email: "lee@realco.test", isActive: true, isOwner: false, isManager: false, projectIds: [] },
        ],
      });
    }
    if (url.startsWith("/api/app/team/project-members/")) {
      const projectId = decodeURIComponent(url.split("/").pop() ?? "");
      if (failMembers === projectId) {
        failMembers = null;
        return json({}, 500);
      }
      return json({
        items: [
          { id: `m-${projectId}-sam`, projectId, userId: SAM, role: "Worker", hourlyRate: 20, userName: "sam", name: "Sam", surname: "Lee" },
          { id: `m-${projectId}-mira`, projectId, userId: OWNER, role: "Admin", hourlyRate: 0, userName: "mira" },
        ],
      });
    }
    if (url.startsWith("/api/app/team/invite-member") && method === "POST") {
      if (opts.inviteError) return json(opts.inviteError, 409);
      const email = JSON.parse(String(init?.body)).email as string;
      if (email.endsWith("@known.test")) {
        return json({ added: true, workspaceRole: "member", member: { id: "m-new", projectId: "p-01", userId: "u-new", role: "Worker", hourlyRate: 0 } });
      }
      return json({ added: false, workspaceRole: "member", invitationId: "inv-new", invitePath: "/invite/tok-new", expiresAt: "2026-10-15T00:00:00Z" });
    }
    if (url.startsWith("/api/app/team/invitations")) {
      return json({ items: [{ id: "inv-1", email: "pending@realco.test", role: "member", expiresAt: "2026-10-20T00:00:00Z", creationTime: "2026-10-06T00:00:00Z" }] });
    }
    if (url.startsWith("/api/app/team/inv-1/invitation") && method === "DELETE") return json(null, 204);
    if (url.startsWith("/api/app/team/workspace-role") && method === "PUT") return json({ userId: SAM, role: "manager" });
    if (url.startsWith("/api/app/team/from-workspace") && method === "DELETE") return json(null, 204);
    if (url.startsWith("/api/app/team/member/") && method === "DELETE") return json(null, 204);
    if (url.startsWith("/api/app/team/member") && method === "PUT") return json({ projectId: "p-01", userId: SAM, role: "Worker", hourlyRate: 20 });
    if (url.startsWith("/api/app/team/member") && method === "POST") return json({ id: "m-added", projectId: "p-10", userId: LEE, role: "Worker", hourlyRate: 30 });
    if (url.startsWith("/api/app/project")) return json({ items: PROJECTS, totalCount: PROJECTS.length });
    if (url.startsWith("/api/app/presence/team")) {
      return json({
        items: [
          {
            userId: SAM,
            userName: "sam",
            name: "Sam Lee",
            deviceId: "d-sam",
            deviceName: "SAM-PC",
            platform: "windows",
            osVersion: "Windows 11 Pro 23H2",
            agentVersion: "0.2.0",
            status: "tracking",
            lastSeenAt: new Date().toISOString(),
            online: true,
          },
        ],
      });
    }
    if (url.startsWith("/api/app/reporting/devices")) {
      return json([
        { userId: SAM, userName: "sam", deviceId: "d-sam", deviceName: "SAM-PC", platform: "windows", osVersion: "Windows 11 Pro 23H2", agentVersion: "0.2.0", lastActivityAt: new Date().toISOString(), trackedMinutes: 600 },
        {
          userId: LEE,
          userName: "lee",
          deviceId: "d-lee",
          deviceName: "Lee's MacBook",
          platform: "macos",
          osVersion: "macOS 14.5 (23F79)",
          agentVersion: "0.1.9",
          lastActivityAt: new Date(Date.now() - 3 * 86_400_000).toISOString(),
          trackedMinutes: 90,
        },
      ]);
    }
    if (url.startsWith("/api/app/reporting/")) return json({ totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: null, perUser: [], perProject: [] });
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 2 }] });
    if (url.startsWith("/api/app/workspace/")) return json({}, 404);
    if (url.startsWith("/api/app/notification/")) return json({ items: [] });
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

function calls(path: string, method: string) {
  return fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").startsWith(path) && (i?.method ?? "GET") === method);
}

async function openManage(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(await screen.findByRole("button", { name: `Edit ${name}` }));
  return screen.getByRole("dialog", { name: `Edit ${name}` });
}

describe("team page — live roster", () => {
  beforeEach(() => localStorage.setItem("dosi-token", "jwt-live"));

  it("shows every member's name, email and role with the app's role names", async () => {
    liveBackend();
    renderAsRole(<TeamPage />, "owner");

    const sam = (await screen.findByRole("heading", { name: "Sam Lee" })).closest("li")!;
    expect(within(sam).getByText("sam@realco.test")).toBeInTheDocument();
    expect(within(sam).getByText("Member")).toBeInTheDocument();
    const mira = screen.getByRole("heading", { name: "Mira Okafor" }).closest("li")!;
    expect(within(mira).getByText("Owner")).toBeInTheDocument();
    // A live roster can't say who is a client, so that filter is not offered.
    expect(screen.queryByRole("tab", { name: "Clients" })).not.toBeInTheDocument();
  });

  it("takes live status from the agents' heartbeats", async () => {
    liveBackend();
    renderAsRole(<TeamPage />, "owner");

    expect(await screen.findByText(/1 tracking now/)).toBeInTheDocument();
  });

  it("keeps Invite available at the seat limit — the server decides, and its answer is shown", async () => {
    liveBackend({ inviteError: { error: { code: "Tracker:SeatLimitReached", message: "Seat limit reached." } } });
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    await screen.findByRole("heading", { name: "Sam Lee" });
    await waitFor(() => expect(screen.getByText(/limit reached/i)).toBeInTheDocument()); // 2 of 2 seats
    const invite = screen.getByRole("button", { name: /invite member/i });
    expect(invite).toBeEnabled();

    await user.click(invite);
    const dialog = screen.getByRole("dialog", { name: "Invite members" });
    await user.type(within(dialog).getByLabelText(/email addresses/i), "new@realco.test");
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    expect(await within(dialog).findByText(/your plan has no free seat/i)).toBeInTheDocument();
  });
});

describe("team page — live invite", () => {
  beforeEach(() => localStorage.setItem("dosi-token", "jwt-live"));

  it("offers every active project (not ABP's first page of 10), sorted by title", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    await screen.findByRole("heading", { name: "Sam Lee" });

    await waitFor(() => expect(calls("/api/app/project?MaxResultCount=1000", "GET").length).toBeGreaterThan(0));
    await user.click(screen.getByRole("button", { name: /invite member/i }));
    const dialog = screen.getByRole("dialog", { name: "Invite members" });
    await waitFor(() => expect(within(within(dialog).getByLabelText(/^project$/i)).getAllByRole("option")).toHaveLength(12));
    const [none, ...options] = within(within(dialog).getByLabelText(/^project$/i))
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(none).toMatch(/no project/i);

    const expected = PROJECTS.filter((p) => !p.isArchived)
      .map((p) => p.title)
      .sort((a, b) => a.localeCompare(b));
    expect(options).toEqual(expected);
  });

  it("someone without an account gets a copyable invitation link with its expiry — never a password", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    await screen.findByRole("heading", { name: "Sam Lee" });
    await waitFor(() => expect(calls("/api/app/project?MaxResultCount=1000", "GET").length).toBeGreaterThan(0));

    await user.click(screen.getByRole("button", { name: /invite member/i }));
    const dialog = screen.getByRole("dialog", { name: "Invite members" });
    await user.type(within(dialog).getByLabelText(/email addresses/i), "new@realco.test");
    await user.type(within(dialog).getByLabelText(/hourly rate/i), "25");
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));

    const link = await within(dialog).findByTestId("invite-link");
    expect(link).toHaveTextContent(`${window.location.origin}/invite/tok-new`);
    expect(within(dialog).getByText(/expires/i)).toBeInTheDocument();
    expect(within(dialog).getByText("Invited")).toBeInTheDocument();
    expect(within(dialog).queryByText(/password/i)).not.toBeInTheDocument();

    const body = JSON.parse(String(calls("/api/app/team/invite-member", "POST")[0][1]?.body));
    expect(body).toEqual({ projectId: "p-12", email: "new@realco.test", role: "Worker", workspaceRole: "member", hourlyRate: 25 });

    await user.click(within(dialog).getByRole("button", { name: /copy/i }));
    expect(await navigator.clipboard.readText()).toBe(`${window.location.origin}/invite/tok-new`);
  });

  it("someone who already has an account is added at once", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    await screen.findByRole("heading", { name: "Sam Lee" });
    await user.click(screen.getByRole("button", { name: /invite member/i }));
    const dialog = screen.getByRole("dialog", { name: "Invite members" });
    await user.type(within(dialog).getByLabelText(/email addresses/i), "ana@known.test");
    await user.click(within(dialog).getByRole("button", { name: /send invites/i }));
    expect(await within(dialog).findByText("Added")).toBeInTheDocument();
    expect(within(dialog).queryByTestId("invite-link")).not.toBeInTheDocument();
  });

  it("lists pending invitations and revokes one after confirming", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    const section = (await screen.findByRole("heading", { name: /pending invitations/i })).closest("section")!;
    expect(within(section).getByText("pending@realco.test")).toBeInTheDocument();
    await user.click(within(section).getByRole("button", { name: /revoke invitation for pending@realco.test/i }));
    expect(calls("/api/app/team/inv-1/invitation", "DELETE")).toHaveLength(0);
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: /revoke invitation/i }));
    await waitFor(() => expect(calls("/api/app/team/inv-1/invitation", "DELETE")).toHaveLength(1));
  });

  it("a manager can't set hourly rates or invite administrators", async () => {
    liveBackend({ roles: [] });
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "admin");
    await screen.findByRole("heading", { name: "Sam Lee" });

    await user.click(screen.getByRole("button", { name: /invite member/i }));
    const dialog = screen.getByRole("dialog", { name: "Invite members" });
    expect(within(dialog).queryByLabelText(/hourly rate/i)).not.toBeInTheDocument();
    const roleOptions = within(within(dialog).getByLabelText(/^role$/i))
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(roleOptions).not.toContain("Administrator");
  });
});

describe("team page — live member management", () => {
  beforeEach(() => localStorage.setItem("dosi-token", "jwt-live"));

  it("a manager sees the rate but can't change it, and can't grant Administrator", async () => {
    liveBackend({ roles: [] });
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "admin");

    const dialog = await openManage(user, "Sam Lee");
    const rate = within(dialog).getByLabelText(/hourly rate/i);
    await waitFor(() => expect(rate).toHaveValue(20));
    expect(rate).toBeDisabled();
    expect(within(dialog).getByText(/only the workspace owner changes rates/i)).toBeInTheDocument();
    const roles = within(within(dialog).getByLabelText(/^role$/i))
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(roles).toEqual(["Member", "Client"]);

    await user.selectOptions(within(dialog).getByLabelText(/^role$/i), "client");
    await user.click(within(dialog).getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(calls("/api/app/team/member?", "PUT")).toHaveLength(1));
    // The unchanged rate travels back as it was: a manager never alters pay.
    expect(JSON.parse(String(calls("/api/app/team/member?", "PUT")[0][1]?.body))).toEqual({ role: "Client", hourlyRate: 20 });
  });

  it("a failed prefill shows an error with Retry and keeps Save off — a read failure is never saved as rate 0", async () => {
    // The dialog opens on the member's first project by title: Project K (p-02).
    liveBackend({ failMembersOnce: "p-02" });
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openManage(user, "Sam Lee");
    expect(await within(dialog).findByText(/couldn't load their current role and rate/i)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /save changes/i })).toBeDisabled();

    await user.click(within(dialog).getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(within(dialog).getByLabelText(/hourly rate/i)).toHaveValue(20));
    expect(within(dialog).getByRole("button", { name: /save changes/i })).toBeEnabled();
  });

  it("removes a member from one project after confirming, by the membership's id", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner", { withToasts: true });

    const dialog = await openManage(user, "Sam Lee");
    await waitFor(() => expect(within(dialog).getByLabelText(/hourly rate/i)).toHaveValue(20));
    await user.click(within(dialog).getByRole("button", { name: /remove from this project/i }));

    const confirm = screen.getByRole("dialog", { name: /remove sam lee from project k\?/i });
    expect(within(confirm).getByText(/time they already tracked stays in reports/i)).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Remove from project" }));

    await waitFor(() => expect(calls("/api/app/team/member/m-p-02-sam", "DELETE")).toHaveLength(1));
    expect(await screen.findByText("Sam Lee removed from Project K")).toBeInTheDocument();
  });

  it("removes a member from the workspace after confirming", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner", { withToasts: true });

    const dialog = await openManage(user, "Sam Lee");
    await user.click(within(dialog).getByRole("button", { name: /^remove…$/i }));
    const confirm = screen.getByRole("dialog", { name: /remove sam lee from the workspace\?/i });
    expect(within(confirm).getByText(/their seat is freed/i)).toBeInTheDocument();
    expect(within(confirm).getByText(/they keep their account; they just lose access to this workspace/i)).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Remove from workspace" }));

    await waitFor(() => expect(calls(`/api/app/team/from-workspace?userId=${SAM}`, "DELETE")).toHaveLength(1));
    expect(await screen.findByText("Sam Lee removed from the workspace")).toBeInTheDocument();
  });

  it("the owner changes a member's workspace role; no password reset is offered", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const dialog = await openManage(user, "Sam Lee");
    expect(within(dialog).queryByRole("button", { name: /reset/i })).not.toBeInTheDocument();
    const select = within(dialog).getByLabelText(/workspace role/i);
    expect(within(select).getAllByRole("option").map((o) => o.textContent)).toEqual(["Owner", "Manager", "Member"]);
    await user.selectOptions(select, "manager");
    await user.click(within(dialog).getByRole("button", { name: /change role/i }));
    await waitFor(() => expect(calls(`/api/app/team/workspace-role?userId=${SAM}&role=manager`, "PUT")).toHaveLength(1));
  });

  it("a manager can't change workspace roles", async () => {
    liveBackend({ roles: [] });
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "admin");
    const dialog = await openManage(user, "Sam Lee");
    expect(within(dialog).queryByLabelText(/workspace role/i)).not.toBeInTheDocument();
  });

  it("adds an existing person to a project they're not on, from their card", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner", { withToasts: true });

    const lee = (await screen.findByRole("heading", { name: "Lee Park" })).closest("li")!;
    await waitFor(() => expect(calls("/api/app/project?MaxResultCount=1000", "GET").length).toBeGreaterThan(0));
    await user.click(within(lee).getByRole("button", { name: /add to project/i }));
    const dialog = screen.getByRole("dialog", { name: "Add to a project" });
    expect(within(dialog).getByRole("radio", { name: /lee park/i })).toBeChecked();

    await user.selectOptions(within(dialog).getByLabelText(/^project$/i), "p-10");
    await user.type(within(dialog).getByLabelText(/hourly rate/i), "30");
    await user.click(within(dialog).getByRole("button", { name: /^add to project$/i }));

    await waitFor(() => expect(calls("/api/app/team/member", "POST")).toHaveLength(1));
    expect(JSON.parse(String(calls("/api/app/team/member", "POST")[0][1]?.body))).toEqual({
      projectId: "p-10",
      userId: LEE,
      role: "Worker",
      hourlyRate: 30,
    });
    expect(await screen.findByText(/lee park added to project c/i)).toBeInTheDocument();
  });

  it("won't offer a project the person is already on", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    const sam = (await screen.findByRole("heading", { name: "Sam Lee" })).closest("li")!;
    await waitFor(() => expect(calls("/api/app/project?MaxResultCount=1000", "GET").length).toBeGreaterThan(0));
    await user.click(within(sam).getByRole("button", { name: /add to project/i }));
    const dialog = screen.getByRole("dialog", { name: "Add to a project" });
    const projectL = within(within(dialog).getByLabelText(/^project$/i)).getByRole("option", { name: /project l — already a member/i });
    expect(projectL).toBeDisabled();
  });
});

describe("team page — devices", () => {
  it("lists live devices with OS, agent version (older ones flagged), status and last seen", async () => {
    localStorage.setItem("dosi-token", "jwt-live");
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    await screen.findByRole("heading", { name: "Sam Lee" });

    await user.click(screen.getByRole("tab", { name: /devices/i }));
    const table = await screen.findByRole("table");
    await waitFor(() => expect(within(table).getByRole("rowheader", { name: "Lee Park" })).toBeInTheDocument());
    const samRow = within(table).getByRole("rowheader", { name: "Sam Lee" }).closest("tr")!;
    expect(within(samRow).getByText("SAM-PC")).toBeInTheDocument();
    expect(within(samRow).getByText("Windows 11 Pro 23H2")).toBeInTheDocument();
    expect(within(samRow).getByText("Tracking")).toBeInTheDocument();
    expect(within(samRow).queryByText("Update available")).not.toBeInTheDocument();

    const leeRow = within(table).getByRole("rowheader", { name: "Lee Park" }).closest("tr")!;
    expect(within(leeRow).getByText("0.1.9")).toBeInTheDocument();
    expect(within(leeRow).getByText("Update available")).toBeInTheDocument();
    expect(within(leeRow).getByText(/no live status · last upload 3d ago/i)).toBeInTheDocument();
  });

  it("the demo workspace lists its members' devices without touching the network", async () => {
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");

    await user.click(screen.getByRole("tab", { name: /devices/i }));
    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("rowheader").length).toBeGreaterThan(1);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("a11y: the devices view has no axe violations", async () => {
    const user = userEvent.setup();
    const { container } = renderAsRole(<TeamPage />, "owner");
    await user.click(screen.getByRole("tab", { name: /devices/i }));
    await screen.findByRole("table");
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("team page — accessibility", () => {
  it("a11y: the roster has no axe violations", async () => {
    const { container } = renderAsRole(<TeamPage />, "owner");
    expect(await axe(container)).toHaveNoViolations();
  });

  it("a11y: the member dialog has no axe violations", async () => {
    localStorage.setItem("dosi-token", "jwt-live");
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<TeamPage />, "owner");
    const dialog = await openManage(user, "Sam Lee");
    await waitFor(() => expect(within(dialog).getByLabelText(/hourly rate/i)).toHaveValue(20));
    expect(await axe(dialog)).toHaveNoViolations();
  });
});
