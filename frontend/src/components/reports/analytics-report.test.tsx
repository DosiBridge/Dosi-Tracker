// Work Analytics: in a live session every figure that adds up comes from the
// reporting endpoints (summary, daily and per-member series, the hourly
// rhythm, devices) in the reporting zone; the period's blocks only feed the
// distributions (focus, activity levels, apps, a day's timeline) and those say
// when they are a sample. The change badges compare with a period that never
// overlaps this one; the 7-day average is seeded with the days before. The
// views are tabs (keyboard-operable) and the Team view only exists for a team.
// Demo mode derives everything from the seeded activities without the
// network. Session + HTTP getters are mocked; the tenant-data live bindings
// are installed for real.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { activities, installLiveDataset, resetTenantDataForTests } from "@/lib/tenant-data";
import { blocksStartingIn, devicesFromBlocks, platformMix, toBlocks } from "@/lib/analytics";
import { resolvePeriod } from "@/lib/work-report";
import { browserTimeZone } from "@/lib/time-zone";
import { referenceNow } from "@/lib/live-session";
import { clearStoredChoices } from "@/hooks/use-stored-choice";
import { buildProject, buildUser, resetFactories } from "@/test/factories";
import type { Role } from "@/lib/types";

const session = vi.hoisted(() => ({
  isLive: true,
  user: { id: "u-owner", name: "Rina Das", role: "owner" as Role },
  workspace: { id: "live-test", name: "Acme Studio" } as { id: string; name: string } | undefined,
}));
const api = vi.hoisted(() => ({ getApi: vi.fn(), getAllPagesApi: vi.fn() }));

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/components/session-provider", () => ({ useSession: () => session }));
vi.mock("@/hooks/useApi", () => ({ getApi: api.getApi, getAllPagesApi: api.getAllPagesApi }));

import { AnalyticsReport } from "./analytics-report";

/* Sun 4 Oct 2026, 12:00 UTC, reporting in UTC: "last 7 days" is Mon 28 Sep … Sun 4 Oct. */
const CLOCK = new Date("2026-10-04T12:00:00Z");
const APOLLO = "#0ea5e9";
const ZEUS = "#f97316";

function row(id: string, userId: string, projectId: string, startIso: string, minutes: number, extra: Record<string, unknown> = {}) {
  return {
    id,
    userId,
    projectId,
    startedAt: startIso,
    endedAt: new Date(Date.parse(startIso) + minutes * 60_000).toISOString(),
    productivity: 70,
    mouseClicks: 10,
    keyboardHits: 50,
    activeWindowsJson: JSON.stringify([{ AppName: "code.exe", WindowTitle: "main.rs", Seconds: minutes * 60 }]),
    ...extra,
  };
}

/* 4 hours: Rina Mon 09:00–10:00 (the screen locked) and 10:30–11:30, Tue 09:00–10:00; Omar Mon 09:30–10:30. */
const ROWS = [
  row("a1", "u-owner", "p1", "2026-09-28T09:00:00Z", 60, { productivity: 80, endReason: "lock", idleSeconds: 600, platform: "windows", deviceId: "d1", agentVersion: "0.2.0" }),
  row("a2", "u-owner", "p1", "2026-09-28T10:30:00Z", 60, { productivity: 80, idleSeconds: 600, activeWindowsJson: JSON.stringify([{ AppName: "chrome.exe", Seconds: 3600 }]) }),
  row("a3", "u-owner", "p1", "2026-09-29T09:00:00Z", 60, { productivity: 60, idleSeconds: 600 }),
  row("a4", "u-worker", "p2", "2026-09-28T09:30:00Z", 60, { productivity: 50, idleSeconds: 1200, platform: "macos", activeWindowsJson: JSON.stringify([{ AppName: "Code", Seconds: 3600 }]) }),
];

