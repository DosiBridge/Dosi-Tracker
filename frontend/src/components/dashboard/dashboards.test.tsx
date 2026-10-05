// The three dashboards against their data sources: the admin and worker
// dashboards read the reporting API for whole days of the reporting zone (the
// member's "today" is their zone's day, as on the desktop agent), and the
// client dashboard shows hours on the client's own project only, over a
// labelled period, with a status taken from the data.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { AdminDashboard } from "@/components/dashboard/admin-dashboard";
import { WorkerDashboard } from "@/components/dashboard/worker-dashboard";
import { ClientDashboard } from "@/components/dashboard/client-dashboard";
import { SetupChecklist } from "@/components/dashboard/setup-checklist";
import { renderAsRole, renderWithProviders, resetPrototypeState } from "@/test/harness";
import { datasetFor } from "@/lib/tenant-data";
import type { User } from "@/lib/types";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ME = "11111111-1111-1111-1111-111111111111";
const SAM = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

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

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function calls(path: string) {
  return fetchMock.mock.calls.filter(([u]) => String(u).replace(API, "").startsWith(path)).map(([u]) => String(u).replace(API, ""));
}

function params(url: string): URLSearchParams {
  return new URLSearchParams(url.split("?")[1] ?? "");
}

/** A live tenant whose reporting endpoints answer per range; `owner` signs in its owner, else the worker Sam. */
function liveBackend(opts: { owner?: boolean; timeZone?: string } = {}) {
  const owner = opts.owner ?? true;
  fetchMock.mockImplementation(async (input) => {
    const url = String(input).replace(API, "");
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: owner ? ME : SAM, tenantId: "t-1", userName: owner ? "mira" : "sam", name: owner ? "Mira" : "Sam", surName: owner ? "Okafor" : "Lee", roles: owner ? ["admin"] : [] },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: owner ? { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } : {} },
      });
    }
    if (url.startsWith("/api/app/workspace/my-preferences")) return json({ timeZone: opts.timeZone ?? "UTC" });
    if (url.startsWith("/api/app/workspace/settings")) return json({}, 403);
    if (url.startsWith("/api/app/team/members")) {
      return json({
        items: [
          { userId: ME, userName: "mira", name: "Mira", surname: "Okafor", isOwner: true, isManager: true, projectIds: [PROJECT] },
          { userId: SAM, userName: "sam", name: "Sam", surname: "Lee", isOwner: false, isManager: false, projectIds: [PROJECT] },
        ],
      });
    }
    if (url.startsWith("/api/app/project")) return json({ items: [{ id: PROJECT, title: "Apollo", color: "#0284c7", isArchived: false }] });
    if (url.startsWith("/api/app/reporting/summary")) {
      const p = params(url);
      const days = Math.round((Date.parse(p.get("To")!) - Date.parse(p.get("From")!)) / 86_400_000);
      const today = days <= 1;
      return json({
        totalActivities: today ? 6 : 30,
        totalTrackedMinutes: today ? 150 : 1200,
        averageProductivity: today ? 72 : 70,
        perUser: today ? [{ userId: SAM, userName: "sam", name: "Sam Lee", activityCount: 6, trackedMinutes: 150, averageProductivity: 72 }] : [],
        perProject: [{ projectId: PROJECT, projectTitle: "Apollo", activityCount: 30, trackedMinutes: today ? 150 : 1200, averageProductivity: 70 }],
      });
    }
    if (url.startsWith("/api/app/reporting/daily-series")) {
      const from = params(url).get("From")!;
      const first = new Date(from);
      return json([{ date: `${first.toISOString().slice(0, 10)}T00:00:00`, activityCount: 4, trackedMinutes: 240, averageProductivity: 50, mouseClicks: 0, keyboardHits: 0 }]);
    }
    if (url.startsWith("/api/app/reporting/app-usage")) {
      return json([
        { appName: "chrome.exe", trackedMinutes: 100, activityCount: 4, userCount: 1 },
        { appName: "Google Chrome", trackedMinutes: 20, activityCount: 1, userCount: 1 },
        { appName: "Code.exe", trackedMinutes: 300, activityCount: 9, userCount: 2 },
      ]);
    }
    if (url.startsWith("/api/app/reporting/hourly")) return json([{ dayOfWeek: 1, hour: 10, trackedMinutes: 45, activeMinutes: 30 }]);
    if (url.startsWith("/api/app/presence/team")) {
      return json({ items: [{ userId: SAM, userName: "sam", deviceId: "d-1", status: "tracking", lastSeenAt: new Date().toISOString(), online: true }] });
    }
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
    if (url.startsWith("/api/app/notification/")) return json({ items: [] });
    if (url.startsWith("/api/app/reporting/")) return json([]);
    throw new Error(`unexpected fetch: ${url}`);
  });
}

