// The command palette in a live session: report shortcuts follow the viewer's
// granted permissions, as the reports hub does — Payroll only for those who may
// see hourly rates (Tracker.Team.Manage). The palette reads the permissions the
// session loaded at sign-in and never asks the server itself (it is mounted on
// every page). The session is mocked; the theme provider is real.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ThemeProvider } from "@/components/theme-provider";
import { users } from "@/lib/tenant-data";
import { resetTenantDataForTests } from "@/lib/demo-tenants";

const session = vi.hoisted(() => ({
  isLive: true,
  user: null as unknown,
  workspace: { id: "live" },
  livePolicies: null as Record<string, boolean> | null,
}));
const api = vi.hoisted(() => ({ getApi: vi.fn() }));

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/components/session-provider", () => ({ useSession: () => session }));
vi.mock("@/hooks/useApi", () => ({ getApi: api.getApi }));

import { CommandPalette } from "@/components/command-palette";

function openPalette() {
  render(
    <ThemeProvider>
      <CommandPalette />
    </ThemeProvider>,
  );
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
}

const option = (name: string) => screen.queryByRole("option", { name: new RegExp(`^${name}`) });

beforeEach(() => {
  resetTenantDataForTests();
  api.getApi.mockReset();
  const admin = users.find((u) => u.role === "admin");
  if (!admin) throw new Error("the demo roster has no admin");
  session.user = admin;
  session.livePolicies = null;
});

describe("CommandPalette (live session)", () => {
  it("leaves Payroll out for an admin who can't see hourly rates", () => {
    session.livePolicies = { "Tracker.Activities.ViewAll": true };
    openPalette();
    expect(option("Attendance")).toBeInTheDocument();
    expect(option("Payroll")).toBeNull();
    expect(api.getApi).not.toHaveBeenCalled();
  });

  it("offers Payroll once Tracker.Team.Manage is granted", () => {
    session.livePolicies = { "Tracker.Activities.ViewAll": true, "Tracker.Team.Manage": true };
    openPalette();
    expect(option("Payroll")).toBeInTheDocument();
  });

  it("falls back to the role's defaults when the session has no permissions", () => {
    openPalette();
    expect(option("Payroll")).toBeInTheDocument();
    expect(api.getApi).not.toHaveBeenCalled();
  });
});
