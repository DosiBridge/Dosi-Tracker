// Member Monitor in a LIVE session (backend token present): the page must show
// only the tenant's real rows or honest empty / error states — never the demo
// people or generated timelines. It also must not crash on partial real data
// (the "Cannot read properties of undefined (reading 'id')" regression: a live
// tenant with no projects made the demo generator read `projects[0].id`).
//
// Who is working NOW comes from the agents' heartbeats (presence/team), the
// day is a calendar day of the reporting zone read page by page, apps come
// from the per-window seconds, and the page refreshes every 30 s while the tab
// is visible.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAuthedBlobUrl: vi.fn(),
  getAllPagesApi: vi.fn(),
  getTeamPresence: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import MonitorPage from "@/app/(dashboard)/monitor/page";
import { ApiError, getAllPagesApi, getApi, getAuthedBlobUrl, getTeamPresence } from "@/hooks/useApi";
import { mapApiActivities, type ApiActivityDto } from "@/lib/live-dataset";
import type { PresenceDto } from "@/lib/api-contract";
import { formatClock } from "@/lib/activity-report";
import { installLiveDataset } from "@/lib/tenant-data";
import type { User } from "@/lib/types";
import { resetPrototypeState } from "@/test/harness";

const DEMO_NAMES = ["Ayesha Rahman", "Tanvir Hasan", "Nusrat Jahan", "Rafiq Islam", "Sadia Akter", "Imran Kabir", "David Chen"];
const RENDER_BUDGET_MS = 20_000;

const today = new Date().toISOString().slice(0, 10);

function liveUser(id: string, name: string, role: User["role"]): User {
  return {
    id,
    name,
    email: `${id}@real.example`,
    role,
    designation: role === "worker" ? "Member" : "Workspace Owner",
    status: "offline",
    timezone: "UTC",
    trackedToday: 95, // non-zero: the old demo generator would try to build a day for them
    productivity: 70,
    joinedAt: "",
  };
}

const owner = liveUser("owner-1", "Olivia Owner", "owner");
const worker = liveUser("worker-1", "Wendy Worker", "worker");
const colleague = liveUser("worker-2", "Carl Colleague", "worker");

function row(over: Partial<ApiActivityDto> & Record<string, unknown> = {}): ApiActivityDto {
  return {
    id: "act-1",
    userId: worker.id,
    projectId: "proj-1",
    startedAt: `${today}T09:00:00Z`,
    endedAt: `${today}T09:10:00Z`,
    productivity: 82,
    mouseClicks: 40,
    keyboardHits: 900,
    description: "Real session",
    activeWindowsJson: JSON.stringify([{ appName: "Rider", windowTitle: "Worker.cs", seconds: 600 }]),
    ...over,
  } as ApiActivityDto;
}

/** Day requests the page made: the endpoint and its paging options. */
function dayRequests() {
  return vi.mocked(getAllPagesApi).mock.calls.map(([endpoint, options]) => ({ url: new URL(`https://x${endpoint}`), options }));
}

/**
 * Serve the day (every page of it) and the reporting zone. Screenshots come
 * from the rows' own capture refs: the page never asks screenshots/{id} per block.
 */
function serveDay(rows: ApiActivityDto[] | Error, options: { zone?: string; totalCount?: number; truncated?: boolean } = {}) {
  vi.mocked(getAllPagesApi).mockImplementation(async () => {
    if (rows instanceof Error) throw rows;
    return { items: rows, totalCount: options.totalCount ?? rows.length, truncated: options.truncated ?? false };
  });
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return { timeZone: options.zone ?? "UTC" };
    if (endpoint === "/api/app/workspace/settings") return {};
    throw new Error(`unexpected endpoint ${endpoint}`);
  });
}

function goLive(self: User, members: User[], sessionRows: ApiActivityDto[] = []) {
  localStorage.setItem("dosi-token", "live-token"); // referenceNow() → the real clock
  installLiveDataset({
    workspaceId: "live-monitor",
    users: members,
    projects: [], // a tenant without (loadable) projects — the original crash
    activities: mapApiActivities(sessionRows),
    now: new Date(),
  });
  session.current = { user: self, isLive: true, status: "ready" };
}

function heartbeat(over: Partial<PresenceDto> = {}): PresenceDto {
  return {
    userId: worker.id,
    userName: "wendy",
    deviceId: "d1",
    status: "tracking",
    online: true,
    lastSeenAt: new Date().toISOString(),
    ...over,
  };
}

function expectNoDemoPeople() {
  for (const name of DEMO_NAMES) expect(screen.queryByText(name)).not.toBeInTheDocument();
}

