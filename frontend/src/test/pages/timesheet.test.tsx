// Timesheet: a Monday–Sunday week of the reporting zone. It must say which
// dates it covers (the span, and the date under each day), step through weeks
// in the demo too (the grid follows the demo's own activity rows, with the
// days still to come left blank), average a member's day over the days they
// worked (not the week's total), and read live rows for the zone's week.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import TimesheetPage from "@/app/(dashboard)/timesheet/page";
import { getApi } from "@/hooks/useApi";
import { NOW } from "@/lib/mock-data";
import { activities, userById } from "@/lib/tenant-data";
import { browserTimeZone } from "@/lib/time-zone";
import { averagePerWorkedDay, timesheetWeek, weekMatrixFromActivities } from "@/lib/timesheet";
import type { User } from "@/lib/types";
import { formatDuration } from "@/lib/utils";
import { resetPrototypeState } from "@/test/harness";

const RENDER_BUDGET_MS = 20_000;

function demoAs(id: string) {
  const user = userById(id)!;
  session.current = { user, isLive: false, status: "demo" };
  return user;
}

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
});

afterEach(() => {
  session.current = null;
});

describe("timesheet — demo", () => {
  it("names the week it covers and the date under each day, and leaves the days to come blank", () => {
    demoAs("u1"); // the owner
    render(<TimesheetPage />);

    // The demo's clock is Tue 14 Jul 2026.
    expect(screen.getAllByText("13 Jul – 19 Jul 2026").length).toBeGreaterThan(0);
    const table = screen.getByRole("table");
    const headers = within(table).getAllByRole("columnheader").map((h) => h.textContent);
    expect(headers.slice(1, 8)).toEqual([
      "Mon13 Jul",
      "Tue14 Jul (today)",
      "Wed15 Jul",
      "Thu16 Jul",
      "Fri17 Jul",
      "Sat18 Jul",
      "Sun19 Jul",
    ]);
    expect(within(table).getAllByText("Upcoming").length).toBeGreaterThan(0);
    expect(getApi).not.toHaveBeenCalled();
  }, RENDER_BUDGET_MS);

  it("steps back a week with labelled buttons, showing that week's own tracked time", async () => {
    const user = userEvent.setup();
    demoAs("u1");
    render(<TimesheetPage />);

    expect(screen.getByRole("button", { name: "Next week" })).toBeDisabled();
    const totalOf = (label: string) => screen.getByText(label).parentElement!.nextElementSibling!.textContent;
    const thisWeek = totalOf("Total this week");

    await user.click(screen.getByRole("button", { name: "Previous week" }));
    expect(screen.getByText("Last week")).toBeInTheDocument();
    expect(screen.getAllByText("6 Jul – 12 Jul 2026").length).toBeGreaterThan(0);
    expect(within(screen.getByRole("table")).queryByText("Upcoming")).not.toBeInTheDocument();
    // The demo's rows go back to Fri 10 Jul: last week has its own (different) total.
    // The KPI names the shown week, not "this week".
    expect(screen.queryByText("Total this week")).not.toBeInTheDocument();
    expect(totalOf("Week total")).not.toBe(thisWeek);

    await user.click(screen.getByRole("button", { name: "Previous week" }));
    await user.click(screen.getByRole("button", { name: "Previous week" }));
    expect(screen.getByText("3 weeks ago")).toBeInTheDocument();
    expect(screen.getByText("No tracked time in 22 Jun – 28 Jun 2026.")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("averages a member's day over the days they worked, not the week's total", async () => {
    const user = userEvent.setup();
    const me = demoAs("u2"); // a worker
    render(<TimesheetPage />);

    // Last week: the demo's rows cover its Friday to Sunday.
    const week = timesheetWeek(new Date(NOW), browserTimeZone(), -1);
    const { matrix } = weekMatrixFromActivities(activities, week.days, [me as User], browserTimeZone());
    const expected = averagePerWorkedDay(matrix[0]);
    expect(expected.days).toBeGreaterThan(1); // so the old bug (average = the week's total) would show

    await user.click(screen.getByRole("button", { name: "Previous week" }));
    expect(screen.getByRole("heading", { name: "My timesheet" })).toBeInTheDocument();
    const average = screen.getByText("Daily average").parentElement!.nextElementSibling!;
    expect(average).toHaveTextContent(formatDuration(expected.minutes));
    expect(screen.getByText(`over ${expected.days} days worked`)).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("a11y: the timesheet has no axe violations", async () => {
    demoAs("u1");
    const { container } = render(<TimesheetPage />);
    expect(await axe(container)).toHaveNoViolations();
  }, RENDER_BUDGET_MS);
});

describe("timesheet — live", () => {
  const worker: User = {
    id: "w-1",
    name: "Wendy Worker",
    email: "w@real.example",
    role: "worker",
    designation: "Member",
    status: "offline",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: "",
  };

  function serve(series: unknown[] | Error, zone = "Asia/Dhaka") {
    vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
      if (endpoint === "/api/app/workspace/my-preferences") return { timeZone: zone };
      if (endpoint === "/api/app/workspace/settings") return {};
      if (endpoint.startsWith("/api/app/reporting/user-daily-series?")) {
        if (series instanceof Error) throw series;
        return series;
      }
      throw new Error(`unexpected endpoint ${endpoint}`);
    });
  }

  it("asks for the reporting zone's week and puts each member-day under its local date", async () => {
    localStorage.setItem("dosi-token", "live-token");
    session.current = { user: worker, isLive: true, status: "ready" };
    const week = timesheetWeek(new Date(), "Asia/Dhaka");
    serve([{ userId: worker.id, userName: "wendy", date: `${week.days[0]}T00:00:00`, trackedMinutes: 125 }]);

    render(<TimesheetPage />);

    expect(await screen.findAllByText("2h 5m")).not.toHaveLength(0);
    const call = vi.mocked(getApi).mock.calls.map(([e]) => e).find((e) => e.startsWith("/api/app/reporting/user-daily-series?"))!;
    const qs = new URLSearchParams(call.split("?")[1]);
    expect(qs.get("TimeZone")).toBe("Asia/Dhaka");
    expect(qs.get("From")).toBe(week.from.toISOString());
    expect(qs.get("To")).toBe(week.to.toISOString());
    expect(screen.getAllByText(week.span).length).toBeGreaterThan(0);
    // The header and the table's caption both name the zone the days belong to.
    expect(screen.getAllByText(/Dhaka \(UTC\+6\)/)).toHaveLength(2);
  }, RENDER_BUDGET_MS);

  it("says when the week can't be loaded, and retries", async () => {
    const user = userEvent.setup();
    localStorage.setItem("dosi-token", "live-token");
    session.current = { user: worker, isLive: true, status: "ready" };
    serve(new Error("offline"));

    render(<TimesheetPage />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't load tracked time from the server.");
    const week = timesheetWeek(new Date(), "Asia/Dhaka");
    serve([{ userId: worker.id, date: week.days[0], trackedMinutes: 60 }]);
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
    expect(screen.getAllByText("1h").length).toBeGreaterThan(0);
  }, RENDER_BUDGET_MS);
});
