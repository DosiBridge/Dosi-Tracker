// LIVE-mode contract of the report screens: a signed-in workspace sees only
// real rows or honest loading/error/empty states — never the demo dataset —
// filters reach the backend as query params, and payroll prices each project
// at that project's own rate. The session and the HTTP getter are mocked; the
// tenant-data live bindings are installed for real, exactly as the session
// provider does after sign-in.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { installLiveDataset, resetTenantDataForTests } from "@/lib/tenant-data";
import { buildProject, buildUser, resetFactories } from "@/test/factories";
import type { Role } from "@/lib/types";

const session = vi.hoisted(() => ({
  isLive: true,
  user: { id: "u-owner", role: "owner" as Role },
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
import { FilterBar } from "./report-shell";

const ALL_REPORTS = [
  ["Payroll", PayrollReport],
  ["Productivity", ProductivityReport],
  ["Time & Activity", TimeActivityReport],
  ["Weekly", WeeklyReport],
  ["Attendance", AttendanceReport],
  ["Apps", AppsReport],
  ["Projects", ProjectsReport],
] as const;

/** Answer getApi by path (+ "#<ProjectId>" when the query carries one); anything else is a test failure. */
function serve(table: Record<string, unknown>) {
  api.getApi.mockImplementation(async (url: string) => {
    const [path, query = ""] = url.split("?");
    const projectId = new URLSearchParams(query).get("ProjectId");
    const key = projectId ? `${path}#${projectId}` : path;
    if (key in table) return table[key];
    throw new Error(`unexpected request ${url}`);
  });
}

const calledUrls = () => api.getApi.mock.calls.map((c) => String(c[0]));

function installRoster() {
  installLiveDataset({
    workspaceId: "live-test",
    users: [
      buildUser({ id: "u-owner", name: "Rina Das", role: "owner" }),
      buildUser({ id: "u-worker", name: "Omar Faruk", role: "worker" }),
    ],
    projects: [buildProject({ id: "p1", title: "Apollo" })],
    activities: [],
    now: new Date(),
  });
}

beforeEach(() => {
  localStorage.clear();
  resetTenantDataForTests();
  resetFactories();
  api.getApi.mockReset();
  session.isLive = true;
  session.user = { id: "u-owner", role: "owner" };
  localStorage.setItem("dosi-token", "test-token");
  installRoster();
});

describe("every report", () => {
  it.each(ALL_REPORTS)("%s: a failed live load shows the error notice and no demo people or projects", async (_name, Report) => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Service Unavailable"));
    render(<Report />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't load this report");
    const text = document.body.textContent ?? "";
    expect(text).not.toMatch(/Ayesha Rahman|Tanvir Hasan|demo data|NaN/);
  });

  it.each(ALL_REPORTS)("%s: demo mode renders the seeded dataset without touching the network", (_name, Report) => {
    session.isLive = false;
    localStorage.clear();
    resetTenantDataForTests();
    render(<Report />);

    expect(api.getApi).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.body.textContent).not.toMatch(/NaN/);
  });
});

describe("Payroll (live)", () => {
  const payrollTable = {
    "/api/app/reporting/summary": {
      perUser: [{ userId: "u-owner", trackedMinutes: 180 }],
      perProject: [
        { projectId: "p1", trackedMinutes: 120 },
        { projectId: "p2", trackedMinutes: 60 },
      ],
    },
    "/api/app/reporting/summary#p1": { perUser: [{ userId: "u-owner", trackedMinutes: 120 }] },
    "/api/app/reporting/summary#p2": { perUser: [{ userId: "u-owner", trackedMinutes: 60 }] },
    "/api/app/team/project-members/p1": [{ projectId: "p1", userId: "u-owner", role: "Admin", hourlyRate: 50 }],
    "/api/app/team/project-members/p2": [{ projectId: "p2", userId: "u-owner", role: "Admin", hourlyRate: 20 }],
  };

  it("pays each project's hours at that project's rate and shows the blended rate", async () => {
    serve(payrollTable);
    render(<PayrollReport />);

    // 2h × $50 + 1h × $20 = $120 over 3h → $40/hr blended (max-rate × hours would be $150).
    expect((await screen.findAllByText("$40/hr")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("$120").length).toBeGreaterThan(0);
    expect(screen.queryByText("$150")).toBeNull();
    expect(screen.getAllByText("Rina Das").length).toBeGreaterThan(0);
    expect(screen.queryByText("Ayesha Rahman")).toBeNull();

    // Memberships are read with the PATH parameter the backend serves.
    expect(calledUrls()).toContain("/api/app/team/project-members/p1");
    expect(calledUrls().some((u) => u.includes("project-members?"))).toBe(false);
  });

  it("sends the selected member to the backend as UserId", async () => {
    serve(payrollTable);
    const user = userEvent.setup();
    render(<PayrollReport />);
    await screen.findAllByText("$40/hr");

    await user.selectOptions(screen.getByDisplayValue("All members"), "u-owner");
    await waitFor(() => expect(calledUrls().some((u) => u.includes("UserId=u-owner"))).toBe(true));
  });

  it("shows an honest error with Retry — zeros, not demo rows", async () => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Internal Server Error"));
    const user = userEvent.setup();
    render(<PayrollReport />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load this report");
    expect(screen.queryByText(/demo data/i)).toBeNull();
    expect(screen.queryByText("Ayesha Rahman")).toBeNull();
    expect(screen.getAllByText("$0").length).toBeGreaterThan(0);

    const before = api.getApi.mock.calls.length;
    await user.click(within(alert).getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(api.getApi.mock.calls.length).toBeGreaterThan(before));
  });
});

describe("Productivity (live)", () => {
  it("never renders NaN% when there is no tracked time", async () => {
    serve({ "/api/app/reporting/summary": { perUser: [] } });
    render(<ProductivityReport />);

    await waitFor(() => expect(screen.queryByText("Loading report data…")).toBeNull());
    expect(api.getApi).toHaveBeenCalled();
    // (Twice: DataTable renders its empty text in both the mobile stack and the table.)
    expect(screen.getAllByText("No tracked time in this range.").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/NaN/);
    // No invented unproductive/idle split for real people.
    expect(screen.queryByText("Unproductive")).toBeNull();
  });
});

describe("Time & Activity (live)", () => {
  it("applies the project filter server-side", async () => {
    serve({
      "/api/app/reporting/summary": { totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: 0, perUser: [] },
      "/api/app/reporting/daily-series": [],
      "/api/app/reporting/summary#p1": { totalActivities: 0, totalTrackedMinutes: 0, averageProductivity: 0, perUser: [] },
      "/api/app/reporting/daily-series#p1": [],
    });
    const user = userEvent.setup();
    render(<TimeActivityReport />);
    await screen.findAllByText("No tracked time for these filters.");

    await user.selectOptions(screen.getByDisplayValue("All projects"), "p1");
    await waitFor(() => {
      const urls = calledUrls();
      expect(urls.some((u) => u.startsWith("/api/app/reporting/summary?") && u.includes("ProjectId=p1"))).toBe(true);
      expect(urls.some((u) => u.startsWith("/api/app/reporting/daily-series?") && u.includes("ProjectId=p1"))).toBe(true);
    });
    expect(screen.queryByText(/Screenshots/)).toBeNull();
  });
});

describe("FilterBar member options", () => {
  const options = () =>
    within(screen.getByDisplayValue("All members"))
      .getAllByRole("option")
      .map((o) => o.textContent);

  it("offers a live manager the real roster", () => {
    render(<FilterBar rangeKey="7d" onRange={() => {}} memberId="all" onMember={() => {}} />);
    expect(options()).toEqual(["All members", "Rina Das", "Omar Faruk"]);
  });

  it("offers a live worker only themselves", () => {
    session.user = { id: "u-worker", role: "worker" };
    render(<FilterBar rangeKey="7d" onRange={() => {}} memberId="all" onMember={() => {}} />);
    expect(options()).toEqual(["All members", "Omar Faruk"]);
  });
});