const SUMMARY = {
  totalActivities: 4,
  totalTrackedMinutes: 240,
  averageProductivity: 67.5,
  totalIdleMinutes: 50,
  totalKeyboardHits: 200,
  totalMouseClicks: 40,
  perUser: [
    { userId: "u-owner", userName: "rina", trackedMinutes: 180, activityCount: 3, averageProductivity: 73.33, idleMinutes: 30 },
    { userId: "u-worker", userName: "omar", trackedMinutes: 60, activityCount: 1, averageProductivity: 50, idleMinutes: 20 },
  ],
  perProject: [
    { projectId: "p1", projectTitle: "Apollo", trackedMinutes: 180, activityCount: 3, averageProductivity: 73.33, idleMinutes: 30 },
    { projectId: "p2", projectTitle: "Zeus", trackedMinutes: 60, activityCount: 1, averageProductivity: 50, idleMinutes: 20 },
  ],
  perUserProject: [
    { userId: "u-owner", projectId: "p1", trackedMinutes: 180 },
    { userId: "u-worker", projectId: "p2", trackedMinutes: 60 },
  ],
};

/* The six days before the period (the average's seed) and the period itself. */
const SERIES_DAYS = ["2026-09-22", "2026-09-23", "2026-09-24", "2026-09-25", "2026-09-26", "2026-09-27", "2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"];
const SERIES = SERIES_DAYS.map((d) => ({
  date: `${d}T00:00:00`,
  trackedMinutes: d === "2026-09-28" ? 180 : d === "2026-09-29" ? 60 : d === "2026-09-25" ? 120 : 0,
  activityCount: d === "2026-09-28" ? 3 : d === "2026-09-29" ? 1 : d === "2026-09-25" ? 2 : 0,
  averageProductivity: d === "2026-09-28" ? 70 : d === "2026-09-29" ? 60 : d === "2026-09-25" ? 65 : null,
  idleMinutes: d === "2026-09-28" ? 40 : d === "2026-09-29" ? 10 : 0,
  keyboardHits: 0,
  mouseClicks: 0,
}));

const USER_DAYS = [
  { userId: "u-owner", userName: "rina", date: "2026-09-28T00:00:00", trackedMinutes: 120, activityCount: 2, idleMinutes: 20, firstStartedAt: "2026-09-28T09:00:00Z", lastEndedAt: "2026-09-28T11:30:00Z" },
  { userId: "u-worker", userName: "omar", date: "2026-09-28T00:00:00", trackedMinutes: 60, activityCount: 1, idleMinutes: 20, firstStartedAt: "2026-09-28T09:30:00Z", lastEndedAt: "2026-09-28T10:30:00Z" },
  { userId: "u-owner", userName: "rina", date: "2026-09-29T00:00:00", trackedMinutes: 60, activityCount: 1, idleMinutes: 10, firstStartedAt: "2026-09-29T09:00:00Z", lastEndedAt: "2026-09-29T10:00:00Z" },
];

/* Monday 09:00 holds 90 minutes, 10:00 60, 11:00 30; Tuesday 09:00 60. */
const HOURLY = [
  { dayOfWeek: 1, hour: 9, trackedMinutes: 90, activeMinutes: 70 },
  { dayOfWeek: 1, hour: 10, trackedMinutes: 60, activeMinutes: 40 },
  { dayOfWeek: 1, hour: 11, trackedMinutes: 30, activeMinutes: 24 },
  { dayOfWeek: 2, hour: 9, trackedMinutes: 60, activeMinutes: 36 },
];

const DEVICES = [
  { userId: "u-owner", userName: "rina", deviceId: "d1", deviceName: "RINA-PC", platform: "windows", osVersion: "Windows 11", agentVersion: "0.2.0", lastActivityAt: "2026-09-29T10:00:00Z", trackedMinutes: 180 },
  { userId: "u-worker", userName: "omar", deviceId: "d2", deviceName: "Omar's Mac", platform: "macos", osVersion: "macOS 14.5", agentVersion: "0.1.9", lastActivityAt: "2026-09-28T10:30:00Z", trackedMinutes: 60 },
];

const PREVIOUS = { totalActivities: 2, totalTrackedMinutes: 192, averageProductivity: 60, perUser: [], perProject: [] };

type Answer = unknown | ((qs: URLSearchParams) => unknown);

function serve(table: Record<string, Answer>, zone = "UTC") {
  const full: Record<string, Answer> = {
    "/api/app/workspace/my-preferences": { timeZone: zone },
    "/api/app/workspace/settings": { defaultTimeZone: null },
    "/api/abp/application-configuration": { auth: { grantedPolicies: { "Tracker.Activities.ViewAll": true } } },
    ...table,
  };
  api.getApi.mockImplementation(async (url: string) => {
    const [path, query = ""] = url.split("?");
    if (path in full) {
      const entry = full[path];
      const value = typeof entry === "function" ? (entry as (qs: URLSearchParams) => unknown)(new URLSearchParams(query)) : entry;
      if (value instanceof Error) throw value;
      return value;
    }
    throw new Error(`unexpected request ${url}`);
  });
}

