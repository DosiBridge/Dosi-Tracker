// "Explain this report" across the reports in a LIVE session (AI assist spec
// §18.5): every report except payroll offers it under its header, and each
// explains exactly the period, filters and reporting zone its own figures
// are read for. Pay figures never reach AI assist. Members don't see it. The
// session and HTTP are mocked; no request reaches an AI provider.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAllPagesApi: vi.fn(),
  getHourlyReport: vi.fn(),
  getDeviceReport: vi.fn(),
  getPayrollReport: vi.fn(),
}));
const ai = vi.hoisted(() => ({ value: null as unknown }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/ai-provider")>()),
  useAiAssistant: () => ai.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentType } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AnalyticsReport } from "@/components/reports/analytics-report";
import { AppsReport } from "@/components/reports/apps-report";
import { AttendanceReport } from "@/components/reports/attendance-report";
import { DevicesReport } from "@/components/reports/devices-report";
import { PayrollReport } from "@/components/reports/payroll-report";
import { ProductivityReport } from "@/components/reports/productivity-report";
import { ProjectsReport } from "@/components/reports/projects-report";
import { TimeActivityReport } from "@/components/reports/time-activity-report";
import { WeeklyReport } from "@/components/reports/weekly-report";
import { WorkReport } from "@/components/reports/work-report";
import { getAllPagesApi, getApi, getDeviceReport, getHourlyReport, getPayrollReport } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import { clearStoredChoices } from "@/hooks/use-stored-choice";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { reportCatalog } from "@/lib/reports-data";
import { aiContext, reportResult } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const ZONE = "Asia/Dhaka";

/** Every report but payroll, by catalog slug, and the endpoint its period's figures come from. */
const EXPLAINED: [string, ComponentType, string][] = [
  ["time-activity", TimeActivityReport, "summary"],
  ["weekly", WeeklyReport, "summary"],
  ["productivity", ProductivityReport, "summary"],
  ["apps-urls", AppsReport, "app-usage"],
  ["projects", ProjectsReport, "summary"],
  ["attendance", AttendanceReport, "attendance"],
  ["devices", DevicesReport, "devices"],
  ["work-report", WorkReport, "summary"],
  ["analytics", AnalyticsReport, "summary"],
];

const pending = () => new Promise<never>(() => {});

beforeEach(() => {
  resetPrototypeState();
  clearStoredChoices();
  localStorage.setItem("dosi-token", "live-token");
  session.current = { isLive: true, user: { id: "u-owner", role: "owner", name: "Rina Das" }, workspace: { id: "ws-live", name: "Real" } };
  ai.value = aiContext({ workspaceId: "ws-live", userId: "u-owner" });
  vi.mocked(getApi).mockReset();
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return { timeZone: ZONE };
    if (endpoint === "/api/app/workspace/settings") return { defaultTimeZone: null };
    if (endpoint.startsWith("/api/abp/application-configuration")) {
      return { auth: { grantedPolicies: { "Tracker.Activities.ViewAll": true, "Tracker.Team.Manage": true } } };
    }
    // The figures themselves never arrive: the explainer doesn't need them.
    return pending();
  });
  for (const fn of [getAllPagesApi, getHourlyReport, getDeviceReport, getPayrollReport]) vi.mocked(fn).mockImplementation(pending);
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(reportResult());
});

/** The first request the report made for its period's figures. */
async function periodQuery(endpoint: string): Promise<URLSearchParams> {
  let found: string | undefined;
  await waitFor(() => {
    found = vi.mocked(getApi).mock.calls.map(([e]) => e).find((e) => e.startsWith(`/api/app/reporting/${endpoint}?`));
    expect(found).toBeDefined();
  });
  return new URLSearchParams(found!.split("?")[1]);
}

describe("Explain this report", () => {
  it("covers every report in the catalog except payroll", () => {
    expect(EXPLAINED.map(([slug]) => slug).sort()).toEqual(
      reportCatalog
        .map((r) => r.slug)
        .filter((slug) => slug !== "payroll")
        .sort(),
    );
  });

  it.each(EXPLAINED)("is offered on %s, for exactly the report's period, filters and zone", async (slug, Report, endpoint) => {
    const user = userEvent.setup();
    render(<Report />);
    const query = await periodQuery(endpoint);
    await user.click(screen.getByRole("button", { name: "Explain this report" }));
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    const [feature, body] = vi.mocked(requestAiFeature).mock.calls[0];
    expect(feature).toBe("report-explanation");
    expect(body).toEqual({
      reportSlug: slug,
      from: query.get("From"),
      to: query.get("To"),
      projectId: null,
      userId: null,
      timeZone: ZONE,
      dayBasis: null,
      regenerate: false,
    });
    expect(await screen.findByText(/This report covers 38 h 12 min/)).toBeInTheDocument();
  });

  it("is never offered on payroll: pay figures never reach AI assist", async () => {
    render(<PayrollReport />);
    expect(await screen.findByRole("heading", { level: 1 })).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("button", { name: "Explain this report" })).toBeNull();
  });

  it("is not offered to a member", async () => {
    session.current = { isLive: true, user: { id: "u-worker", role: "worker", name: "Sam" }, workspace: { id: "ws-live", name: "Real" } };
    ai.value = aiContext({ role: "worker", status: demoAiStatus("worker") });
    render(<WeeklyReport />);
    await periodQuery("summary");
    expect(screen.queryByRole("button", { name: "Explain this report" })).toBeNull();
  });
});
