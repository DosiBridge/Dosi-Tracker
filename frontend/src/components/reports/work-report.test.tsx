// The client-ready Work Report: in a live session it shows only the backend's
// figures (whole days of the reporting zone, a Total row equal to the sum of
// the rows, idle time, what was worked on) or an honest loading/error/empty
// state — never the demo dataset; in demo mode it derives the same document
// from the seeded activities without the network. Periods are one click, step
// back with the arrows, or are a custom range; hours read in the viewer's
// chosen format; the summary copies as plain text; every worked day opens to
// who worked, when (on their own clock) and a timeline with its gaps named.
// The session and the HTTP getters are mocked; tenant-data live bindings are
// installed for real, as the session provider does.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { activities, installLiveDataset, resetTenantDataForTests } from "@/lib/tenant-data";
import { formatHoursLong, resolvePeriod } from "@/lib/work-report";
import { toBlocks, blocksStartingIn } from "@/lib/analytics";
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

import { WorkReport } from "./work-report";

/* Sun 4 Oct 2026, 12:00 UTC; the reporting zone is UTC unless a test says otherwise. "Last 7 days" is Mon 28 Sep … Sun 4 Oct. */
const CLOCK = new Date("2026-10-04T12:00:00Z");
const DAYS = ["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"];

const SUMMARY = {
  totalActivities: 14,
  totalTrackedMinutes: 452,
  averageProductivity: 73.27,
  totalMouseClicks: 0,
  totalKeyboardHits: 0,
  totalIdleMinutes: 45,
  perUser: [
    { userId: "u-owner", userName: "rina", activityCount: 10, trackedMinutes: 300, averageProductivity: 80, idleMinutes: 30 },
    { userId: "u-worker", userName: "omar", activityCount: 4, trackedMinutes: 152, averageProductivity: 60, idleMinutes: 15 },
  ],
  perProject: [{ projectId: "p1", projectTitle: "Apollo", activityCount: 14, trackedMinutes: 452, averageProductivity: 73.27, idleMinutes: 45 }],
};

const SERIES = DAYS.map((d) => ({
  date: `${d}T00:00:00`,
  activityCount: d === "2026-09-28" ? 10 : d === "2026-09-29" ? 4 : 0,
  trackedMinutes: d === "2026-09-28" ? 300 : d === "2026-09-29" ? 152 : 0,
  averageProductivity: d === "2026-09-28" ? 80 : d === "2026-09-29" ? 60 : null,
  idleMinutes: d === "2026-09-28" ? 30 : d === "2026-09-29" ? 15 : 0,
  mouseClicks: 0,
  keyboardHits: 0,
}));

const USER_DAYS = [
  {
    userId: "u-owner",
    userName: "rina",
    date: "2026-09-28T00:00:00",
    trackedMinutes: 300,
    activityCount: 10,
    idleMinutes: 30,
    firstStartedAt: "2026-09-28T03:00:00Z",
    lastEndedAt: "2026-09-28T08:15:00Z",
  },
  {
    userId: "u-worker",
    userName: "omar",
    date: "2026-09-29T00:00:00",
    trackedMinutes: 152,
    activityCount: 4,
    idleMinutes: 15,
    firstStartedAt: "2026-09-29T10:00:00Z",
    lastEndedAt: "2026-09-29T12:32:00Z",
  },
];

function block(id: string, userId: string, startIso: string, minutes: number, extra: Record<string, unknown> = {}) {
  return {
    id,
    userId,
    projectId: "p1",
    startedAt: startIso,
    endedAt: new Date(Date.parse(startIso) + minutes * 60_000).toISOString(),
    productivity: 80,
    mouseClicks: 10,
    keyboardHits: 50,
    activeWindowsJson: "[]",
    ...extra,
  };
}

/* Rina works from Dhaka (UTC+6): 09:00–11:00, a break, 11:15–14:15 local. Omar's agent reports no zone. */
const BLOCKS = [
  block("b1", "u-owner", "2026-09-28T03:00:00Z", 120, { endReason: "break", utcOffsetMinutes: 360, description: "Checkout flow", idleSeconds: 600 }),
  block("b2", "u-owner", "2026-09-28T05:15:00Z", 180, { endReason: "interval", utcOffsetMinutes: 360, description: "Payment API", idleSeconds: 1200 }),
  block("b3", "u-worker", "2026-09-29T10:00:00Z", 152, { productivity: 60, description: "  Checkout \n  flow ", idleSeconds: 900 }),
];

