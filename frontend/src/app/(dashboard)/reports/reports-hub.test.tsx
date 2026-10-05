// The reports hub: its main action opens the Work Report (no dead button),
// scheduled delivery is honestly "coming soon", and the catalog lists only
// what the viewer can open — their role's routes, and payroll only for those
// who may see hourly rates. The session and the HTTP getter are mocked.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { axe } from "jest-axe";
import { reportCatalog } from "@/lib/reports-data";
import { canAccess } from "@/lib/roles";
import { resetTenantDataForTests } from "@/lib/tenant-data";
import type { Role } from "@/lib/types";

const session = vi.hoisted(() => ({
  isLive: false,
  user: { id: "u1", role: "owner" as Role, name: "Ayesha Rahman" },
  livePolicies: null as Record<string, boolean> | null,
}));
const api = vi.hoisted(() => ({ getApi: vi.fn() }));

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/components/session-provider", () => ({ useSession: () => session }));
vi.mock("@/hooks/useApi", () => ({ getApi: api.getApi }));

import ReportsHubPage from "./page";

/** The hrefs of the report links on the page, without duplicates. */
const reportLinks = () => [
  ...new Set(
    screen
      .queryAllByRole("link")
      .map((a) => a.getAttribute("href") ?? "")
      .filter((href) => href.startsWith("/reports/")),
  ),
];

beforeEach(() => {
  localStorage.clear();
  resetTenantDataForTests();
  api.getApi.mockReset();
  session.isLive = false;
  session.user = { id: "u1", role: "owner", name: "Ayesha Rahman" };
  session.livePolicies = null;
});

describe("Reports hub", () => {
  it("points its main action at the Work Report and marks scheduled delivery as coming soon", () => {
    render(<ReportsHubPage />);

    expect(screen.getByRole("link", { name: /Open Work Report/ })).toHaveAttribute("href", "/reports/work-report");
    expect(screen.queryByRole("button", { name: /Schedule report/ })).toBeNull();
    expect(screen.getByText("Coming soon")).toBeInTheDocument();
    expect(api.getApi).not.toHaveBeenCalled();
  });

  it("lists every report, the new Devices & Agents included, in one catalog", () => {
    render(<ReportsHubPage />);

    expect(reportLinks().sort()).toEqual(reportCatalog.map((r) => `/reports/${r.slug}`).sort());
    expect(screen.getByRole("link", { name: /Devices & Agents/ })).toHaveAttribute("href", "/reports/devices");
    expect(screen.getByRole("heading", { name: "All reports" })).toBeInTheDocument();
  });

  it("lists only the reports a client's role can open", () => {
    session.user = { id: "u7", role: "client", name: "Kim Client" };
    render(<ReportsHubPage />);

    const allowed = reportCatalog.filter((r) => canAccess("client", `/reports/${r.slug}`)).map((r) => `/reports/${r.slug}`);
    expect(reportLinks().sort()).toEqual(allowed.sort());
    expect(reportLinks()).not.toContain("/reports/payroll");
    expect(reportLinks()).not.toContain("/reports/attendance");
  });

  it("leaves payroll out for a live viewer who can't see hourly rates", async () => {
    session.isLive = true;
    session.user = { id: "u-admin", role: "admin", name: "Omar Faruk" };
    api.getApi.mockImplementation(async (url: string) => {
      if (url.startsWith("/api/abp/application-configuration")) return { auth: { grantedPolicies: { "Tracker.Activities.ViewAll": true } } };
      throw new Error(`unexpected request ${url}`);
    });
    render(<ReportsHubPage />);

    await waitFor(() => expect(reportLinks()).not.toContain("/reports/payroll"));
    expect(reportLinks()).toContain("/reports/attendance");
  });

  it("uses the permissions the session already read, without asking the server again", () => {
    session.isLive = true;
    session.user = { id: "u-admin", role: "admin", name: "Omar Faruk" };
    session.livePolicies = { "Tracker.Activities.ViewAll": true };
    render(<ReportsHubPage />);

    expect(reportLinks()).not.toContain("/reports/payroll");
    expect(reportLinks()).toContain("/reports/attendance");
    expect(api.getApi).not.toHaveBeenCalled();
  });

  it("a11y: the hub has no axe violations", async () => {
    const { container } = render(<ReportsHubPage />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