/** Every observed element is "in view" at once, so lazy thumbnails load. */
class InViewObserver {
  constructor(private readonly callback: IntersectionObserverCallback) {}
  observe(target: Element) {
    this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

const originalObserver = window.IntersectionObserver;

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
  vi.mocked(getAllPagesApi).mockReset();
  vi.mocked(getAuthedBlobUrl).mockReset();
  vi.mocked(getTeamPresence).mockReset();
  vi.mocked(getTeamPresence).mockResolvedValue([]);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  session.current = null;
  window.IntersectionObserver = originalObserver;
});

describe("monitor — live session", () => {
  it("shows a worker only their own real day, read page by page for that member", async () => {
    goLive(worker, [worker]);
    serveDay([row(), row({ id: "act-2", startedAt: `${today}T09:10:00Z`, endedAt: `${today}T09:20:00Z`, activeWindowsJson: "{not json" })]);

    render(<MonitorPage />);

    expect(await screen.findByText("Worker.cs")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "My day" })).toBeInTheDocument();
    expect(screen.getAllByText("Rider").length).toBeGreaterThan(0);
    // Malformed window JSON degrades to an honest label instead of crashing.
    expect(screen.getAllByText("Tracked activity").length).toBeGreaterThan(0);
    expect(screen.getByText("2 entries")).toBeInTheDocument();
    expectNoDemoPeople();

    const [day] = dayRequests();
    expect(day.url.pathname).toBe("/api/app/activity");
    expect(day.url.searchParams.get("From")).toBe(`${today}T00:00:00.000Z`);
    expect(day.url.searchParams.get("UserId")).toBe(worker.id);
    expect(day.options).toMatchObject({ pageSize: 1000, maxItems: 5000 });
  }, RENDER_BUDGET_MS);

  it("shows each block's screenshot from its own capture refs — thumbnail first, lazily — and never a webcam frame", async () => {
    window.IntersectionObserver = InViewObserver as unknown as typeof IntersectionObserver;
    goLive(worker, [worker]);
    const capturedAt = `${today}T09:04:00Z`;
    serveDay([
      row({
        captures: [
          { id: "shot-1", kind: "screen", capturedAt, blurred: true },
          { id: "thumb-1", kind: "thumb", capturedAt, blurred: true },
          { id: "cam-1", kind: "webcam", capturedAt },
        ],
      }),
    ]);
    vi.mocked(getAuthedBlobUrl).mockImplementation(async (endpoint: string) => `blob:${endpoint}`);
    const originalRevoke = URL.revokeObjectURL; // jsdom has none; the tiles revoke on unmount
    const revoke = vi.fn();
    URL.revokeObjectURL = revoke;

    const { unmount } = render(<MonitorPage />);

    const shot = await screen.findByRole("img", { name: `Screenshot at ${formatClock(capturedAt, "UTC")}` });
    expect(shot).toHaveAttribute("src", "blob:/api/app/activity/screenshot/thumb-1/content");
    const requested = vi.mocked(getAuthedBlobUrl).mock.calls.map(([e]) => e);
    expect(requested).not.toContain("/api/app/activity/screenshot/cam-1/content");
    expect(requested).not.toContain("/api/app/activity/screenshot/shot-1/content"); // the thumbnail was enough
    // No sequential per-block metadata round trips any more.
    expect(vi.mocked(getApi).mock.calls.some(([e]) => e.startsWith("/api/app/activity/screenshots"))).toBe(false);
    expect(screen.getByText("Blurred")).toBeInTheDocument();
    unmount();
    expect(revoke).toHaveBeenCalledWith("blob:/api/app/activity/screenshot/thumb-1/content");
    URL.revokeObjectURL = originalRevoke;
  }, RENDER_BUDGET_MS);

  it("tells a member with no activity at all to install the desktop agent", async () => {
    goLive(worker, [worker]);
    serveDay([]);

    render(<MonitorPage />);

    expect(await screen.findByText("No activity yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Get the desktop agent" })).toBeInTheDocument();
    expectNoDemoPeople();
  }, RENDER_BUDGET_MS);

  it("shows an error with Retry when the day can't be loaded — never demo data — and recovers", async () => {
    const user = userEvent.setup();
    goLive(worker, [worker]);
    serveDay(new Error("offline"));

    render(<MonitorPage />);

    expect(await screen.findByText("Couldn't load activity")).toBeInTheDocument();
    expectNoDemoPeople();
    expect(screen.queryByText(/entries$/)).not.toBeInTheDocument();

    serveDay([row()]);
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Worker.cs")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("lists the real team for a manager and opens a member's real day", async () => {
    const user = userEvent.setup();
    goLive(owner, [owner, worker, colleague], [row()]);
    serveDay([row()]);

    render(<MonitorPage />);

    expect(await screen.findByText("Wendy Worker")).toBeInTheDocument();
    expect(screen.getByText("Carl Colleague")).toBeInTheDocument();
    expect(screen.getByText("Olivia Owner")).toBeInTheDocument();
    expect(screen.getByText("3 of 3 members · click a row for the full day")).toBeInTheDocument();
    expectNoDemoPeople();

    await user.click(screen.getByRole("button", { name: /Wendy Worker/ }));
    expect(await screen.findByText("Worker.cs")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Wendy Worker" })).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("shows a manager an install prompt instead of a roster when nothing was ever tracked", async () => {
    goLive(owner, [owner, worker]);
    serveDay([]);

    render(<MonitorPage />);

    expect(await screen.findByText("No activity yet")).toBeInTheDocument();
    const kpis = screen.getByText("Team size").closest("div")!.parentElement!;
    expect(within(kpis).getByText("2")).toBeInTheDocument();
    expectNoDemoPeople();
  }, RENDER_BUDGET_MS);

  it("reads the day in the reporting zone and says so", async () => {
    goLive(worker, [worker]);
    serveDay([row()], { zone: "Asia/Dhaka" });

    render(<MonitorPage />);

    expect(await screen.findByText(/times in Dhaka \(UTC\+6\)/)).toBeInTheDocument();
    const [day] = dayRequests();
    // A Dhaka day starts at 18:00 UTC the evening before.
    expect(day.url.searchParams.get("From")).toMatch(/T18:00:00\.000Z$/);
    expect(Date.parse(day.url.searchParams.get("To")!) - Date.parse(day.url.searchParams.get("From")!)).toBe(24 * 3_600_000);
  }, RENDER_BUDGET_MS);
});

