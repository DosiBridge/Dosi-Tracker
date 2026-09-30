// LIVE MODE end-to-end through the real providers: signed in with a backend
// token, every surface shows ONLY the tenant's real data or an honest empty /
// error state — never the demo people, companies, cards or notifications.
// Only global fetch is stubbed (a small in-memory backend below).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import DashboardLayout from "@/app/(dashboard)/layout";
import TeamPage from "@/app/(dashboard)/team/page";
import TimesheetPage from "@/app/(dashboard)/timesheet/page";
import BillingPage from "@/app/(dashboard)/billing/page";
import { Topbar } from "@/components/layout/topbar";
import { SetupChecklist } from "@/components/dashboard/setup-checklist";
import { AdminDashboard } from "@/components/dashboard/admin-dashboard";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

const API = process.env.NEXT_PUBLIC_API_URL || "https://localhost:44342";
const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: status < 300 ? "OK" : "Server Error", json: async () => JSON.parse(text), text: async () => text } as Response;
}

interface BackendOptions {
  members?: unknown[];
  failNotifications?: boolean;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

function backend(opts: BackendOptions = {}) {
  const members = opts.members ?? [
    { userId: OWNER, userName: "mira", name: "Mira", surname: "Okafor", email: "mira@realco.test", isActive: true, isOwner: true, isManager: true, projectIds: [PROJECT] },
    { userId: WORKER, userName: "sam", name: "Sam", surname: "Lee", email: "sam@realco.test", isActive: true, isOwner: false, isManager: false, projectIds: [PROJECT] },
  ];
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: OWNER, tenantId: "t-1", userName: "mira", name: "Mira", surName: "Okafor", email: "mira@realco.test", roles: ["admin"] },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } },
      });
    }
    if (url.startsWith("/api/app/team/members")) return json({ items: members });
    if (url.startsWith("/api/app/team/project-members/")) return json({ items: [{ projectId: PROJECT, userId: WORKER, role: "Worker", hourlyRate: 20 }] });
    if (url.startsWith("/api/app/team/member") && method === "PUT") return json({ projectId: PROJECT, userId: WORKER, role: "Admin", hourlyRate: 35 });
    if (url.startsWith("/api/app/project")) return json({ items: [{ id: PROJECT, title: "Apollo", isArchived: false }] });
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }, { id: "plan-b", name: "Business", pricePerUser: 12, maxSeats: 100 }] });
    if (url.startsWith("/api/app/billing/invoices")) return json({ items: [{ id: "inv-1", amount: 18, status: "paid", dueDate: "2026-09-01T00:00:00Z", creationTime: "2026-08-18T00:00:00Z" }] });
    if (url.startsWith("/api/app/notification/my-notifications")) {
      if (opts.failNotifications) return json({}, 500);
      return json({ items: [{ id: "n-1", message: "You were added to Apollo", isRead: false, creationTime: "2026-09-22T10:00:00Z" }] });
    }
    if (url.startsWith("/api/app/notification/mark-as-read/")) return json(null);
    if (url.startsWith("/api/app/reporting/user-daily-series")) {
      return json([
        { userId: OWNER, userName: "mira", date: "2026-01-05T00:00:00Z", trackedMinutes: 60, activityCount: 1 },
        { userId: WORKER, userName: "sam", date: "2026-01-05T00:00:00Z", trackedMinutes: 90, activityCount: 2 },
      ]);
    }
    if (url.startsWith("/api/app/reporting/")) return json({ totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: 0, perUser: [], perProject: [] });
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
  localStorage.setItem("dosi-token", "jwt-live");
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const DEMO_NAMES = ["Ayesha Rahman", "Tanvir Hasan", "Dosi Labs", "Acme Studio", "Nimbus Co"];
function expectNoDemoData() {
  for (const name of DEMO_NAMES) expect(screen.queryByText(name)).not.toBeInTheDocument();
}

