// ActivitiesPage in a LIVE session: cards show the block's REAL screenshot
// (the small "thumb" rendition first, then the full capture, then the app
// glyph), loaded only once a card nears the viewport and never more than four
// at a time. Opening a card shows the full block report — the capture fetched
// from `screenshots/{activityId}` (a PATH parameter: the old `?activityId=`
// query 404'd on every call), stats, the per-minute timeline and the apps &
// windows table — while legacy rows without that data still read cleanly.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAuthedBlobUrl: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import ActivitiesPage from "@/app/(dashboard)/activities/page";
import { getApi, getAuthedBlobUrl } from "@/hooks/useApi";
import { mapApiActivities, mapApiProject, type ApiActivityDto } from "@/lib/live-dataset";
import { formatClock } from "@/lib/activity-report";
import { installLiveDataset } from "@/lib/tenant-data";
import type { User } from "@/lib/types";
import { __setPathname } from "@/test/next-navigation-stub";
import { resetPrototypeState } from "@/test/harness";

const RENDER_BUDGET_MS = 20_000;

const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";
const guid = (n: number) => `aaaaaaaa-0000-0000-0000-${String(n).padStart(12, "0")}`;

const A_NEW = guid(1); // new agent: thumb + screen + webcam, timeline, per-window counts
const A_LEGACY = guid(2); // legacy row: one window, no seconds, no captures property, no timeline
const A_SCREEN_ONLY = guid(3); // capture without a thumb rendition
const THUMB_NEW = guid(101);
const SCREEN_NEW = guid(102);
const WEBCAM_NEW = guid(103);
const SCREEN_ONLY = guid(301);

function member(id: string, name: string, role: User["role"]): User {
  return { id, name, email: `${id}@real.example`, role, designation: role === "owner" ? "Workspace Owner" : "Member", status: "offline", timezone: "UTC", trackedToday: 0, productivity: 0, joinedAt: "" };
}
const owner = member(OWNER, "Olivia Owner", "owner");
const worker = member(WORKER, "Wendy Worker", "worker");

/** A block that ended `endedMinsAgo` ago and lasted `seconds`. */
function times(endedMinsAgo: number, seconds: number) {
  const end = Date.now() - endedMinsAgo * 60_000;
  return { startedAt: new Date(end - seconds * 1000).toISOString(), endedAt: new Date(end).toISOString() };
}

function newRow(over: Partial<ApiActivityDto> = {}): ApiActivityDto {
  return {
    id: A_NEW,
    userId: WORKER,
    projectId: PROJECT,
    ...times(30, 298), // 4m 58s
    productivity: 82,
    mouseClicks: 21,
    keyboardHits: 325,
    description: "Tracked session",
    activeWindowsJson: JSON.stringify([
      { AppName: "code.exe", WindowTitle: "main.rs", Seconds: 180, KeyboardHits: 300, MouseClicks: 10 },
      { AppName: "chrome.exe", WindowTitle: "PR #12 review", Seconds: 60, KeyboardHits: 5, MouseClicks: 9 },
      { AppName: "code.exe", WindowTitle: "lib.rs", Seconds: 40, KeyboardHits: 20, MouseClicks: 2 },
    ]),
    runningProgramsJson: JSON.stringify([{ AppName: "slack.exe" }, { AppName: "spotify.exe" }]),
    timelineJson: JSON.stringify([
      { Minute: 0, KeyboardHits: 80, MouseClicks: 3, Active: true, AppName: "code.exe" },
      { Minute: 1, KeyboardHits: 120, MouseClicks: 5, Active: true, AppName: "code.exe" },
      { Minute: 2, KeyboardHits: 0, MouseClicks: 0, Active: false, AppName: null },
      { Minute: 3, KeyboardHits: 5, MouseClicks: 9, Active: true, AppName: "chrome.exe" },
      { Minute: 4, KeyboardHits: 120, MouseClicks: 4, Active: true, AppName: "code.exe" },
    ]),
    captures: [
      { id: SCREEN_NEW, kind: "screen" },
      { id: THUMB_NEW, kind: "thumb" },
      { id: WEBCAM_NEW, kind: "webcam" },
    ],
    ...over,
  };
}