/** A payload as an older server sends it: without the fields it doesn't know. */
function omit<T extends object>(value: T, ...keys: string[]): Partial<T> {
  return Object.fromEntries(Object.entries(value).filter(([key]) => !keys.includes(key))) as Partial<T>;
}

type Answer = unknown | ((qs: URLSearchParams) => unknown);

const OWNER_ACCESS = { auth: { grantedPolicies: { "Tracker.Activities.ViewAll": true, "Tracker.Team.Manage": true } } };

/** Answer getApi by path (optionally by query); anything else is a test failure. The reporting zone is UTC unless given. */
function serve(table: Record<string, Answer>, zone: string | null = "UTC") {
  const full: Record<string, Answer> = {
    "/api/app/workspace/my-preferences": { timeZone: zone },
    "/api/app/workspace/settings": { defaultTimeZone: null },
    "/api/abp/application-configuration": OWNER_ACCESS,
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

/** Answer the paged block reads: the rows, or a function of the query; an Error fails the read. */
function servePages(rows: unknown[] | Error | ((qs: URLSearchParams) => { items: unknown[]; totalCount: number; truncated: boolean })) {
  api.getAllPagesApi.mockImplementation(async (url: string) => {
    if (rows instanceof Error) throw rows;
    if (typeof rows === "function") return rows(new URLSearchParams(url.split("?")[1] ?? ""));
    return { items: rows, totalCount: rows.length, truncated: false };
  });
}

const fullTable = {
  "/api/app/reporting/summary": SUMMARY,
  "/api/app/reporting/daily-series": SERIES,
  "/api/app/reporting/user-daily-series": USER_DAYS,
};

const calledUrls = () => api.getApi.mock.calls.map((c) => String(c[0]));
const queriesOf = (path: string) =>
  calledUrls()
    .filter((u) => u.startsWith(`/api/app/reporting/${path}?`))
    .map((u) => new URLSearchParams(u.split("?")[1]));
const blockQueries = () => api.getAllPagesApi.mock.calls.map((c) => new URLSearchParams(String(c[0]).split("?")[1]));
const dailyLog = () => screen.getByRole("table", { name: /Hours worked per day/ });
const keyFigures = () => within(screen.getByLabelText("Key figures"));
const rowOf = (table: HTMLElement, name: RegExp) => within(table).getByRole("row", { name });
const notesCard = () => screen.getByRole("heading", { name: "What was worked on" }).closest(".wr-notes") as HTMLElement;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(CLOCK);
  localStorage.clear();
  clearStoredChoices();
  resetTenantDataForTests();
  resetFactories();
  api.getApi.mockReset();
  api.getAllPagesApi.mockReset();
  servePages(BLOCKS);
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
    projects: [buildProject({ id: "p1", title: "Apollo" })],
    activities: [],
    now: new Date(),
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Work Report (live)", () => {
  it("renders the hero, key figures, the daily log, the team and the project split", async () => {
    serve(fullTable);
    render(<WorkReport />);

    const log = await screen.findByRole("table", { name: /Hours worked per day/ });

    // Hero: workspace, exact whole-day period, the reporting zone, scope, generation time, headline total.
    expect(screen.getByText("Acme Studio")).toBeInTheDocument();
    const hero = screen.getByRole("region", { name: "Mon, 28 Sep – Sun, 4 Oct 2026" });
    expect(within(hero).getByText("All projects · All members")).toBeInTheDocument();
    expect(within(hero).getByText("UTC")).toBeInTheDocument();
    expect(within(hero).getByText("4 Oct 2026, 12:00")).toBeInTheDocument();
    expect(within(hero).getByText("7h 32m")).toBeInTheDocument();

    // Key figures; the period's activity is the server's exact figure (73.27 → 73).
    const kpis = keyFigures();
    expect(kpis.getByText("73%")).toBeInTheDocument();
    // A team's day is the team's total, so its average is per member-day (452 / 2 member-days).
    expect(kpis.getByText("Average per member-day")).toBeInTheDocument();
    expect(kpis.getByText("3h 46m")).toBeInTheDocument();
    expect(kpis.getByText("Members")).toBeInTheDocument();
    // Idle time: 45 of 452 tracked minutes had no input.
    expect(kpis.getByText("Idle time")).toBeInTheDocument();
    expect(kpis.getByText("45m")).toBeInTheDocument();
    expect(kpis.getByText("10% of tracked time · no input")).toBeInTheDocument();
    expect(within(log).getByRole("columnheader", { name: "Team hours" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Team hours per day" })).toBeInTheDocument();

    // One row per day, worked and empty; weekends flagged; idle per day; Total = sum of the rows.
    expect(within(log).getAllByRole("row")).toHaveLength(1 + 7 + 1);
    const monday = rowOf(log, /Mon, 28 Sep/);
    expect(within(monday).getByText("5h")).toBeInTheDocument();
    expect(within(monday).getByText("80%")).toBeInTheDocument();
    expect(within(monday).getByText("10")).toBeInTheDocument();
    expect(within(monday).getByText("30m")).toBeInTheDocument();
    expect(within(rowOf(log, /Tue, 29 Sep/)).getByText("2h 32m")).toBeInTheDocument();
    expect(within(rowOf(log, /Sat, 3 Oct/)).getByText("Weekend")).toBeInTheDocument();
    expect(within(rowOf(log, /Sun, 4 Oct/)).getByText("Today")).toBeInTheDocument();
    expect(within(rowOf(log, /Wed, 30 Sep/)).getAllByText("—").length).toBeGreaterThan(0);
    const total = rowOf(log, /^Total/);
    expect(within(total).getByText("7h 32m")).toBeInTheDocument();
    expect(within(total).getByText("14")).toBeInTheDocument();
    expect(within(total).getByText("45m")).toBeInTheDocument();

    // Team timesheet: roster names (not server user names), member totals, idle, grand total.
    const sheet = screen.getByRole("table", { name: /Hours per member per day/ });
    expect(within(rowOf(sheet, /Rina Das/)).getAllByText("5h").length).toBe(2); // the cell and the row total
    expect(within(rowOf(sheet, /Rina Das/)).getByText("30m")).toBeInTheDocument();
    expect(within(rowOf(sheet, /Omar Faruk/)).getAllByText("2h 32m").length).toBe(2);
    expect(within(rowOf(sheet, /^Total/)).getByText("7h 32m")).toBeInTheDocument();
    expect(within(sheet).getByRole("columnheader", { name: "Idle" })).toBeInTheDocument();
    // …and the team leaderboard beside the log.
    const team = within(screen.getByRole("list", { name: "Hours per member" }));
    expect(team.getAllByRole("listitem")[0]).toHaveTextContent("Rina Das");
    expect(team.getAllByRole("listitem")[0]).toHaveTextContent("idle 30m");

    // Project breakdown resolves the title and shows the share.
    const projectsCard = screen.getByRole("heading", { name: "Project breakdown" }).closest(".wr-keep") as HTMLElement;
    const apollo = within(projectsCard).getByRole("listitem");
    expect(apollo).toHaveTextContent("Apollo");
    expect(apollo).toHaveTextContent("7h 32m · 100%");

    // Highlights in plain words.
    expect(screen.getByText("Busiest day")).toBeInTheDocument();
    expect(screen.getByText("Most hours")).toBeInTheDocument();

    expect(screen.getByText("Prepared with Dosi Tracker")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/NaN|Ayesha Rahman|Tanvir Hasan|demo data/);
  });

  it("lists what was worked on: time per task note, by note, by day and by member", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    const card = await waitFor(() => notesCard());
    const notes = within(within(card).getByRole("list", { name: "Time per task note" })).getAllByRole("listitem");
    // Rina's "Checkout flow" and Omar's "  Checkout \n  flow " are one note: 120 + 152 minutes, Omar's share first.
    expect(notes[0]).toHaveTextContent("Checkout flow");
    expect(notes[0]).toHaveTextContent("4h 32m · 60%");
    expect(notes[0]).toHaveTextContent("2 days · Omar Faruk, Rina Das");
    expect(notes[1]).toHaveTextContent("Payment API");
    expect(notes[1]).toHaveTextContent("3h · 40%");
    expect(within(card).queryByText(/Sample/)).toBeNull();

    await user.click(within(card).getByRole("button", { name: "By day" }));
    expect(within(card).getByRole("region", { name: "Mon, 28 Sep" })).toHaveTextContent("Payment API3h");
    expect(within(card).getByRole("region", { name: "Tue, 29 Sep" })).toHaveTextContent("Checkout flow2h 32m");

    await user.click(within(card).getByRole("button", { name: "By member" }));
    const omar = within(card).getByRole("region", { name: "Omar Faruk" });
    expect(omar).toHaveTextContent("2h 32m");
    expect(omar).toHaveTextContent("Checkout flow");
  });

  it("queries whole days of the reporting zone for every endpoint, plus the period before for the comparison", async () => {
    serve(fullTable);
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    for (const path of ["summary", "daily-series", "user-daily-series"]) {
      const qs = queriesOf(path)[0];
      expect(qs, path).toBeDefined();
      expect(qs.get("From")).toBe("2026-09-28T00:00:00.000Z");
      expect(qs.get("To")).toBe("2026-10-05T00:00:00.000Z");
      expect(qs.get("TimeZone")).toBe("UTC");
    }
    const previous = queriesOf("summary")[1];
    expect(previous.get("From")).toBe("2026-09-21T00:00:00.000Z");
    expect(previous.get("To")).toBe("2026-09-28T00:00:00.000Z");
    // The blocks behind the notes are read up to the moment the read started.
    await waitFor(() => expect(blockQueries()).toHaveLength(1));
    expect(blockQueries()[0].get("From")).toBe("2026-09-28T00:00:00.000Z");
    expect(blockQueries()[0].get("To")).toBe("2026-10-04T12:00:00.000Z");
    expect(api.getAllPagesApi.mock.calls[0][1]).toMatchObject({ maxItems: 5000 });
  });

  it("buckets days in the viewer's own zone and says so", async () => {
    serve(fullTable, "Asia/Dhaka");
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    const qs = queriesOf("daily-series")[0];
    // Days are Dhaka days: Mon 28 Sep begins at 18:00 UTC on the 27th.
    expect(qs.get("From")).toBe("2026-09-27T18:00:00.000Z");
    expect(qs.get("To")).toBe("2026-10-04T18:00:00.000Z");
    expect(qs.get("TimeZone")).toBe("Asia/Dhaka");
    expect(within(screen.getByRole("region", { name: /Mon, 28 Sep/ })).getByText("Dhaka (UTC+6)")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: /Mon, 28 Sep/ })).getByText("4 Oct 2026, 18:00")).toBeInTheDocument();
  });

  it("waits for the reporting zone before asking for figures (never fetches one zone, then another)", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    serve(fullTable);
    const answer = api.getApi.getMockImplementation()!;
    api.getApi.mockImplementation(async (url: string) => {
      if (url.startsWith("/api/app/workspace/my-preferences")) await gate;
      return answer(url);
    });
    render(<WorkReport />);
    expect(await screen.findByText("Loading report data…")).toBeInTheDocument();
    expect(queriesOf("summary")).toHaveLength(0);
    release();
    await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(queriesOf("daily-series")).toHaveLength(1);
  });

  it("shows the change against the period before", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": (qs: URLSearchParams) =>
        qs.get("From") === "2026-09-21T00:00:00.000Z" ? { ...SUMMARY, totalTrackedMinutes: 400 } : SUMMARY,
    });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    // (452 − 400) / 400 = +13 %.
    const hero = screen.getByRole("region", { name: /Mon, 28 Sep/ });
    const pill = within(hero).getByText(/vs the 7 days before/).parentElement as HTMLElement;
    expect(pill).toHaveTextContent("+13% vs the 7 days before");
  });

  it("leaves the change out when the comparison can't be loaded", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": (qs: URLSearchParams) =>
        qs.get("From") === "2026-09-21T00:00:00.000Z" ? new Error("boom") : SUMMARY,
    });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(screen.queryByText(/vs the 7 days before/)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("switches period in one click and steps back with the arrows", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    expect(screen.getByRole("button", { name: "Next period" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Month" }));
    await waitFor(() => expect(queriesOf("daily-series").some((q) => q.get("From") === "2026-10-01T00:00:00.000Z")).toBe(true));
    expect(screen.getByText("This month")).toBeInTheDocument();
    expect(queriesOf("daily-series").at(-1)?.get("To")).toBe("2026-10-05T00:00:00.000Z");

    await user.click(screen.getByRole("button", { name: "Previous period" }));
    await waitFor(() => expect(queriesOf("daily-series").at(-1)?.get("From")).toBe("2026-09-01T00:00:00.000Z"));
    expect(queriesOf("daily-series").at(-1)?.get("To")).toBe("2026-10-01T00:00:00.000Z");
    // The comparison is all of August, never a day of September.
    const previous = queriesOf("summary").at(-1);
    expect([previous?.get("From"), previous?.get("To")]).toEqual(["2026-08-01T00:00:00.000Z", "2026-08-31T00:00:00.000Z"]);
    expect(screen.getByText("Last month")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next period" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Back to today" }));
    await waitFor(() => expect(screen.getByText("This month")).toBeInTheDocument());
  });

  it("reports a custom range of days, compared with the same number of days before it", async () => {
    // The series answers only the days it is asked for.
    serve({
      ...fullTable,
      "/api/app/reporting/daily-series": (qs: URLSearchParams) =>
        SERIES.filter((d) => d.date.slice(0, 10) >= String(qs.get("From")).slice(0, 10) && d.date.slice(0, 10) < String(qs.get("To")).slice(0, 10)),
    });
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    await user.click(screen.getByRole("button", { name: "Custom" }));
    fireEvent.change(screen.getByLabelText("From"), { target: { value: "2026-09-01" } });
    fireEvent.change(screen.getByLabelText("To"), { target: { value: "2026-09-15" } });
    await user.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => expect(queriesOf("daily-series").at(-1)?.get("From")).toBe("2026-09-01T00:00:00.000Z"));
    expect(queriesOf("daily-series").at(-1)?.get("To")).toBe("2026-09-16T00:00:00.000Z");
    const previous = queriesOf("summary").at(-1);
    expect([previous?.get("From"), previous?.get("To")]).toEqual(["2026-08-17T00:00:00.000Z", "2026-09-01T00:00:00.000Z"]);
    expect(screen.getByRole("button", { name: "Custom" })).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByRole("region", { name: "Tue, 1 Sep – Tue, 15 Sep 2026" })).toBeInTheDocument();
  });

  it("formats hours the way the viewer chooses, totals adding up the rounded rows, and remembers it", async () => {
    const week = DAYS.map((d, i) => ({ ...SERIES[i], trackedMinutes: i < 5 ? 452 : 0, averageProductivity: i < 5 ? 70 : null }));
    serve({ ...fullTable, "/api/app/reporting/daily-series": week });
    const user = userEvent.setup();
    render(<WorkReport />);
    const log = await screen.findByRole("table", { name: /Hours worked per day/ });

    const formats = within(screen.getByRole("group", { name: "Hours format" }));
    await user.click(formats.getByRole("button", { name: "7:30" }));
    expect(within(rowOf(log, /^Total/)).getByText("37:40")).toBeInTheDocument();
    expect(localStorage.getItem("dosi-work-report-hours")).toBe("clock");

    await user.click(formats.getByRole("button", { name: "7.50" }));
    // Five rows of 7.53 — the Total is their sum, 37.65 (the minutes alone would round to 37.67).
    expect(within(rowOf(log, /Mon, 28 Sep/)).getByText("7.53")).toBeInTheDocument();
    expect(within(rowOf(log, /^Total/)).getByText("37.65")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: /Mon, 28 Sep/ })).getByText("37.65")).toBeInTheDocument();
    expect(formats.getByRole("button", { name: "7.50" })).toHaveAttribute("aria-pressed", "true");
  });

  it("copies a plain-text summary for email or chat, with what was worked on", async () => {
    serve(fullTable);
    const user = userEvent.setup(); // (installs its own clipboard; ours replaces it below)
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    await waitFor(() => expect(within(notesCard()).getByText("Payment API")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: /Copy summary/ }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const text = String(writeText.mock.calls[0][0]);
    expect(text).toContain("Work Report — Acme Studio");
    expect(text).toContain("Mon, 28 Sep – Sun, 4 Oct 2026 · All projects · All members");
    expect(text).toContain("Total worked: 7h 32m on 2 of 7 days");
    expect(text).toContain("Idle: 45m (10% of tracked time)");
    expect(text).toContain("• Apollo — 7h 32m (100%)");
    expect(text).toContain("Worked on:\n• Checkout flow — 4h 32m\n• Payment API — 3h");
  });

  it("opens a day to show who worked, when on their own clock, and a timeline with its gaps named", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    const log = await screen.findByRole("table", { name: /Hours worked per day/ });
    await waitFor(() => expect(within(notesCard()).getByText("Payment API")).toBeInTheDocument());

    const open = screen.getByRole("button", { name: "Show who worked on Mon, 28 Sep" });
    expect(open).toHaveAttribute("aria-expanded", "false");
    await user.click(open);
    const who = screen.getByRole("list", { name: "Who worked on Mon, 28 Sep" });
    expect(within(who).getAllByRole("listitem")).toHaveLength(1);
    expect(who).toHaveTextContent("Rina Das");
    expect(who).toHaveTextContent("5h");
    // Rina works from Dhaka: her 03:00–08:15 UTC reads as her own 09:00–14:15.
    expect(who).toHaveTextContent("09:00–14:15 local (UTC+6) · idle 30m");
    // The timeline names the gap after the block that ended for a break.
    const lane = screen.getByRole("list", { name: "Rina Das: 03:00–08:15" });
    expect(within(lane).getByText(/05:00–05:15 · On a break · 15m/)).toBeInTheDocument();
    expect(within(lane).getByText(/Worked 03:00–05:00 · Apollo · 2h · 80% activity · Checkout flow/)).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "What was worked on, Mon, 28 Sep" })).toHaveTextContent("Payment API3h");
    expect(screen.getByRole("button", { name: "Hide who worked on Mon, 28 Sep" })).toHaveAttribute("aria-expanded", "true");
    // Days without time have nothing to open.
    expect(screen.queryByRole("button", { name: /who worked on Wed, 30 Sep/ })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Hide empty days (5)" }));
    // Header + 2 worked days + Monday's breakdown + Total.
    expect(within(log).getAllByRole("row")).toHaveLength(1 + 2 + 1 + 1);
    expect(screen.getByText(/5 empty days hidden/)).toBeInTheDocument();
    expect(within(rowOf(log, /^Total/)).getByText("7h 32m")).toBeInTheDocument();
  });

  it("a member whose agent reports no zone shows times on the report's clock", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    await waitFor(() => expect(within(notesCard()).getByText("Payment API")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Show who worked on Tue, 29 Sep" }));
    expect(screen.getByRole("list", { name: "Who worked on Tue, 29 Sep" })).toHaveTextContent("10:00–12:32 · idle 15m");
  });

  it("shows the days as a calendar on request", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    await user.click(screen.getByRole("button", { name: "Calendar" }));
    expect(screen.getByRole("img", { name: /Calendar of hours worked per day/ })).toBeInTheDocument();
  });

  it("applies the project filter server-side and names it in the scope", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });

    await user.selectOptions(screen.getByRole("combobox", { name: "Project" }), "p1");
    await waitFor(() => expect(calledUrls().some((u) => u.includes("ProjectId=p1"))).toBe(true));
    await waitFor(() => expect(blockQueries().some((q) => q.get("ProjectId") === "p1")).toBe(true));
    expect(await screen.findByText("Apollo · All members")).toBeInTheDocument();
  });

  it("hides the team sections when one member is selected", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours per member per day/ });

    serve({
      ...fullTable,
      "/api/app/reporting/summary": { ...SUMMARY, perUser: [SUMMARY.perUser[0]] },
      "/api/app/reporting/user-daily-series": [USER_DAYS[0]],
    });
    await user.selectOptions(screen.getByRole("combobox", { name: "Member" }), "u-owner");
    await waitFor(() => expect(calledUrls().some((u) => u.includes("UserId=u-owner"))).toBe(true));
    expect(await screen.findByText("All projects · Rina Das")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("table", { name: /Hours per member per day/ })).toBeNull());
    expect(screen.queryByRole("list", { name: "Hours per member" })).toBeNull();

    // One person: plain hours and a per-worked-day average.
    expect(within(dailyLog()).getByRole("columnheader", { name: "Hours" })).toBeInTheDocument();
    expect(screen.getByText("Average per worked day")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Hours per day" })).toBeInTheDocument();
    expect(screen.queryByText("Members")).toBeNull();
    expect(screen.getByText("Full days (8h+)")).toBeInTheDocument();
  });

  it("reads activity that wasn't measured as a dash, never 0%", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": { ...SUMMARY, averageProductivity: null },
      "/api/app/reporting/daily-series": SERIES.map((d) => ({ ...d, averageProductivity: null })),
    });
    render(<WorkReport />);
    const log = await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(keyFigures().getByText("Not measured")).toBeInTheDocument();
    expect(within(rowOf(log, /Mon, 28 Sep/)).queryByText(/%/)).toBeNull();
    expect(within(rowOf(log, /Mon, 28 Sep/)).getAllByText("Not measured").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/\b0%/);
  });

  it("says when an older server doesn't measure idle time", async () => {
    serve({
      ...fullTable,
      "/api/app/reporting/summary": omit(SUMMARY, "totalIdleMinutes"),
      "/api/app/reporting/daily-series": SERIES.map((d) => omit(d, "idleMinutes")),
      "/api/app/reporting/user-daily-series": USER_DAYS.map((d) => omit(d, "idleMinutes", "firstStartedAt", "lastEndedAt")),
    });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    const idle = keyFigures().getByText("Idle time").closest("div") as HTMLElement;
    expect(idle).toHaveTextContent("—");
    expect(idle).toHaveTextContent("Not measured");
    expect(screen.getByRole("table", { name: /Hours per member per day/ })).not.toHaveTextContent(/Idle/);
  });

  it("marks the notes as a sample when the period has more blocks than were read", async () => {
    serve(fullTable);
    servePages(() => ({ items: BLOCKS, totalCount: 12_000, truncated: true }));
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    const card = await waitFor(() => notesCard());
    expect(await within(card).findByText(/From the newest 5,000 of 12,000 tracked blocks/)).toBeInTheDocument();
    // The hours themselves still come from the reporting totals.
    expect(within(rowOf(dailyLog(), /^Total/)).getByText("7h 32m")).toBeInTheDocument();
  });

  it("keeps the report when only the blocks behind the notes fail, and offers Retry", async () => {
    serve(fullTable);
    servePages(new Error("boom"));
    const user = userEvent.setup();
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    const card = notesCard();
    expect(await within(card).findByText(/task notes couldn.t be loaded/)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();

    servePages(BLOCKS);
    await user.click(within(card).getByRole("button", { name: /Retry/ }));
    expect(await within(card).findByText("Payment API")).toBeInTheDocument();
  });

  it("a failed load shows the error notice with Retry, dashes instead of figures, no exports, and no demo rows", async () => {
    api.getApi.mockRejectedValue(new Error("API GET Error: Service Unavailable"));
    const user = userEvent.setup();
    render(<WorkReport />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load this report");
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByText("No tracked time in this period")).toBeNull();
    expect(keyFigures().getAllByText("—").length).toBeGreaterThanOrEqual(5);
    expect(screen.getByRole("button", { name: /Copy summary/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
    expect(document.body.textContent).not.toMatch(/NaN|Ayesha Rahman|Tanvir Hasan|demo data/);

    const before = api.getApi.mock.calls.length;
    await user.click(within(alert).getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(api.getApi.mock.calls.length).toBeGreaterThan(before));
  });

  it("offers Export only once the figures are in", async () => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    serve(fullTable);
    const answer = api.getApi.getMockImplementation()!;
    api.getApi.mockImplementation(async (url: string) => {
      if (url.startsWith("/api/app/reporting/summary")) await gate;
      return answer(url);
    });
    render(<WorkReport />);
    await screen.findByText("Loading report data…");
    expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
    release();
    await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(screen.getByRole("button", { name: /Export/ })).toBeEnabled();
  });

  it("shows an honest empty state that offers a longer period, with nothing to export", async () => {
    serve({
      "/api/app/reporting/summary": { ...SUMMARY, totalTrackedMinutes: 0, perUser: [], perProject: [] },
      "/api/app/reporting/daily-series": SERIES.map((d) => ({ ...d, trackedMinutes: 0, activityCount: 0, averageProductivity: null })),
      "/api/app/reporting/user-daily-series": [],
    });
    servePages([]);
    const user = userEvent.setup();
    render(<WorkReport />);

    expect(await screen.findByText("No tracked time in this period")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByText("Project breakdown")).toBeNull();
    expect(screen.getByRole("button", { name: /Export/ })).toBeDisabled();
    expect(document.body.textContent).not.toMatch(/NaN/);

    await user.click(screen.getByRole("button", { name: "Show the last 30 days" }));
    await waitFor(() => expect(queriesOf("daily-series").at(-1)?.get("From")).toBe("2026-09-05T00:00:00.000Z"));
  });

  it("keeps the daily report when only the per-member series fails, and says so", async () => {
    serve({ ...fullTable, "/api/app/reporting/user-daily-series": new Error("boom") });
    render(<WorkReport />);

    await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(screen.queryByRole("table", { name: /Hours per member per day/ })).toBeNull();
    expect(screen.getByText(/per-member timesheet couldn.t be loaded/)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("a member without ViewAll is reported on themselves, and offered only themselves", async () => {
    session.user = { id: "u-worker", name: "Omar Faruk", role: "worker" };
    serve({
      ...fullTable,
      "/api/abp/application-configuration": { auth: { grantedPolicies: {} } },
      "/api/app/reporting/summary": { ...SUMMARY, perUser: [SUMMARY.perUser[1]] },
      "/api/app/reporting/user-daily-series": [USER_DAYS[1]],
    });
    render(<WorkReport />);
    expect(await screen.findByText("All projects · Omar Faruk")).toBeInTheDocument();
    const members = within(screen.getByRole("combobox", { name: "Member" })).getAllByRole("option");
    expect(members.map((o) => o.textContent)).toEqual(["All members", "Omar Faruk"]);
  });

  it("a manager whose ViewAll was trimmed can't pick someone else's name either", async () => {
    session.user = { id: "u-owner", name: "Rina Das", role: "owner" };
    serve({ ...fullTable, "/api/abp/application-configuration": { auth: { grantedPolicies: { "Tracker.Team.Manage": true } } } });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    await waitFor(() =>
      expect(within(screen.getByRole("combobox", { name: "Member" })).getAllByRole("option").map((o) => o.textContent)).toEqual(["All members", "Rina Das"]),
    );
  });

  it("falls back to a neutral workspace name, and labels projects by the server's title or their id", async () => {
    session.workspace = undefined;
    serve({
      ...fullTable,
      "/api/app/reporting/summary": {
        ...SUMMARY,
        perProject: [
          { projectId: "9f8e7d6c-0000-1111", trackedMinutes: 300 },
          { projectId: "archived-1", projectTitle: "Legacy CRM", isArchived: true, trackedMinutes: 152 },
        ],
      },
    });
    render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours worked per day/ });
    expect(screen.getByText("Workspace")).toBeInTheDocument();
    expect(screen.getAllByText("Project 9f8e7d6c").length).toBeGreaterThan(0);
    const projectsCard = screen.getByRole("heading", { name: "Project breakdown" }).closest(".wr-keep") as HTMLElement;
    expect(within(projectsCard).getByText("Legacy CRM")).toBeInTheDocument();
    expect(within(projectsCard).getByText("Archived")).toBeInTheDocument();
  });

  it("a11y: the populated report, with a day opened, has no axe violations", async () => {
    serve(fullTable);
    const user = userEvent.setup();
    const { container } = render(<WorkReport />);
    await screen.findByRole("table", { name: /Hours per member per day/ });
    await waitFor(() => expect(within(notesCard()).getByText("Payment API")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Show who worked on Mon, 28 Sep" }));
    vi.useRealTimers(); // axe schedules work on real timers
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("Work Report (demo)", () => {
  beforeEach(() => {
    session.isLive = false;
    session.workspace = { id: "w1", name: "Dosi Labs" };
    localStorage.clear();
    resetTenantDataForTests();
  });

  it("derives the document from the seeded activities without touching the network", () => {
    render(<WorkReport />);

    expect(api.getApi).not.toHaveBeenCalled();
    expect(api.getAllPagesApi).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();

    // The same period, recomputed independently: the blocks that started in it, in the browser's zone.
    const period = resolvePeriod({ kind: "preset", preset: "7d", offset: 0 }, referenceNow(), browserTimeZone());
    expect(period.days).toHaveLength(7);
    const inPeriod = blocksStartingIn(toBlocks(activities), period.from.getTime(), period.to.getTime());
    const expected = inPeriod.reduce((s, b) => s + b.minutes, 0);
    expect(expected).toBeGreaterThan(0);

    const log = dailyLog();
    expect(within(log).getAllByRole("row")).toHaveLength(1 + 7 + 1);
    expect(within(rowOf(log, /^Total/)).getByText(formatHoursLong(expected))).toBeInTheDocument();
    expect(screen.getByText("Dosi Labs")).toBeInTheDocument();
    // The seeded notes are listed under what was worked on.
    expect(within(notesCard()).getByRole("list", { name: "Time per task note" })).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/NaN/);
  });

  it("a11y: the demo report has no axe violations", async () => {
    const { container } = render(<WorkReport />);
    dailyLog();
    vi.useRealTimers();
    expect(await axe(container)).toHaveNoViolations();
  });
});