describe("live mode — shell", () => {
  it("the dashboard layout waits for the real identity, then shows only the real tenant", async () => {
    backend();
    renderWithProviders(
      <DashboardLayout>
        <div>page body</div>
      </DashboardLayout>,
      { route: "/dashboard" },
    );
    expect(screen.getByText(/loading your workspace/i)).toBeInTheDocument();
    expect(await screen.findByText("page body")).toBeInTheDocument();
    expect(screen.getAllByText("RealCo").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Mira Okafor").length).toBeGreaterThan(0);
    expectNoDemoData();
  });

  it("shows an error with retry — not demo data — when the backend is down", async () => {
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"));
    renderWithProviders(
      <DashboardLayout>
        <div>page body</div>
      </DashboardLayout>,
      { route: "/dashboard" },
    );
    expect(await screen.findByText(/can't reach dosi-tracker/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /try again/i })).toBeInTheDocument();
    expect(screen.queryByText("page body")).not.toBeInTheDocument();
    expectNoDemoData();
  });
});

describe("live mode — topbar notifications", () => {
  it("lists the user's real notifications and marks one read on the backend", async () => {
    backend();
    renderWithProviders(<Topbar onMenuClick={() => {}} />);
    const bell = await screen.findByRole("button", { name: "Notifications" });
    await waitFor(() => expect(within(bell).getByText("1")).toBeInTheDocument());

    fireEvent.click(bell);
    fireEvent.click(screen.getByText("You were added to Apollo"));

    await waitFor(() =>
      expect(fetchMock.mock.calls.some(([u, i]) => String(u).endsWith("/api/app/notification/mark-as-read/n-1") && i?.method === "POST")).toBe(true),
    );
    expect(within(bell).queryByText("1")).not.toBeInTheDocument();
    expect(screen.queryByText("Weekly report ready")).not.toBeInTheDocument(); // demo feed never shows
  });

  it("an unreachable notifications API reads as an error, not demo alerts", async () => {
    backend({ failNotifications: true });
    renderWithProviders(<Topbar onMenuClick={() => {}} />);
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).includes("my-notifications"))).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    expect(await screen.findByText(/couldn't load notifications/i)).toBeInTheDocument();
    expect(screen.queryByText("Low activity detected")).not.toBeInTheDocument();
  });
});

describe("live mode — team", () => {
  it("renders the real roster with backend roles and real seat usage, and edits a member's role/rate", async () => {
    backend();
    const user = userEvent.setup();
    renderWithProviders(<TeamPage />, { withToasts: true });

    expect(await screen.findByText("Sam Lee")).toBeInTheDocument();
    expect(screen.getByText("Mira Okafor")).toBeInTheDocument();
    expect(screen.getByText(/2 of 25 seats used/)).toBeInTheDocument(); // backend plan, not the demo table
    expectNoDemoData();

    await user.click(screen.getByRole("button", { name: "Edit Sam Lee" }));
    const dialog = await screen.findByRole("dialog");
    const rate = within(dialog).getByLabelText(/hourly rate/i);
    await waitFor(() => expect(rate).toHaveValue(20)); // prefilled from the project membership
    await user.selectOptions(within(dialog).getByLabelText(/^role$/i), "admin");
    await user.clear(rate);
    await user.type(rate, "35");
    await user.click(within(dialog).getByRole("button", { name: /save changes/i }));

    await waitFor(() => {
      const put = fetchMock.mock.calls.find(([, i]) => i?.method === "PUT");
      expect(put).toBeDefined();
      expect(String(put![0])).toBe(`${API}/api/app/team/member?projectId=${PROJECT}&userId=${WORKER}`);
      expect(JSON.parse(String(put![1]!.body))).toEqual({ role: "Admin", hourlyRate: 35 });
    });
    expect(await screen.findByText("Member updated")).toBeInTheDocument();
  });

  it("a failed roster load shows an error, never the demo team", async () => {
    backend();
    const base = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) =>
      String(input).includes("/api/app/team/members") && !String(input).includes("project") ? json({}, 500) : base(input, init),
    );
    renderWithProviders(<TeamPage />);
    expect(await screen.findByText(/couldn't load your team/i)).toBeInTheDocument();
    expectNoDemoData();
  });
});

describe("live mode — timesheet", () => {
  it("gives every member their own row from user-daily-series (not the tenant summed under one name)", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-01-07T12:00:00Z")); // a Wednesday; week starts Mon 2026-01-05
    backend();
    renderWithProviders(<TimesheetPage />);

    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).includes("/api/app/reporting/user-daily-series"))).toBe(true));
    const table = await screen.findByRole("table");
    await waitFor(() => expect(within(table).getByText("Sam Lee")).toBeInTheDocument());
    const samRow = within(table).getByText("Sam Lee").closest("tr")!;
    const miraRow = within(table).getByText("Mira Okafor").closest("tr")!;
    expect(within(samRow).getAllByText("1h 30m").length).toBeGreaterThan(0);
    expect(within(miraRow).getAllByText("1h").length).toBeGreaterThan(0);
    expect(screen.getByText("Timesheet")).toBeInTheDocument(); // the team view, not "My timesheet"
  });
});

describe("live mode — billing", () => {
  it("shows the real plan and invoices, no fake card, and an honest payments notice", async () => {
    backend();
    renderWithProviders(<BillingPage />);

    await waitFor(() => expect(screen.getAllByText("Starter").length).toBeGreaterThan(0));
    expect((await screen.findAllByText("$18")).length).toBeGreaterThan(0); // the real invoice (card + table views)
    expect(screen.queryByText(/4242/)).not.toBeInTheDocument();
    expect(screen.getByText("No payment method on file.")).toBeInTheDocument();
    expect(screen.getAllByText(/online payments are coming soon/i).length).toBeGreaterThan(0);
    expect(screen.queryByText("Enterprise")).not.toBeInTheDocument(); // demo plan table not shown
    expect(document.body.textContent).not.toMatch(/[a-z0-9-]\.dositracker\.app/); // no invented tenant subdomain
    expect(document.body.textContent).not.toContain("Demo mode");
  });
});

describe("live mode — dashboard + onboarding", () => {
  it("a brand-new tenant sees honest empty states and an unticked 'Invite your team'", async () => {
    backend({
      members: [{ userId: OWNER, userName: "mira", name: "Mira", surname: "Okafor", isActive: true, isOwner: true, isManager: true, projectIds: [] }],
    });
    renderWithProviders(
      <>
        <SetupChecklist isOwner />
        <AdminDashboard userName="Mira" isOwner />
      </>,
    );

    expect((await screen.findAllByText(/no activity yet — install the desktop agent/i)).length).toBeGreaterThan(0);
    expect(screen.getByText("No one has tracked time today yet.")).toBeInTheDocument();
    expect(screen.getByText("No app usage recorded this week.")).toBeInTheDocument();
    expectNoDemoData();

    const invite = screen.getByText("Invite your team");
    expect(invite).not.toHaveClass("line-through"); // a workspace of one hasn't invited anyone
    for (const link of screen.getAllByRole("link", { name: /get the agent/i })) {
      expect(link).toHaveAttribute("href", "/download");
    }
  });
});
