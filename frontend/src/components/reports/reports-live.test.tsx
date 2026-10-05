// LIVE-mode contract of the period reports (Time & Activity, Weekly,
// Productivity, Apps, Projects, Attendance, Payroll, Devices): a signed-in
// workspace sees only real rows or honest loading / error / empty states —
// never the demo dataset; every reporting request carries the reporting time
// zone and whole-day bounds; filters reach the backend as query params; pay
// comes from the server's effective-dated payroll. The session and the HTTP
// getter are mocked; the tenant-data live bindings are installed for real,
// exactly as the session provider does after sign-in. (The reports hub's
// tests sit beside its page: components may not import pages.)
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { installLiveDataset, resetTenantDataForTests } from "@/lib/tenant-data";
import { clearStoredChoices } from "@/hooks/use-stored-choice";
import { buildProject, buildUser, resetFactories } from "@/test/factories";
import type { Role } from "@/lib/types";

const session = vi.hoisted(() => ({
  isLive: true,
  user: { id: "u-owner", role: "owner" as Role, name: "Rina Das" },
}));
const api = vi.hoisted(() => ({ getApi: vi.fn() }));

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/components/session-provider", () => ({ useSession: () => session }));
vi.mock("@/hooks/useApi", () => ({ getApi: api.getApi }));

import { PayrollReport } from "./payroll-report";
import { ProductivityReport } from "./productivity-report";
import { TimeActivityReport } from "./time-activity-report";
import { WeeklyReport } from "./weekly-report";
import { AttendanceReport } from "./attendance-report";
import { AppsReport } from "./apps-report";
import { ProjectsReport } from "./projects-report";
import { DevicesReport } from "./devices-report";

const REPORTS = [
  ["Time & Activity", TimeActivityReport],
  ["Weekly", WeeklyReport],
  ["Productivity", ProductivityReport],
  ["Apps", AppsReport],
  ["Projects", ProjectsReport],
  ["Attendance", AttendanceReport],
  ["Payroll", PayrollReport],
  ["Devices", DevicesReport],
] as const;

/** Wednesday 15 July 2026, 15:00 in Dhaka (the viewer's reporting zone below). */
const NOW = new Date("2026-07-15T09:00:00.000Z");

const ALL_POLICIES = { "Tracker.Activities.ViewAll": true, "Tracker.Team.Manage": true };

/** What every live report reads besides its figures: the reporting zone and the viewer's permissions. */
function base(policies: Record<string, boolean> = ALL_POLICIES): Record<string, unknown> {
  return {
    "/api/app/workspace/my-preferences": { timeZone: "Asia/Dhaka" },
    "/api/app/workspace/settings": { defaultTimeZone: null },
    "/api/abp/application-configuration": { auth: { grantedPolicies: policies } },
  };
}

/** Answer getApi by path (+ "#<ProjectId>" when the query carries one); an Error rejects; anything else is a test failure. */
function serve(table: Record<string, unknown>) {
  api.getApi.mockImplementation(async (url: string) => {
    const [path, query = ""] = url.split("?");
    const projectId = new URLSearchParams(query).get("ProjectId");
    const key = projectId && `${path}#${projectId}` in table ? `${path}#${projectId}` : path;
    if (!(key in table)) throw new Error(`unexpected request ${url}`);
    const value = table[key];
    if (value instanceof Error) throw value;
    return value;
  });
}

const calledUrls = () => api.getApi.mock.calls.map((c) => String(c[0]));
const reportingCalls = () => calledUrls().filter((u) => u.startsWith("/api/app/reporting/"));
const params = (url: string) => new URLSearchParams(url.split("?")[1] ?? "");

function installRoster() {
  installLiveDataset({
    workspaceId: "live-test",
    users: [
      buildUser({ id: "u-owner", name: "Rina Das", role: "owner" }),
      buildUser({ id: "u-worker", name: "Omar Faruk", role: "worker" }),
    ],
    projects: [
      buildProject({ id: "p1", title: "Apollo", memberIds: ["u-owner", "u-worker"] }),
      buildProject({ id: "p-old", title: "Legacy", archived: true, memberIds: ["u-owner"] }),
    ],
    activities: [],
    now: new Date(),
  });
}