function legacyRow(): ApiActivityDto {
  return {
    id: A_LEGACY,
    userId: WORKER,
    projectId: PROJECT,
    ...times(60, 600),
    productivity: 55,
    mouseClicks: 40,
    keyboardHits: 900,
    description: "Old agent block",
    activeWindowsJson: JSON.stringify([{ AppName: "Rider", WindowTitle: "Worker.cs" }]),
    runningProgramsJson: "[]",
  };
}

function screenOnlyRow(): ApiActivityDto {
  return { ...newRow(), id: A_SCREEN_ONLY, ...times(90, 300), timelineJson: "[]", captures: [{ id: SCREEN_ONLY, kind: "screen" }] };
}

/* ---- a controllable IntersectionObserver: nothing is "in view" until the test says so ---- */
class FakeObserver {
  static all: FakeObserver[] = [];
  targets = new Set<Element>();
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeObserver.all.push(this);
  }
  observe(el: Element) {
    this.targets.add(el);
  }
  unobserve(el: Element) {
    this.targets.delete(el);
  }
  disconnect() {
    this.targets.clear();
  }
  takeRecords() {
    return [];
  }
  fire() {
    if (this.targets.size === 0) return;
    const entries = [...this.targets].map((target) => ({ target, isIntersecting: true }) as IntersectionObserverEntry);
    this.callback(entries, this as unknown as IntersectionObserver);
  }
}
function scrollEverythingIntoView() {
  act(() => {
    for (const o of [...FakeObserver.all]) o.fire();
  });
}

const originalObserver = window.IntersectionObserver;
const originalRevoke = URL.revokeObjectURL;
let revoke: ReturnType<typeof vi.fn>;

function json(body: unknown): Response {
  const text = JSON.stringify(body);
  return { ok: true, status: 200, statusText: "OK", json: async () => JSON.parse(text), text: async () => text } as Response;
}

function goLive(rows: ApiActivityDto[]) {
  localStorage.setItem("dosi-token", "live-token");
  installLiveDataset({
    workspaceId: "live-activities",
    users: [owner, worker],
    projects: [mapApiProject({ id: PROJECT, title: "Apollo", color: "#0d9488" }, [WORKER])],
    activities: mapApiActivities(rows),
    now: new Date(),
  });
  session.current = { user: owner, isLive: true, status: "ready" };
  // The page's list request goes through the real useApi hook → fetch.
  vi.stubGlobal("fetch", vi.fn<typeof fetch>().mockImplementation(async () => json({ items: rows, totalCount: rows.length })));
}

/** Blob URLs are fake strings naming what was downloaded. */
const blobFor = (endpoint: string) => `blob:${endpoint}`;
const content = (id: string) => `/api/app/activity/screenshot/${id}/content`;

beforeEach(() => {
  resetPrototypeState();
  FakeObserver.all = [];
  window.IntersectionObserver = FakeObserver as unknown as typeof IntersectionObserver;
  revoke = vi.fn();
  URL.revokeObjectURL = revoke as unknown as typeof URL.revokeObjectURL;
  vi.mocked(getApi).mockReset();
  vi.mocked(getAuthedBlobUrl).mockReset();
  vi.mocked(getAuthedBlobUrl).mockImplementation(async (endpoint: string) => blobFor(endpoint));
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  // Unmount while the URL/observer stubs are still installed (hooks run in stack order,
  // so the global cleanup in setup.ts would otherwise run after they are restored).
  cleanup();
  window.IntersectionObserver = originalObserver;
  URL.revokeObjectURL = originalRevoke;
  vi.unstubAllGlobals();
  session.current = null;
});

const imgSrcs = (root: ParentNode = document) => [...root.querySelectorAll("img")].map((img) => img.getAttribute("src"));