function servePages(rows: unknown[] | Error | { items: unknown[]; totalCount: number; truncated: boolean }) {
  api.getAllPagesApi.mockImplementation(async () => {
    if (rows instanceof Error) throw rows;
    return Array.isArray(rows) ? { items: rows, totalCount: rows.length, truncated: false } : rows;
  });
}

const fullTable = {
  "/api/app/reporting/summary": (qs: URLSearchParams) => (qs.get("From") === "2026-09-21T00:00:00.000Z" ? PREVIOUS : SUMMARY),
  "/api/app/reporting/daily-series": SERIES,
  "/api/app/reporting/user-daily-series": USER_DAYS,
  "/api/app/reporting/hourly": HOURLY,
  "/api/app/reporting/devices": DEVICES,
};

const queriesOf = (path: string) =>
  api.getApi.mock.calls
    .map((c) => String(c[0]))
    .filter((u) => u.startsWith(`${path}?`))
    .map((u) => new URLSearchParams(u.split("?")[1]));
const blockQueries = () => api.getAllPagesApi.mock.calls.map((c) => new URLSearchParams(String(c[0]).split("?")[1]));
const keyFigures = () => within(screen.getByLabelText("Key figures"));
const finalValue = (scope: ReturnType<typeof keyFigures>, text: string) => scope.getByText(text, { selector: ".sr-only" });
const kpi = (label: string) => keyFigures().getByText(label).closest("div.flex") as HTMLElement;
const card = (title: string) => screen.getByRole("heading", { name: title }).closest(".wr-keep") as HTMLElement;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(CLOCK);
  localStorage.clear();
  clearStoredChoices();
  resetTenantDataForTests();
  resetFactories();
  api.getApi.mockReset();
  api.getAllPagesApi.mockReset();
  servePages(ROWS);
  session.isLive = true;
  session.user = { id: "u-owner", name: "Rina Das", role: "owner" };
  session.workspace = { id: "live-test", name: "Acme Studio" };
  localStorage.setItem("dosi-token", "test-token");
  installLiveDataset({
    workspaceId: "live-test",
    users: [
      buildUser({ id: "u-owner", name: "Rina Das", role: "owner" }),
      buildUser({ id: "u-worker", name: "Omar Faruk", role: "worker" }),
    ],
    projects: [buildProject({ id: "p1", title: "Apollo", color: APOLLO }), buildProject({ id: "p2", title: "Zeus", color: ZEUS })],
    activities: [],
    now: new Date(),
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Work Analytics (live)", () => {
  it("shows the period, highlights and key figures — the totals from the reporting endpoints", async () => {
    serve(fullTable);
    render(<AnalyticsReport />);

    expect(await screen.findByRole("heading", { name: "Mon, 28 Sep – Sun, 4 Oct" })).toBeInTheDocument();
    const kpis = keyFigures();
    await waitFor(() => expect(finalValue(kpis, "4h")).toBeInTheDocument());
    // The server's activity (67.5) and idle time (50 of 240 minutes), not a sum over blocks.
    expect(finalValue(kpis, "68%")).toBeInTheDocument();
    expect(finalValue(kpis, "50m")).toBeInTheDocument();
    expect(kpis.getByText("21% of tracked time")).toBeInTheDocument();
    expect(kpis.getAllByText("2", { selector: ".sr-only" })).toHaveLength(2); // days worked, and active members
    expect(kpis.getByText("of 7 days")).toBeInTheDocument();
    expect(kpis.getByText("Active members")).toBeInTheDocument();
    expect(kpis.getByText("4 sessions")).toBeInTheDocument();
    // Focus stretches come from the blocks: four sittings of an hour (the screen lock split Rina's Monday).
    await waitFor(() => expect(kpis.getByText("average · longest 1h")).toBeInTheDocument());

    const highlights = within(screen.getByRole("list", { name: "Highlights" }));
    expect(highlights.getByText("Peak 09:00–10:00")).toBeInTheDocument();
    expect(highlights.getByText("Busiest on Mon")).toBeInTheDocument();
    expect(highlights.getByText("Typical day 09:00–10:45")).toBeInTheDocument();
    expect(highlights.getByText("Longest stretch 1h")).toBeInTheDocument();

    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { name: "Hours and activity trend" })).toBeInTheDocument();
    expect(screen.getByText(/days and hours in UTC/)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/NaN|Ayesha Rahman|demo data/);
  });

  it("asks every endpoint for the period's days in the reporting zone, the series from the days before for the average", async () => {
    serve(fullTable);
    render(<AnalyticsReport />);
    await screen.findByRole("heading", { name: "Mon, 28 Sep – Sun, 4 Oct" });
    await waitFor(() => expect(blockQueries()).toHaveLength(1));

    for (const path of ["summary", "user-daily-series", "hourly", "devices"]) {
      const qs = queriesOf(`/api/app/reporting/${path}`)[0];
      expect(qs.get("From"), path).toBe("2026-09-28T00:00:00.000Z");
      expect(qs.get("To"), path).toBe("2026-10-05T00:00:00.000Z");
      expect(qs.get("TimeZone"), path).toBe("UTC");
    }
    // Six days earlier, so the 7-day average of the first days is a full week's.
    const series = queriesOf("/api/app/reporting/daily-series")[0];
    expect(series.get("From")).toBe("2026-09-22T00:00:00.000Z");
    expect(series.get("To")).toBe("2026-10-05T00:00:00.000Z");

    // The blocks: the period up to the moment the read started, at most MAX_BLOCKS of them.
    expect(blockQueries()[0].get("From")).toBe("2026-09-28T00:00:00.000Z");
    expect(blockQueries()[0].get("To")).toBe("2026-10-04T12:00:00.000Z");
    expect(api.getAllPagesApi.mock.calls[0][1]).toMatchObject({ pageSize: 1000, maxItems: 5000 });

    const previous = queriesOf("/api/app/reporting/summary")[1];
    expect(previous.get("From")).toBe("2026-09-21T00:00:00.000Z");
    expect(previous.get("To")).toBe("2026-09-28T00:00:00.000Z");
    // (240 − 192) / 192 = +25 % hours; activity 68 % vs 60 % = +13 %.
    await waitFor(() => expect(keyFigures().getByText("+25%")).toBeInTheDocument());
    expect(keyFigures().getByText("+13%")).toBeInTheDocument();
  });

  it("compares a month in progress with the month before only, never its own first day", async () => {
    vi.setSystemTime(new Date("2026-10-31T12:00:00Z"));
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });

    await user.click(screen.getByRole("button", { name: "Month" }));
    await waitFor(() => expect(queriesOf("/api/app/reporting/summary").some((q) => q.get("From") === "2026-09-01T00:00:00.000Z")).toBe(true));
    const previous = queriesOf("/api/app/reporting/summary").find((q) => q.get("From") === "2026-09-01T00:00:00.000Z");
    expect(previous?.get("To")).toBe("2026-10-01T00:00:00.000Z");
  });

  it("says when the distributions are a sample of a very busy period, while the totals still cover it all", async () => {
    serve(fullTable);
    servePages({ items: ROWS, totalCount: 12_000, truncated: true });
    render(<AnalyticsReport />);

    expect(await screen.findByText(/This period has 12,000 tracked blocks/)).toBeInTheDocument();
    expect(screen.getByText(/charts marked “Sample” use the newest 5,000/)).toBeInTheDocument();
    expect(finalValue(keyFigures(), "4h")).toBeInTheDocument();
    expect(within(kpi("Focus stretch")).getByText("Sample")).toBeInTheDocument();
    expect(within(kpi("Hours tracked")).queryByText("Sample")).toBeNull();
  });

  it("draws the work rhythm from /reporting/hourly and the working windows on each member's clock", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await user.click(await screen.findByRole("tab", { name: "Work rhythm" }));
    const panel = screen.getByRole("tabpanel", { name: "Work rhythm" });
    expect(within(panel).getByRole("img", { name: /busiest slot is Mon 09:00–10:00 with 1h 30m/ })).toBeInTheDocument();
    expect(within(panel).getByRole("img", { name: /24-hour clock.*09:00–10:00/ })).toBeInTheDocument();
    expect(within(panel).getByText("Average start", { selector: "p" }).parentElement).toHaveTextContent("09:00");
    expect(within(panel).getByText("Average finish", { selector: "p" }).parentElement).toHaveTextContent("10:45");
    expect(within(panel).getByText("Average span", { selector: "p" }).parentElement).toHaveTextContent("1h 45m");
  });

  it("shows a day's timeline with the gaps named, for any worked day", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await user.click(await screen.findByRole("tab", { name: "Work rhythm" }));
    const timeline = card("Day timeline");
    // The latest worked day first: Tuesday, Rina alone.
    const picker = within(timeline).getByRole("combobox", { name: "Day" });
    await waitFor(() => expect(picker).toHaveValue("2026-09-29"));
    expect(within(timeline).getByRole("list", { name: "Rina Das: 09:00–10:00" })).toBeInTheDocument();

    await user.selectOptions(picker, "2026-09-28");
    const lanes = within(timeline).getAllByRole("list").filter((l) => l.tagName === "OL");
    expect(lanes.map((l) => l.getAttribute("aria-label"))).toEqual(["Rina Das: 09:00–11:30", "Omar Faruk: 09:30–10:30"]);
    expect(within(lanes[0]).getByText(/10:00–10:30 · Screen locked · 30m/)).toBeInTheDocument();
    expect(within(timeline).getByText("Screen locked")).toBeInTheDocument(); // the legend names the kind
  });

  it("switches views with the tabs, by mouse and keyboard", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });

    screen.getByRole("tab", { name: "Overview" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Work rhythm" })).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Focus & activity" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Focus & activity" })).toHaveFocus();
    expect(await screen.findByRole("img", { name: /of tracked time was in stretches of an hour or more/ })).toBeInTheDocument();
    expect(within(card("Deep work")).getByText("Input per tracked hour", { selector: "p" }).parentElement).toHaveTextContent("60");

    await user.keyboard("{End}");
    expect(screen.getByRole("tab", { name: "Team" })).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowRight}"); // wraps to the first
    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
  });

  it("the device and platform mix, and the desktop app versions", async () => {
    serve(fullTable);
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });
    const devices = card("Devices & platforms");
    expect(devices).toHaveTextContent("2 devices · 2 members");
    const platforms = within(within(devices).getByRole("list", { name: "Tracked time by platform" })).getAllByRole("listitem");
    expect(platforms[0]).toHaveTextContent("Windows1 device3h · 75%");
    expect(platforms[1]).toHaveTextContent("macOS1 device1h · 25%");
    expect(within(devices).getByRole("list", { name: "Desktop app versions" })).toHaveTextContent("v0.2.0 · 1v0.1.9 · 1");
    expect(card("Idle time per day")).toHaveTextContent("50m in all (21%)");
  });

  it("colours who-works-on-what by project, keeping members neutral", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await user.click(await screen.findByRole("tab", { name: "Projects & apps" }));
    const flow = screen.getByRole("img", { name: /Hours from members to projects/ });
    expect(flow.getAttribute("aria-label")).toBe("Hours from members to projects: Rina Das to Apollo 3h, Omar Faruk to Zeus 1h");
    const bands = [...flow.querySelectorAll("path")].map((p) => p.getAttribute("stroke"));
    expect(bands).toEqual([APOLLO, ZEUS]);
    const nodes = [...flow.querySelectorAll("rect")].map((r) => r.getAttribute("fill"));
    expect(nodes).toEqual(["var(--muted-foreground)", "var(--muted-foreground)", APOLLO, ZEUS]);
    expect(screen.getByRole("img", { name: "Project map: Apollo 3h, Zeus 1h" })).toBeInTheDocument();
  });

  it("compares the team in the Team view, start and finish on each member's clock", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await user.click(await screen.findByRole("tab", { name: "Team" }));

    const table = screen.getByRole("table", { name: /Per-member hours/ });
    const rina = within(table).getByRole("row", { name: /Rina Das/ });
    expect(rina).toHaveTextContent("3h");
    expect(rina).toHaveTextContent("30m"); // idle
    expect(rina).toHaveTextContent("73%");
    expect(rina).toHaveTextContent("2/7");
    expect(rina).toHaveTextContent("09:00–10:45*"); // no zone reported: read in the report's
    expect(rina).toHaveTextContent("67%"); // consistency: 2h and 1h days
    expect(within(table).getByRole("row", { name: /Omar Faruk/ })).toHaveTextContent("1h");
    expect(screen.getByText(/No time zone reported by the member/)).toBeInTheDocument();

    // Every bubble can be reached and read; labels never overlap (see lib/analytics placeLabels).
    expect(screen.getByRole("img", { name: "Rina Das: 3h, 73% activity, 3 sessions" })).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("img", { name: "Omar Faruk: 1h, 50% activity, 1 session" })).toBeInTheDocument();

    const picker = within(screen.getByRole("group", { name: "Members to compare" }));
    expect(picker.getByRole("button", { name: "Rina Das" })).toHaveAttribute("aria-pressed", "true");
    await user.click(picker.getByRole("button", { name: "Omar Faruk" }));
    expect(picker.getByRole("button", { name: "Omar Faruk" })).toHaveAttribute("aria-pressed", "false");
  });

  it("has no Team view for one person", async () => {
    serve({ ...fullTable, "/api/app/reporting/summary": { ...SUMMARY, perUser: [SUMMARY.perUser[0]] } });
    servePages(ROWS.filter((r) => r.userId === "u-owner"));
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });
    expect(screen.queryByRole("tab", { name: "Team" })).toBeNull();
    expect(keyFigures().getByText("Tracked sessions")).toBeInTheDocument();
  });

  it("reads activity that wasn't measured as a dash, and leaves it out of the charts", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": { ...SUMMARY, averageProductivity: null, perUser: SUMMARY.perUser.map((u) => ({ ...u, averageProductivity: null })) },
      "/api/app/reporting/daily-series": SERIES.map((d) => ({ ...d, averageProductivity: null })),
    });
    servePages(ROWS.map((r) => ({ ...r, productivity: null })));
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });
    expect(finalValue(keyFigures(), "—")).toBeInTheDocument();
    expect(within(kpi("Average activity")).getByText("Not measured")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Average activity not measured" })).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Focus & activity" }));
    expect(card("Activity distribution")).toHaveTextContent("4h not measured, left out");
    await user.click(screen.getByRole("tab", { name: "Team" }));
    expect(screen.getByText(/2 members without measured activity aren't plotted/)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/NaN/);
  });

  it("an older server without the hourly and device reports: the rhythm comes from the blocks, devices say so", async () => {
    const oldSummary = Object.fromEntries(Object.entries(SUMMARY).filter(([key]) => key !== "perUserProject"));
    serve({
      ...fullTable,
      "/api/app/reporting/summary": oldSummary,
      "/api/app/reporting/hourly": new Error("Not Found"),
      "/api/app/reporting/devices": new Error("Not Found"),
    });
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });
    expect(card("Devices & platforms")).toHaveTextContent("Device details aren't available from this server.");
    expect(screen.queryByRole("alert")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Work rhythm" }));
    expect(screen.getByRole("img", { name: /busiest slot is Mon 09:00–10:00 with 1h 30m/ })).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Projects & apps" }));
    expect(screen.getByRole("img", { name: /Hours from members to projects: Rina Das to Apollo 3h/ })).toBeInTheDocument();
  });

  it("keeps the totals when the blocks fail, with Retry where they were needed", async () => {
    serve(fullTable);
    servePages(new Error("boom"));
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });
    expect(finalValue(keyFigures(), "4h")).toBeInTheDocument();
    expect(within(kpi("Focus stretch")).getByText("Couldn't load the blocks")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Focus & activity" }));
    expect(screen.getAllByText(/blocks behind this chart couldn.t be loaded/).length).toBeGreaterThan(0);

    servePages(ROWS);
    await user.click(screen.getAllByRole("button", { name: /Retry/ })[0]);
    expect(await screen.findByRole("img", { name: /of tracked time was in stretches of an hour or more/ })).toBeInTheDocument();
  });

  it("applies the filters server-side", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    await screen.findByRole("tab", { name: "Overview" });

    await user.selectOptions(screen.getByRole("combobox", { name: "Project" }), "p2");
    await waitFor(() => expect(blockQueries().some((q) => q.get("ProjectId") === "p2")).toBe(true));
    expect(queriesOf("/api/app/reporting/summary").some((q) => q.get("ProjectId") === "p2")).toBe(true);
    expect(queriesOf("/api/app/reporting/hourly").some((q) => q.get("ProjectId") === "p2")).toBe(true);
    expect(await screen.findByText(/Zeus · All members/)).toBeInTheDocument();
  });

  it("a failed load shows the error notice with Retry and no figures, exports or demo rows", async () => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Service Unavailable"));
    const user = userEvent.setup();
    render(<AnalyticsReport />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load this report");
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(keyFigures().getAllByText("—", { selector: ".sr-only" }).length).toBeGreaterThanOrEqual(5);
    expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
    expect(document.body.textContent).not.toMatch(/NaN|Ayesha Rahman|demo data/);

    const before = api.getApi.mock.calls.length;
    await user.click(within(alert).getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(api.getApi.mock.calls.length).toBeGreaterThan(before));
  });

  it("an empty period offers a longer one", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": { ...SUMMARY, totalTrackedMinutes: 0, totalActivities: 0, perUser: [], perProject: [], perUserProject: [] },
      "/api/app/reporting/daily-series": SERIES.map((d) => ({ ...d, trackedMinutes: 0, activityCount: 0 })),
      "/api/app/reporting/user-daily-series": [],
      "/api/app/reporting/hourly": [],
      "/api/app/reporting/devices": [],
    });
    servePages([]);
    const user = userEvent.setup();
    render(<AnalyticsReport />);

    expect(await screen.findByText("No tracked time in this period")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Show the last 30 days" }));
    await waitFor(() => expect(blockQueries().at(-1)?.get("From")).toBe("2026-09-05T00:00:00.000Z"));
  });

  it("a11y: the populated report has no axe violations (overview, rhythm and team)", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    const { container } = render(<AnalyticsReport />);
    await screen.findByRole("heading", { name: "Hours and activity trend" });
    await waitFor(() => expect(within(kpi("Focus stretch")).getByText("average · longest 1h")).toBeInTheDocument());
    vi.useRealTimers();
    expect(await axe(container)).toHaveNoViolations();
    await user.click(screen.getByRole("tab", { name: "Work rhythm" }));
    expect(await axe(container)).toHaveNoViolations();
    await user.click(screen.getByRole("tab", { name: "Team" }));
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("Work Analytics (demo)", () => {
  beforeEach(() => {
    session.isLive = false;
    session.workspace = { id: "w1", name: "Dosi Labs" };
    localStorage.clear();
    resetTenantDataForTests();
  });

  it("derives every view from the seeded activities without the network", async () => {
    const user = userEvent.setup();
    render(<AnalyticsReport />);
    expect(api.getApi).not.toHaveBeenCalled();
    expect(api.getAllPagesApi).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
    for (const name of ["Overview", "Work rhythm", "Focus & activity", "Projects & apps", "Team"]) {
      await user.click(screen.getByRole("tab", { name }));
      expect(screen.getByRole("tabpanel", { name })).toBeInTheDocument();
    }
    expect(document.body.textContent).not.toMatch(/NaN|undefined/);
  });

  it("shows the demo blocks' devices and platforms, or says there are none", () => {
    render(<AnalyticsReport />);
    // The same figures, recomputed from the seeded rows of the period (in the browser's zone).
    const period = resolvePeriod({ kind: "preset", preset: "7d", offset: 0 }, referenceNow(), browserTimeZone());
    const shares = platformMix(devicesFromBlocks(blocksStartingIn(toBlocks(activities), period.from.getTime(), period.to.getTime())));
    const devices = card("Devices & platforms");
    if (shares.length === 0) {
      expect(devices).toHaveTextContent("No device details were reported for this period.");
    } else {
      const items = within(within(devices).getByRole("list", { name: "Tracked time by platform" })).getAllByRole("listitem");
      expect(items.map((li) => li.textContent?.match(/^(Windows|macOS|Not reported)/)?.[1])).toEqual(shares.map((s) => s.label));
      expect(items.map((li) => li.textContent?.match(/(\d+)%$/)?.[1])).toEqual(shares.map((s) => String(s.share)));
    }
  });

  it("a11y: the demo report has no axe violations", async () => {
    const { container } = render(<AnalyticsReport />);
    screen.getByRole("tablist");
    vi.useRealTimers();
    expect(await axe(container)).toHaveNoViolations();
  });
});