const emptySummary = { totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: null, perUser: [], perProject: [], perUserProject: [] };

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  localStorage.clear();
  clearStoredChoices();
  resetTenantDataForTests();
  resetFactories();
  api.getApi.mockReset();
  session.isLive = true;
  session.user = { id: "u-owner", role: "owner", name: "Rina Das" };
  localStorage.setItem("dosi-token", "test-token");
  installRoster();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("every report", () => {
  it.each(REPORTS)("%s: a failed live load shows the error notice and no demo people or projects", async (_name, Report) => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Service Unavailable"));
    render(<Report />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't load this report");
    const text = document.body.textContent ?? "";
    expect(text).not.toMatch(/Ayesha Rahman|Tanvir Hasan|demo data|NaN/);
    // Nothing to export from a failed load.
    expect(screen.getByRole("button", { name: "Export" })).toBeDisabled();
  });

  it.each(REPORTS)("%s: demo mode renders the seeded dataset without touching the network", (_name, Report) => {
    session.isLive = false;
    localStorage.clear();
    resetTenantDataForTests();
    render(<Report />);

    expect(api.getApi).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.body.textContent).not.toMatch(/NaN/);
    // The shared period toolbar, with its real custom range (no fake "Custom" preset).
    expect(screen.getByRole("group", { name: "Period" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Custom" })).toHaveAttribute("aria-haspopup", "dialog");
  });

  it.each(REPORTS)("%s: every reporting request carries the reporting zone and whole local days", async (_name, Report) => {
    serve({
      ...base(),
      "/api/app/reporting/summary": emptySummary,
      "/api/app/reporting/daily-series": [],
      "/api/app/reporting/user-daily-series": [],
      "/api/app/reporting/app-usage": [],
      "/api/app/reporting/attendance": { days: [], rows: [] },
      "/api/app/reporting/payroll": { rows: [], totalMinutes: 0, totalAmount: 0 },
      "/api/app/reporting/devices": [],
      "/api/app/presence/team": [],
      "/api/app/project": [],
    });
    render(<Report />);

    await waitFor(() => expect(reportingCalls().length).toBeGreaterThan(0));
    for (const url of reportingCalls()) {
      const qs = params(url);
      expect(qs.get("TimeZone"), url).toBe("Asia/Dhaka");
      // Dhaka midnights: a period never starts or ends mid-day.
      expect(qs.get("From"), url).toMatch(/T18:00:00\.000Z$/);
      expect(qs.get("To"), url).toBe("2026-07-15T18:00:00.000Z");
    }
    // The zone is named once, in the toolbar.
    expect(screen.getAllByText("Dhaka (UTC+6)")).toHaveLength(1);
  });
});

describe("Payroll (live)", () => {
  const payroll = {
    rows: [
      { userId: "u-owner", userName: "rina", projectId: "p1", projectTitle: "Apollo", activityCount: 8, trackedMinutes: 120, amount: 100, hourlyRate: 50 },
      { userId: "u-owner", userName: "rina", projectId: "p-old", projectTitle: "Legacy", activityCount: 4, trackedMinutes: 60, amount: 20.5, hourlyRate: 20.5 },
      { userId: "u-worker", userName: "omar", projectId: "p1", projectTitle: "Apollo", activityCount: 6, trackedMinutes: 90, amount: 45.25, hourlyRate: 30.1667 },
    ],
    totalMinutes: 270,
    totalAmount: 165.75,
  };

  it("pays from the server's effective-dated lines, in dollars and cents, with totals that add up", async () => {
    serve({ ...base(), "/api/app/reporting/payroll": payroll });
    render(<PayrollReport />);

    expect((await screen.findAllByText("$165.75")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("$120.50").length).toBeGreaterThan(0); // Rina: $100.00 + $20.50
    expect(screen.getAllByText("$45.25").length).toBeGreaterThan(0);
    expect(screen.getAllByText("$40.17/hr").length).toBeGreaterThan(0); // $120.50 over 3 paid hours
    expect(screen.getAllByText("Rina Das").length).toBeGreaterThan(0);
    // No invented billable/internal split, and no per-project request fan-out.
    expect(screen.queryByText(/Billable|Internal/)).toBeNull();
    expect(calledUrls().some((u) => u.includes("/reporting/summary") || u.includes("project-members"))).toBe(false);
  });

  it("prices at current rates on a server without the payroll endpoint — and says so", async () => {
    serve({
      ...base(),
      "/api/app/reporting/payroll": Object.assign(new Error("API GET Error: Not Found"), { status: 404 }),
      "/api/app/reporting/summary": { ...emptySummary, perProject: [{ projectId: "p1", projectTitle: "Apollo", trackedMinutes: 180, activityCount: 9 }] },
      "/api/app/reporting/summary#p1": { ...emptySummary, perUser: [{ userId: "u-owner", trackedMinutes: 120 }, { userId: "u-worker", trackedMinutes: 60 }] },
      "/api/app/team/project-members/p1": [
        { projectId: "p1", userId: "u-owner", role: "Admin", hourlyRate: 50 },
        { projectId: "p1", userId: "u-worker", role: "Worker", hourlyRate: 0 },
      ],
    });
    render(<PayrollReport />);

    expect(await screen.findByText(/these amounts use each member's current rate/)).toBeInTheDocument();
    expect(screen.getAllByText("$100.00").length).toBeGreaterThan(0);
    // Omar's hour has no rate: listed, flagged, not paid.
    expect(screen.getAllByText("1h without a rate").length).toBeGreaterThan(0);
    expect(calledUrls()).toContain("/api/app/team/project-members/p1");
  });

  it("sends the selected member and project to the server", async () => {
    serve({ ...base(), "/api/app/reporting/payroll": payroll });
    const user = userEvent.setup();
    render(<PayrollReport />);
    await screen.findAllByText("$165.75");

    await user.selectOptions(screen.getByRole("combobox", { name: "Member" }), "u-worker");
    await user.selectOptions(screen.getByRole("combobox", { name: "Project" }), "p-old");
    await waitFor(() => {
      const last = params(reportingCalls()[reportingCalls().length - 1]);
      expect([last.get("UserId"), last.get("ProjectId")]).toEqual(["u-worker", "p-old"]);
    });
    // Archived projects stay choosable, labelled.
    expect(within(screen.getByRole("combobox", { name: "Project" })).getByRole("option", { name: "Legacy (archived)" })).toBeInTheDocument();
  });

  it("shows an honest error with Retry — dashes, not $0.00", async () => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Internal Server Error"));
    const user = userEvent.setup();
    render(<PayrollReport />);

    const alert = await screen.findByRole("alert");
    // No amount at all — a failed load must not read "Total payable $0.00".
    expect(screen.queryByText(/\$\d/)).toBeNull();
    expect(screen.getAllByText("—", { selector: "dd" }).length).toBe(4);
    const before = api.getApi.mock.calls.length;
    await user.click(within(alert).getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(api.getApi.mock.calls.length).toBeGreaterThan(before));
  });

  it("is closed to viewers who can't manage the team (hourly rates)", async () => {
    session.user = { id: "u-owner", role: "admin", name: "Rina Das" };
    serve({ ...base({ "Tracker.Activities.ViewAll": true }), "/api/app/reporting/payroll": payroll });
    render(<PayrollReport />);

    expect(await screen.findByText("Payroll is for team managers")).toBeInTheDocument();
    expect(calledUrls().some((u) => u.includes("/reporting/payroll"))).toBe(false);
  });
});

describe("Attendance (live)", () => {
  it("lists members who tracked nothing, and counts present + absent over the working days so far", async () => {
    serve({
      ...base(),
      // An older server: no weekend/start fields, and members without time left out.
      "/api/app/reporting/attendance": {
        days: ["2026-07-13T00:00:00Z", "2026-07-14T00:00:00Z", "2026-07-15T00:00:00Z"],
        rows: [{ userId: "u-owner", daysPresent: 2, daysAbsent: 1, workedMinutes: 600, presentDays: ["2026-07-13T00:00:00Z", "2026-07-14T00:00:00Z"] }],
      },
    });
    render(<AttendanceReport />);

    expect(await screen.findByText("2 of 3 present")).toBeInTheDocument();
    expect(screen.getByText("0 of 3 present")).toBeInTheDocument(); // Omar, absent all week
    expect(screen.getByText("33%", { selector: "dd" })).toBeInTheDocument(); // 2 present of 6 member working days
    // The day grid: three working days so far, today marked.
    const grid = screen.getByRole("table", { name: /daily presence/i });
    expect(within(grid).getAllByRole("columnheader").map((th) => th.textContent)).toEqual(["Member", "Mon13", "Tue14", "Today15"]);
    // The scroll box contains its visually hidden labels (no page widening on phones).
    expect(grid.parentElement).toHaveClass("relative");
  });

  it("shows typical start and finish and weekend work from a contract-v2 server", async () => {
    serve({
      ...base(),
      "/api/app/reporting/attendance": {
        days: ["2026-07-13T00:00:00", "2026-07-14T00:00:00", "2026-07-15T00:00:00"],
        rows: [
          {
            userId: "u-owner",
            userName: "rina",
            daysPresent: 3,
            daysAbsent: 0,
            weekendDaysWorked: 1,
            workedMinutes: 1500,
            idleMinutes: 45,
            averageStartMinutes: 545,
            averageEndMinutes: 1030,
            presentDays: ["2026-07-11T00:00:00", "2026-07-13T00:00:00", "2026-07-14T00:00:00", "2026-07-15T00:00:00"],
          },
          { userId: "u-worker", userName: "omar", daysPresent: 0, daysAbsent: 3, weekendDaysWorked: 0, workedMinutes: 0, presentDays: [] },
        ],
      },
    });
    render(<AttendanceReport />);

    expect(await screen.findByText("09:05 – 17:10")).toBeInTheDocument();
    expect(screen.getAllByText("09:05").length).toBeGreaterThan(0);
    expect(screen.getByText("3 of 3 present")).toBeInTheDocument();
  });
});

describe("Apps & Websites (live)", () => {
  const usage = [
    { appName: "chrome.exe", trackedMinutes: 30, activityCount: 3, userCount: 2 },
    { appName: "Chrome.exe", trackedMinutes: 10, activityCount: 1, userCount: 1 },
    { appName: "code.exe", trackedMinutes: 120, activityCount: 6, userCount: 2 },
    { appName: "TextInputHost.exe", trackedMinutes: 0, activityCount: 9, userCount: 2 },
  ];

  it("reads every app under one canonical name and files it by app category", async () => {
    serve({ ...base(), "/api/app/reporting/app-usage": usage });
    const user = userEvent.setup();
    render(<AppsReport />);

    expect((await screen.findAllByText("Visual Studio Code")).length).toBeGreaterThan(0);
    // No raw process name is listed anywhere (the footnote explains the folding in prose).
    expect(screen.queryByText(/^(chrome|code)\.exe$/i)).toBeNull();
    expect(screen.queryAllByText("Google Chrome").length).toBeGreaterThan(0);
    expect(screen.getAllByText("40m").length).toBeGreaterThan(0); // Google Chrome, folded
    expect(screen.getByText("75%", { selector: "dd" })).toBeInTheDocument(); // productive apps: 120 of 160 min
    // Background processes stay out of the way until asked for.
    expect(screen.queryByText("TextInputHost")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show background apps (1)" }));
    expect(screen.getAllByText("TextInputHost").length).toBeGreaterThan(0);
  });

  it("narrows to one member on the server", async () => {
    serve({ ...base(), "/api/app/reporting/app-usage": usage });
    const user = userEvent.setup();
    render(<AppsReport />);
    await screen.findAllByText("Visual Studio Code");

    await user.selectOptions(screen.getByRole("combobox", { name: "Member" }), "u-worker");
    await waitFor(() => expect(reportingCalls().some((u) => params(u).get("UserId") === "u-worker")).toBe(true));
  });
});

describe("Project Breakdown (live)", () => {
  it("includes archived projects with time, labelled, and costs projects from the payroll lines", async () => {
    serve({
      ...base(),
      "/api/app/reporting/summary": {
        ...emptySummary,
        totalTrackedMinutes: 180,
        perProject: [
          { projectId: "p1", projectTitle: "Apollo", trackedMinutes: 120, activityCount: 6, averageProductivity: 80 },
          { projectId: "p-old", projectTitle: "Legacy", isArchived: true, trackedMinutes: 60, activityCount: 3, averageProductivity: 60 },
        ],
      },
      "/api/app/project": [
        { id: "p1", title: "Apollo", color: "#006bff", isArchived: false },
        { id: "p-old", title: "Legacy", isArchived: true },
        { id: "p2", title: "Zephyr", isArchived: false },
      ],
      "/api/app/reporting/payroll": {
        rows: [
          { userId: "u-owner", projectId: "p1", trackedMinutes: 120, amount: 100, hourlyRate: 50 },
          { userId: "u-owner", projectId: "p-old", trackedMinutes: 60, amount: 30, hourlyRate: 30 },
        ],
      },
    });
    render(<ProjectsReport />);

    expect((await screen.findAllByText("$130.00")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Legacy").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Archived").length).toBeGreaterThan(0);
    expect(screen.getAllByText("3h").length).toBeGreaterThan(0); // the archived project's hour counts in the total
    expect(screen.getAllByText("Zephyr").length).toBeGreaterThan(0); // active, no time yet
    // One summary, one payroll: no request per project.
    expect(calledUrls().filter((u) => u.startsWith("/api/app/reporting/summary"))).toHaveLength(1);
    expect(calledUrls().some((u) => u.includes("project-members"))).toBe(false);
  });

  it("shows no money to a viewer who can't see rates", async () => {
    session.user = { id: "u-owner", role: "admin", name: "Rina Das" };
    serve({
      ...base({ "Tracker.Activities.ViewAll": true }),
      "/api/app/reporting/summary": { ...emptySummary, totalTrackedMinutes: 60, perProject: [{ projectId: "p1", trackedMinutes: 60, activityCount: 1 }] },
      "/api/app/project": [{ id: "p1", title: "Apollo" }],
    });
    render(<ProjectsReport />);

    expect(await screen.findByText("needs permission to manage the team")).toBeInTheDocument();
    expect(screen.queryByText(/\$\d/)).toBeNull();
    expect(calledUrls().some((u) => u.includes("/reporting/payroll"))).toBe(false);
  });
});

describe("Time & Activity (live)", () => {
  const summary = {
    totalActivities: 9,
    totalTrackedMinutes: 180,
    totalIdleMinutes: 30,
    averageProductivity: 70,
    perUser: [{ userId: "u-owner", activityCount: 9, trackedMinutes: 180, idleMinutes: 30, averageProductivity: 70 }],
    perProject: [{ projectId: "p1", trackedMinutes: 180, activityCount: 9 }],
    perUserProject: [{ userId: "u-owner", projectId: "p1", trackedMinutes: 180, activityCount: 9, idleMinutes: 30, averageProductivity: 70, amount: 0, hourlyRate: 0 }],
  };

  it("shows idle time and each member's top project", async () => {
    serve({ ...base(), "/api/app/reporting/summary": summary, "/api/app/reporting/daily-series": [{ date: "2026-07-14T00:00:00", trackedMinutes: 180, idleMinutes: 30, averageProductivity: 70, activityCount: 9 }] });
    render(<TimeActivityReport />);

    expect((await screen.findAllByRole("columnheader", { name: /Idle/ })).length).toBeGreaterThan(0);
    expect(screen.getAllByRole("columnheader", { name: /Top project/ }).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Apollo", { selector: "span" }).length).toBeGreaterThan(0);
    expect(screen.getAllByText("30m").length).toBeGreaterThan(0);
    expect(screen.getAllByText("2h 6m").length).toBeGreaterThan(0); // active: 70% of 3h
  });

  it("applies the project filter server-side", async () => {
    serve({ ...base(), "/api/app/reporting/summary": emptySummary, "/api/app/reporting/daily-series": [] });
    const user = userEvent.setup();
    render(<TimeActivityReport />);
    await screen.findByText("No tracked time in this period");

    await user.selectOptions(screen.getByRole("combobox", { name: "Project" }), "p1");
    await waitFor(() => {
      expect(reportingCalls().some((u) => u.startsWith("/api/app/reporting/summary?") && params(u).get("ProjectId") === "p1")).toBe(true);
      expect(reportingCalls().some((u) => u.startsWith("/api/app/reporting/daily-series?") && params(u).get("ProjectId") === "p1")).toBe(true);
    });
  });

  it("exports through the safe CSV writer once loaded — never while loading", async () => {
    let release: (value: unknown) => void = () => {};
    serve({ ...base(), "/api/app/reporting/daily-series": [] });
    const fallback = api.getApi.getMockImplementation()!;
    api.getApi.mockImplementation((url: string) =>
      url.startsWith("/api/app/reporting/summary") ? new Promise((resolve) => (release = resolve)) : fallback(url),
    );
    const blobs: Blob[] = [];
    URL.createObjectURL = vi.fn((blob: Blob) => {
      blobs.push(blob);
      return "blob:csv";
    });
    URL.revokeObjectURL = vi.fn();
    const user = userEvent.setup();
    render(<TimeActivityReport />);

    await waitFor(() => expect(reportingCalls().length).toBeGreaterThan(0));
    expect(screen.getByRole("button", { name: "Export" })).toBeDisabled();
    release(summary);

    const exportButton = screen.getByRole("button", { name: "Export" });
    await waitFor(() => expect(exportButton).toBeEnabled());
    await user.click(exportButton);
    const menu = screen.getByRole("menu", { name: "Export" });
    await user.click(within(menu).getByRole("menuitem", { name: "Download CSV" }));
    expect(blobs).toHaveLength(1);
    // Starts with the UTF-8 byte order mark (Blob.text() would strip it while decoding).
    expect([...new Uint8Array(await blobs[0].arrayBuffer()).slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf]);
    const csv = await blobs[0].text();
    expect(csv).toContain("Rina Das,3:00,3,70,2.1,0.5,9,Apollo");
    expect(csv).toContain("Total,3:00,3,70,2.1,0.5,9,");
  });

  it("a custom range asks for exactly its days, in the reporting zone", async () => {
    serve({ ...base(), "/api/app/reporting/summary": emptySummary, "/api/app/reporting/daily-series": [] });
    const user = userEvent.setup();
    render(<TimeActivityReport />);
    await screen.findByText("No tracked time in this period");

    await user.click(screen.getByRole("button", { name: "Custom" }));
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2026-07-01" } });
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "2026-07-11" } });
    await user.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => {
      const qs = params(reportingCalls()[reportingCalls().length - 1]);
      expect([qs.get("From"), qs.get("To")]).toEqual(["2026-06-30T18:00:00.000Z", "2026-07-11T18:00:00.000Z"]);
    });
    expect(screen.getByText("1 Jul – 11 Jul 2026")).toBeInTheDocument();
  });

  it("offers a viewer the server reports only about (no ViewAll) just themselves", async () => {
    session.user = { id: "u-owner", role: "admin", name: "Rina Das" };
    serve({ ...base({ "Tracker.Team.Manage": true }), "/api/app/reporting/summary": emptySummary, "/api/app/reporting/daily-series": [] });
    render(<TimeActivityReport />);

    await waitFor(() =>
      expect(within(screen.getByRole("combobox", { name: "Member" })).getAllByRole("option").map((o) => o.textContent)).toEqual(["All members", "Rina Das"]),
    );
  });
});

describe("Productivity (live)", () => {
  it("never renders NaN% when there is no tracked time", async () => {
    serve({ ...base(), "/api/app/reporting/summary": emptySummary, "/api/app/reporting/app-usage": [] });
    render(<ProductivityReport />);

    expect(await screen.findByText("No tracked time in this period")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/NaN/);
  });

  it("splits members' time by input, and app time by app category", async () => {
    serve({
      ...base(),
      "/api/app/reporting/summary": {
        ...emptySummary,
        totalTrackedMinutes: 240,
        averageProductivity: 50,
        perUser: [
          { userId: "u-owner", trackedMinutes: 120, idleMinutes: 30, averageProductivity: 75, activityCount: 4 },
          // A Mac agent that couldn't measure input: no activity figure is invented.
          { userId: "u-worker", trackedMinutes: 120, idleMinutes: 0, averageProductivity: null, activityCount: 4 },
        ],
      },
      "/api/app/reporting/app-usage": [
        { appName: "code.exe", trackedMinutes: 90, activityCount: 4, userCount: 1 },
        { appName: "chrome.exe", trackedMinutes: 30, activityCount: 4, userCount: 2 },
      ],
    });
    render(<ProductivityReport />);

    expect(await screen.findByText("75%", { selector: "dd" })).toBeInTheDocument(); // productive apps: 90 of 120 app minutes
    expect(screen.getAllByText("1h 30m").length).toBeGreaterThan(0); // Rina's active time
    expect(screen.getAllByText("· not measured").length).toBeGreaterThan(0); // Omar
    expect(screen.getByRole("list", { name: "App time by category" })).toHaveTextContent("Productive");
  });
});

describe("Weekly Summary (live)", () => {
  it("names the busiest day by its date", async () => {
    serve({
      ...base(),
      "/api/app/reporting/summary": {
        ...emptySummary,
        totalTrackedMinutes: 360,
        perUser: [{ userId: "u-owner", trackedMinutes: 360, averageProductivity: 60, activityCount: 12 }],
      },
      "/api/app/reporting/daily-series": [
        { date: "2026-07-13T00:00:00", trackedMinutes: 60, averageProductivity: 60, activityCount: 2 },
        { date: "2026-07-14T00:00:00", trackedMinutes: 300, averageProductivity: 60, activityCount: 10 },
      ],
      "/api/app/reporting/user-daily-series": [
        { userId: "u-owner", date: "2026-07-13T00:00:00", trackedMinutes: 60, activityCount: 2 },
        { userId: "u-owner", date: "2026-07-14T00:00:00", trackedMinutes: 300, activityCount: 10 },
      ],
    });
    render(<WeeklyReport />);

    expect(await screen.findByText("Tue, 14 Jul")).toBeInTheDocument();
    expect(screen.getByText("5h tracked")).toBeInTheDocument();
    expect(screen.getAllByText("of 3").length).toBeGreaterThan(0); // days worked out of this week's 3 days so far
  });
});

describe("Devices & Agents (live)", () => {
  it("merges the period's devices with live presence and flags outdated agents", async () => {
    serve({
      ...base(),
      "/api/app/reporting/devices": [
        { userId: "u-owner", userName: "rina", deviceId: "D1", deviceName: "RINA-PC", platform: "windows", osVersion: "Windows 11 Pro", agentVersion: "0.3.0", lastActivityAt: "2026-07-15T08:00:00", trackedMinutes: 300 },
        { userId: "u-worker", userName: "omar", deviceId: "D2", deviceName: "Omar's MacBook", platform: "macos", osVersion: "macOS 14.5", agentVersion: "0.2.4", lastActivityAt: "2026-07-14T08:00:00Z", trackedMinutes: 120 },
      ],
      "/api/app/presence/team": [
        { userId: "u-owner", userName: "rina", deviceId: "D1", status: "tracking", lastSeenAt: "2026-07-15T08:59:30Z", online: true, projectTitle: "Apollo" },
        { userId: "u-worker", userName: "omar", deviceId: "D2", status: "idle", lastSeenAt: "2026-07-15T06:00:00Z", online: false },
      ],
    });
    render(<DevicesReport />);

    expect((await screen.findAllByText("RINA-PC")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Tracking").length).toBeGreaterThan(0);
    expect(screen.getAllByText("last seen 3h ago").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Update available").length).toBeGreaterThan(0);
    expect(screen.getByText("1 of 2")).toBeInTheDocument();
    expect(screen.getByText("1 Windows · 1 macOS")).toBeInTheDocument();
  });
});

describe("accessibility", () => {
  it.each(REPORTS)("a11y: %s (demo) has no axe violations", async (_name, Report) => {
    session.isLive = false;
    localStorage.clear();
    resetTenantDataForTests();
    const { container } = render(<Report />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