describe("activity cards — real screenshot thumbnails (live)", () => {
  it("loads nothing until a card scrolls into view, then shows the thumb (or the full capture when there is no thumb)", async () => {
    goLive([newRow(), legacyRow(), screenOnlyRow()]);
    render(<ActivitiesPage />);

    expect(await screen.findAllByRole("button", { name: /^Open session:/ })).toHaveLength(3);
    // Lazy: two tiles with captures wait as skeletons; the legacy row is a glyph straight away.
    expect(screen.getAllByTestId("capture-thumb-skeleton")).toHaveLength(2);
    expect(screen.getByTitle("Rider")).toBeInTheDocument();
    expect(getAuthedBlobUrl).not.toHaveBeenCalled();

    scrollEverythingIntoView();

    await waitFor(() => expect(imgSrcs()).toEqual(expect.arrayContaining([blobFor(content(THUMB_NEW)), blobFor(content(SCREEN_ONLY))])));
    const requested = vi.mocked(getAuthedBlobUrl).mock.calls.map(([e]) => e);
    expect(requested).toContain(content(THUMB_NEW));
    expect(requested).not.toContain(content(SCREEN_NEW)); // the thumb loaded, so the full capture is never fetched for the tile
    expect(requested).not.toContain(content(WEBCAM_NEW));
    expect(screen.queryAllByTestId("capture-thumb-skeleton")).toHaveLength(0);

    // The card still names the block's main app (code.exe: 220s across two windows) over the image,
    // keeps its time badge, and flags the webcam capture.
    const card = screen.getAllByRole("button", { name: /^Open session:/ })[0];
    expect(within(card).getByText("code.exe")).toBeInTheDocument();
    expect(within(card).getByTitle("Webcam opt-in")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("falls back to the full capture when the thumb fails, and to the app glyph when nothing loads", async () => {
    vi.mocked(getAuthedBlobUrl).mockImplementation(async (endpoint: string) => {
      if (endpoint === content(THUMB_NEW) || endpoint === content(SCREEN_ONLY)) throw new Error("404");
      return blobFor(endpoint);
    });
    goLive([newRow(), screenOnlyRow()]);
    render(<ActivitiesPage />);
    const cards = await screen.findAllByRole("button", { name: /^Open session:/ });

    scrollEverythingIntoView();

    await waitFor(() => expect(imgSrcs()).toEqual([blobFor(content(SCREEN_NEW))]));
    expect(vi.mocked(getAuthedBlobUrl).mock.calls.map(([e]) => e)).toEqual(
      expect.arrayContaining([content(THUMB_NEW), content(SCREEN_NEW), content(SCREEN_ONLY)]),
    );
    // The screen-only block had nothing else to try: its tile is the app glyph again, not a skeleton.
    await waitFor(() => expect(within(cards[1]).queryByTestId("capture-thumb-skeleton")).toBeNull());
    expect(cards[1].querySelector("img")).toBeNull();
    expect(within(cards[1]).getAllByTitle("code.exe").length).toBeGreaterThan(0);
  }, RENDER_BUDGET_MS);

  it("never has more than four downloads in flight, and abandons them when the page goes away", async () => {
    const inFlight = new Set<string>();
    let peak = 0;
    vi.mocked(getAuthedBlobUrl).mockImplementation(
      (endpoint: string, init?: { signal?: AbortSignal }) =>
        new Promise<string>((_, reject) => {
          inFlight.add(endpoint);
          peak = Math.max(peak, inFlight.size);
          init?.signal?.addEventListener("abort", () => {
            inFlight.delete(endpoint);
            reject(new DOMException("aborted", "AbortError"));
          });
        }),
    );
    const rows = Array.from({ length: 7 }, (_, i) =>
      newRow({ id: guid(10 + i), ...times(10 + i * 10, 300), captures: [{ id: guid(500 + i), kind: "thumb" }] }),
    );
    goLive(rows);
    const { unmount } = render(<ActivitiesPage />);
    await screen.findAllByRole("button", { name: /^Open session:/ });

    scrollEverythingIntoView();
    await waitFor(() => expect(getAuthedBlobUrl).toHaveBeenCalledTimes(4));
    expect(peak).toBe(4);

    unmount();
    await waitFor(() => expect(inFlight.size).toBe(0));
    expect(getAuthedBlobUrl).toHaveBeenCalledTimes(4); // queued tiles never started
  }, RENDER_BUDGET_MS);

  it("the Screens grid shows the same real thumbnails", async () => {
    __setPathname("/activities?view=screens");
    goLive([newRow()]);
    render(<ActivitiesPage />);
    await waitFor(() => expect(FakeObserver.all.length).toBeGreaterThan(0));
    expect(screen.queryAllByRole("button", { name: /^Open session:/ })).toHaveLength(0); // the grid, not the session cards

    scrollEverythingIntoView();

    await waitFor(() => expect(imgSrcs()).toEqual([blobFor(content(THUMB_NEW))]));
    expect(screen.getByText("code.exe")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);
});

describe("activity drawer — the block report (live)", () => {
  it("fetches captures by PATH id and renders the capture, stats, timeline and apps table", async () => {
    const user = userEvent.setup();
    vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
      if (endpoint === `/api/app/activity/screenshots/${A_NEW}`) {
        return [
          { id: WEBCAM_NEW, activityId: A_NEW, kind: "webcam", blurred: false, capturedAt: "2026-09-29T10:03:00Z", sizeBytes: 9000, contentType: "image/jpeg" },
          { id: SCREEN_NEW, activityId: A_NEW, kind: "screen", blurred: true, capturedAt: "2026-09-29T10:02:00Z", sizeBytes: 250_000, contentType: "image/jpeg" },
        ];
      }
      throw new Error(`unexpected endpoint ${endpoint}`);
    });
    goLive([newRow()]);
    const { unmount } = render(<ActivitiesPage />);

    await user.click(await screen.findByRole("button", { name: /^Open session:/ }));
    const dialog = await screen.findByRole("dialog", { name: "Activity detail" });

    // The bug: the id travels in the path, never as ?activityId=.
    const calls = vi.mocked(getApi).mock.calls.map(([e]) => e);
    expect(calls).toEqual([`/api/app/activity/screenshots/${A_NEW}`]);
    expect(calls.some((e) => e.includes("?activityId="))).toBe(false);

    // Header: member, project, range, duration.
    expect(within(dialog).getByText("Wendy Worker")).toBeInTheDocument();
    expect(within(dialog).getByText("Apollo")).toBeInTheDocument();
    expect(within(dialog).getAllByText("4m 58s").length).toBeGreaterThan(0);

    // The full-size capture: opens in a new tab, downloadable; the webcam frame shows too.
    const open = await within(dialog).findByRole("link", { name: /open full size/i });
    expect(open).toHaveAttribute("href", blobFor(content(SCREEN_NEW)));
    expect(open).toHaveAttribute("target", "_blank");
    expect(open).toHaveAttribute("rel", expect.stringContaining("noopener"));
    const download = within(dialog).getByRole("link", { name: /download/i });
    expect(download).toHaveAttribute("download", expect.stringMatching(/^screen-.*\.jpg$/));
    expect(within(dialog).getByText(/blurred/)).toBeInTheDocument();
    expect(within(dialog).getByRole("heading", { name: "Webcam (opt-in)" })).toBeInTheDocument();
    expect(imgSrcs(dialog)).toContain(blobFor(content(WEBCAM_NEW)));

    // Stats.
    expect(within(dialog).getByText("4 of 5 min")).toBeInTheDocument();
    expect(within(dialog).getByText("82%")).toBeInTheDocument();
    expect(within(dialog).getByText("325")).toBeInTheDocument();

    // Timeline: an accessible summary, clock-time axis, and a minute-by-minute table.
    const chart = within(dialog).getByRole("group", { name: /^Input per minute/ });
    expect(chart).toHaveAccessibleName(expect.stringContaining("active 4 of 5 minutes"));
    expect(chart).toHaveAccessibleName(expect.stringContaining("325 keyboard presses and 21 mouse clicks"));
    const minuteTable = within(dialog).getByRole("table", { name: "Input per minute" });
    const minuteRows = within(minuteTable).getAllByRole("row").slice(1);
    expect(minuteRows).toHaveLength(5);
    expect(within(minuteRows[2]).getByText("Idle")).toBeInTheDocument();
    expect(within(minuteRows[0]).getByText(formatClock(new Date(Date.parse(newRow().startedAt)).toISOString()))).toBeInTheDocument();
    // Keyboard inspection: focus the chart and step through the minutes.
    act(() => chart.focus());
    fireEvent.keyDown(chart, { key: "ArrowRight" });
    expect(within(dialog).getByText(/: 120 keyboard, 5 mouse in code\.exe$/)).toBeInTheDocument();

    // Apps & windows: grouped by app, summed, share of the block, input per app and per window.
    const apps = within(dialog).getByRole("table", { name: /per app and window/ });
    const code = within(apps).getByRole("button", { name: /code\.exe/ });
    const codeRow = code.closest("tr")!;
    expect(within(codeRow).getByText("3m 40s")).toBeInTheDocument();
    expect(within(codeRow).getByText("74%")).toBeInTheDocument();
    expect(within(codeRow).getByText("320")).toBeInTheDocument();
    expect(within(codeRow).getByText("12")).toBeInTheDocument();
    const mainRs = within(apps).getByText("main.rs").closest("tr")!;
    expect(within(mainRs).getByText("3m")).toBeInTheDocument();
    expect(within(apps).getByText("lib.rs")).toBeInTheDocument();
    const chromeRow = within(apps).getByText("chrome.exe").closest("tr")!;
    expect(within(chromeRow).getByText("1m")).toBeInTheDocument();
    expect(within(chromeRow).getByText("20%")).toBeInTheDocument();
    // Multi-window apps collapse.
    expect(code).toHaveAttribute("aria-expanded", "true");
    await user.click(code);
    expect(code).toHaveAttribute("aria-expanded", "false");
    expect(within(apps).queryByText("main.rs")).not.toBeInTheDocument();

    // Running programs.
    expect(within(dialog).getByText("slack.exe")).toBeInTheDocument();

    // Object URLs are released when the page goes away.
    unmount();
    expect(revoke).toHaveBeenCalledWith(blobFor(content(SCREEN_NEW)));
    expect(revoke).toHaveBeenCalledWith(blobFor(content(WEBCAM_NEW)));
  }, RENDER_BUDGET_MS);

  it("a legacy row still reads cleanly: no timeline, credited focus time, — for unreported input", async () => {
    const user = userEvent.setup();
    vi.mocked(getApi).mockResolvedValue([]);
    goLive([legacyRow()]);
    render(<ActivitiesPage />);

    await user.click(await screen.findByRole("button", { name: /^Open session:/ }));
    const dialog = await screen.findByRole("dialog", { name: "Activity detail" });

    expect(await within(dialog).findByText("No screenshot captured")).toBeInTheDocument();
    expect(getApi).toHaveBeenCalledWith(`/api/app/activity/screenshots/${A_LEGACY}`);
    expect(within(dialog).queryByRole("heading", { name: "Activity timeline" })).not.toBeInTheDocument();
    expect(within(dialog).queryByText("Active minutes")).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("heading", { name: "Running programs" })).not.toBeInTheDocument();

    const apps = within(dialog).getByRole("table", { name: /per app and window/ });
    const rider = within(apps).getByText("Rider").closest("tr")!;
    expect(within(rider).getByText("10m")).toBeInTheDocument(); // the whole block, credited
    expect(within(rider).getByText("100%")).toBeInTheDocument();
    expect(within(rider).getAllByText("—")).toHaveLength(2); // keys + clicks not reported
    expect(within(dialog).getByText(/credited with the whole block/)).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("a11y: the full block report has no axe violations", async () => {
    const user = userEvent.setup();
    vi.mocked(getApi).mockResolvedValue([
      { id: SCREEN_NEW, activityId: A_NEW, kind: "screen", blurred: false, capturedAt: "2026-09-29T10:02:00Z", contentType: "image/jpeg" },
    ]);
    goLive([newRow()]);
    render(<ActivitiesPage />);

    await user.click(await screen.findByRole("button", { name: /^Open session:/ }));
    const dialog = await screen.findByRole("dialog", { name: "Activity detail" });
    await within(dialog).findByRole("link", { name: /open full size/i });

    expect(await axe(dialog)).toHaveNoViolations();
  }, RENDER_BUDGET_MS);

  it("says so when the captures can't be loaded", async () => {
    const user = userEvent.setup();
    vi.mocked(getApi).mockRejectedValue(new Error("API GET Error: Not Found"));
    goLive([newRow()]);
    render(<ActivitiesPage />);

    await user.click(await screen.findByRole("button", { name: /^Open session:/ }));
    const dialog = await screen.findByRole("dialog", { name: "Activity detail" });
    expect(await within(dialog).findByText("Couldn’t load captures.")).toBeInTheDocument();
    // The rest of the report doesn't depend on the captures.
    expect(within(dialog).getByRole("heading", { name: "Activity timeline" })).toBeInTheDocument();
  }, RENDER_BUDGET_MS);
});