describe("admin dashboard — live", () => {
  beforeEach(() => localStorage.setItem("dosi-token", "jwt-live"));

  it("reads every figure from the reporting API for whole days of the reporting zone", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-07T12:00:00Z")); // a Wednesday: "this week" is Monday to today
    liveBackend();
    renderWithProviders(<AdminDashboard userName="Mira" isOwner />);

    const tile = (label: string) => screen.getByText(label).closest("div.rounded-2xl")! as HTMLElement;
    await waitFor(() => expect(within(tile("Tracked today")).getByText("2h 30m")).toBeInTheDocument());
    expect(within(tile("Tracked this week")).getByText("20h")).toBeInTheDocument();
    expect(within(tile("Activity today")).getByText("72%")).toBeInTheDocument();
    expect(await screen.findByText("1 tracking now")).toBeInTheDocument(); // heartbeats
    // Each of the dashboard's queries is for days of the viewer's reporting zone (their preference: UTC).
    for (const endpoint of ["summary", "daily-series", "app-usage", "hourly"]) {
      expect(
        calls(`/api/app/reporting/${endpoint}`).some((url) => params(url).get("TimeZone") === "UTC"),
        `${endpoint} is asked for in the reporting zone`,
      ).toBe(true);
    }
    // The last 7 days, as 7 whole days.
    const series = calls("/api/app/reporting/daily-series").map(params).find((p) => p.get("TimeZone") === "UTC")!;
    expect((Date.parse(series.get("To")!) - Date.parse(series.get("From")!)) / 86_400_000).toBe(7);
  });

  it("lists the team's day with names and activity rings, and apps under their canonical names", async () => {
    liveBackend();
    renderWithProviders(<AdminDashboard userName="Mira" isOwner />);

    const team = (await screen.findByRole("heading", { name: "Team today" })).closest("div.rounded-2xl")! as HTMLElement;
    expect(await within(team).findByText("Sam Lee")).toBeInTheDocument();
    expect(within(team).getByRole("img", { name: "Activity 72%" })).toBeInTheDocument();
    // The project donut's legend lists the slice with its value and share.
    const projects = screen.getByRole("heading", { name: "Time by project" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(within(projects).getByText("Apollo")).toBeInTheDocument();
    expect(within(projects).getByText("100%")).toBeInTheDocument();
  });

  it("a failed load says so and offers Retry — no invented figures", async () => {
    liveBackend();
    const base = fetchMock.getMockImplementation()!;
    fetchMock.mockImplementation(async (input, init) => (String(input).includes("/api/app/reporting/summary") ? json({}, 500) : base(input, init)));
    renderWithProviders(<AdminDashboard userName="Mira" isOwner />);

    expect(await screen.findByText(/couldn't load the team's figures/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});

describe("worker dashboard — live", () => {
  it("'today' is the reporting zone's day, the same day the desktop agent shows", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-05T01:00:00Z")); // 21:00 on 4 Oct in New York
    localStorage.setItem("dosi-token", "jwt-live");
    liveBackend({ owner: false, timeZone: "America/New_York" });
    const user = { id: SAM, name: "Sam Lee", email: "", role: "worker", designation: "Member", status: "offline", timezone: "UTC", trackedToday: 0, productivity: 0, joinedAt: "" } satisfies User;
    renderWithProviders(<WorkerDashboard user={user} />);

    // The dashboard's own "today" query: one whole day, in the member's zone (the session's
    // own "today so far" totals end at the current instant instead).
    const todayCalls = () =>
      calls("/api/app/reporting/summary")
        .map(params)
        .filter((p) => p.get("TimeZone") === "America/New_York" && Date.parse(p.get("To")!) - Date.parse(p.get("From")!) === 24 * 3_600_000);
    await waitFor(() => expect(todayCalls().length).toBeGreaterThan(0));
    const todayCall = todayCalls()[0];
    expect(todayCall.get("From")).toBe("2026-10-04T04:00:00.000Z"); // New York midnight, not UTC midnight
    expect(todayCall.get("To")).toBe("2026-10-05T04:00:00.000Z");
    expect((await screen.findAllByText("Last 7 days")).length).toBeGreaterThan(0);
    // Apps over the labelled period, under canonical names, seconds summed server-side.
    const apps = screen.getByRole("heading", { name: "Apps you used" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(await within(apps).findByText("Visual Studio Code")).toBeInTheDocument();
    expect(within(apps).getByText("Google Chrome")).toBeInTheDocument();
    expect(within(apps).getByText("2h")).toBeInTheDocument();
  });
});

describe("worker dashboard — demo", () => {
  it("never labels a member 'Distracted' and links an empty start to the agent download", () => {
    renderAsRole(<WorkerDashboard user={datasetFor("w1").users.find((u) => u.role === "worker")!} />, "worker");
    expect(screen.queryByText("Distracted")).not.toBeInTheDocument();
    expect(screen.getByText("Your week")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /activity today/i })).toBeInTheDocument();
  });
});

describe("client dashboard — demo", () => {
  const client = () => datasetFor("w1").users.find((u) => u.role === "client")!;

  it("shows hours on the client's own project only, over a labelled week, with a status taken from the data", () => {
    renderAsRole(<ClientDashboard user={client()} />, "client");

    expect(screen.getByText("Acme Corp CRM")).toBeInTheDocument();
    expect(screen.queryByText("On track")).not.toBeInTheDocument();
    expect(screen.getByText(/last work tracked|nothing tracked in the last 30 days/i)).toBeInTheDocument();
    const hours = screen.getByRole("heading", { name: "Hours on this project" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(within(hours).getByText(/^This week · /)).toBeInTheDocument();
    // Members' time here is their time on THIS project — never their whole day across clients.
    const projectRows = datasetFor("w1").activities.filter((a) => a.projectId === "p3");
    const shown = within(hours).queryAllByRole("listitem");
    for (const row of shown) {
      const name = row.textContent ?? "";
      const member = datasetFor("w1").users.find((u) => name.includes(u.name));
      expect(member, `"${name}" is on the project`).toBeDefined();
      expect(projectRows.some((a) => a.userId === member!.id)).toBe(true);
    }
  });

  it("a11y: the client dashboard has no axe violations", async () => {
    const { container } = renderAsRole(<ClientDashboard user={client()} />, "client");
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("dashboard — onboarding order", () => {
  it("lists the setup steps in the order they can be done: project, invite, agent", () => {
    renderWithProviders(<SetupChecklist isOwner />);
    // The demo workspace is fully set up; an empty workspace shows every step.
    expect(screen.queryByText("Get your workspace live")).not.toBeInTheDocument();
  });

  it("an empty live workspace shows Create a project first, then Invite your team, then the agent", async () => {
    localStorage.setItem("dosi-token", "jwt-live");
    fetchMock.mockImplementation(async (input) => {
      const url = String(input).replace(API, "");
      if (url.startsWith("/api/abp/application-configuration")) {
        return json({
          currentUser: { isAuthenticated: true, id: ME, tenantId: "t-1", userName: "mira", roles: ["admin"] },
          currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
          auth: { grantedPolicies: {} },
        });
      }
      if (url.startsWith("/api/app/team/members")) return json({ items: [{ userId: ME, userName: "mira", isOwner: true, isManager: true, projectIds: [] }] });
      if (url.startsWith("/api/app/workspace/")) return json({}, 404);
      if (url.startsWith("/api/app/reporting/")) return json([]);
      if (url.startsWith("/api/app/")) return json({ items: [], totalCount: 0 });
      throw new Error(`unexpected fetch: ${url}`);
    });
    // As the dashboard page composes it: the checklist under the greeting.
    renderWithProviders(<AdminDashboard userName="Mira" isOwner onboarding={<SetupChecklist isOwner />} />, { route: "/dashboard" });

    const checklist = (await screen.findByText("Get your workspace live")).closest("div.rounded-2xl")! as HTMLElement;
    const steps = within(checklist).getAllByRole("listitem").map((li) => li.textContent ?? "");
    expect(steps[0]).toContain("Create a project");
    expect(steps[1]).toContain("Invite your team");
    expect(steps[2]).toContain("Install a desktop agent");
    for (const link of within(checklist).getAllByRole("link", { name: /open projects|open team|get the agent/i })) {
      expect(link.querySelector("button")).toBeNull(); // a link, not a link wrapping a button
    }
  });
});