describe("monitor — who is working now (presence/team)", () => {
  it("shows each member's real state, what they're in and since when, and filters by it", async () => {
    const user = userEvent.setup();
    goLive(owner, [owner, worker, colleague], [row()]);
    serveDay([row()]);
    vi.mocked(getTeamPresence).mockResolvedValue([
      heartbeat({ currentApp: "Code.exe", statusSince: `${today}T09:05:00Z`, projectTitle: "Apollo" }),
      heartbeat({ userId: colleague.id, userName: "carl", deviceId: "d2", status: "break" }),
    ]);

    render(<MonitorPage />);

    const wendy = (await screen.findByText("Wendy Worker")).closest("button")!;
    await waitFor(() => expect(within(wendy).getByText("Tracking")).toBeInTheDocument());
    expect(within(wendy).getByText(`In Visual Studio Code · since ${formatClock(`${today}T09:05:00Z`, "UTC")}`)).toBeInTheDocument();
    const carl = screen.getByText("Carl Colleague").closest("button")!;
    expect(within(carl).getByText("On a break")).toBeInTheDocument();
    const olivia = screen.getByText("Olivia Owner").closest("button")!;
    expect(within(olivia).getByText("Offline")).toBeInTheDocument();
    expect(screen.getByText("Tracking now · 1 away")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Away" }));
    expect(screen.getByText("1 of 3 members · click a row for the full day")).toBeInTheDocument();
    expect(screen.queryByText("Wendy Worker")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Tracking" }));
    await user.click(screen.getByRole("button", { name: /Wendy Worker/ }));
    // The member's own page says what the agent reports right now.
    expect(await screen.findByText("Apollo", { exact: false })).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("falls back to recent uploads on a server without presence, with a window of two project intervals", async () => {
    goLive(worker, [worker]);
    const recent = new Date(Date.now() - 5 * 60_000);
    serveDay([row({ startedAt: new Date(recent.getTime() - 10 * 60_000).toISOString(), endedAt: recent.toISOString() })]);
    vi.mocked(getTeamPresence).mockRejectedValue(
      new ApiError("API GET Error: Not Found", 404, { code: null, message: null, details: null, validationErrors: [] }),
    );

    render(<MonitorPage />);

    expect(await screen.findByText(/from recent uploads/)).toBeInTheDocument();
    expect(screen.getAllByText("Tracking").length).toBeGreaterThan(0);
    // An older server is asked once, not every 30 seconds.
    expect(getTeamPresence).toHaveBeenCalledTimes(1);
  }, RENDER_BUDGET_MS);

  it("refreshes presence and today's blocks every 30 s while the tab is visible, and pauses while it is hidden", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let visibility: DocumentVisibilityState = "visible";
    const visibilitySpy = vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
    try {
      goLive(worker, [worker]);
      serveDay([row()]);
      render(<MonitorPage />);
      await screen.findByText("Worker.cs");
      const presenceCalls = () => vi.mocked(getTeamPresence).mock.calls.length;
      const dayCalls = () => vi.mocked(getAllPagesApi).mock.calls.length;
      const [p0, d0] = [presenceCalls(), dayCalls()];

      await act(async () => {
        vi.advanceTimersByTime(30_000);
      });
      await waitFor(() => expect(presenceCalls()).toBe(p0 + 1));
      await waitFor(() => expect(dayCalls()).toBe(d0 + 1));

      visibility = "hidden";
      act(() => document.dispatchEvent(new Event("visibilitychange")));
      await act(async () => {
        vi.advanceTimersByTime(120_000);
      });
      expect(presenceCalls()).toBe(p0 + 1);
      expect(dayCalls()).toBe(d0 + 1);

      // Back after more than an interval: refreshed at once.
      visibility = "visible";
      act(() => document.dispatchEvent(new Event("visibilitychange")));
      await waitFor(() => expect(presenceCalls()).toBe(p0 + 2));
      expect(screen.getByText(/^Updated /)).toBeInTheDocument();
    } finally {
      visibilitySpy.mockRestore();
      vi.useRealTimers();
    }
  }, RENDER_BUDGET_MS);
});

describe("monitor — the member's day (live)", () => {
  it("credits each app with the seconds it was in front, not the whole block to its top app", async () => {
    goLive(worker, [worker]);
    const windows = JSON.stringify([
      { appName: "Code.exe", windowTitle: "api.rs", seconds: 360 },
      { appName: "slack.exe", windowTitle: "#dev", seconds: 240 },
    ]);
    serveDay([
      row({ activeWindowsJson: windows }),
      row({ id: "act-2", startedAt: `${today}T09:10:00Z`, endedAt: `${today}T09:20:00Z`, activeWindowsJson: windows }),
    ]);

    render(<MonitorPage />);

    const apps = (await screen.findByRole("heading", { name: "Apps used" })).closest("div")!.parentElement!;
    expect(within(apps).getByText("Visual Studio Code")).toBeInTheDocument();
    expect(within(apps).getByText("12m")).toBeInTheDocument();
    expect(within(apps).getByText("Slack")).toBeInTheDocument();
    expect(within(apps).getByText("8m")).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("shows activity as not measured when the agent couldn't measure it, and the idle time it reported", async () => {
    goLive(worker, [worker]);
    serveDay([row({ productivity: null, idleSeconds: 240, endReason: "idle", description: "Planning" })]);

    render(<MonitorPage />);

    const activityKpi = (await screen.findByText("Activity")).parentElement!;
    expect(within(activityKpi).getByText("Not measured")).toBeInTheDocument();
    expect(screen.getByText("Idle while tracked")).toBeInTheDocument();
    expect(screen.getAllByText("4m idle").length).toBeGreaterThan(0);
    expect(screen.getByText("Idle")).toBeInTheDocument(); // the end-reason chip
    expect(screen.getByText("Planning")).toBeInTheDocument(); // the task note
  }, RENDER_BUDGET_MS);

  it("says when a day had more blocks than it read", async () => {
    goLive(worker, [worker]);
    serveDay([row()], { totalCount: 6000, truncated: true });

    render(<MonitorPage />);

    expect(await screen.findByText(/Showing the first 1 of 6,000 blocks recorded on this day/)).toBeInTheDocument();
  }, RENDER_BUDGET_MS);

  it("a11y: the member's day has no axe violations", async () => {
    goLive(worker, [worker]);
    serveDay([row()]);
    vi.mocked(getTeamPresence).mockResolvedValue([heartbeat({ currentApp: "Rider" })]);
    const { container } = render(<MonitorPage />);
    await screen.findByText("Worker.cs");
    expect(await axe(container)).toHaveNoViolations();
  }, RENDER_BUDGET_MS);
});

describe("monitor — demo session (no token)", () => {
  it("still renders the seeded demo day", () => {
    session.current = {
      user: { ...liveUser("u2", "Tanvir Hasan", "worker") },
      isLive: false,
      status: "demo",
    };

    render(<MonitorPage />);

    expect(screen.getByRole("heading", { name: "My day" })).toBeInTheDocument();
    expect(screen.getByText(/\d+ entries/)).toBeInTheDocument();
    expect(getApi).not.toHaveBeenCalled();
    expect(getTeamPresence).not.toHaveBeenCalled();
    expect(getAllPagesApi).not.toHaveBeenCalled();
  }, RENDER_BUDGET_MS);
});
