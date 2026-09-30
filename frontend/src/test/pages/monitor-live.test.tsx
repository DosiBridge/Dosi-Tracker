// Member Monitor in a LIVE session (backend token present): the page must show
// only the tenant's real rows or honest empty / error states — never the demo
// people or generated timelines. It also must not crash on partial real data
// (the "Cannot read properties of undefined (reading 'id')" regression: a live
// tenant with no projects made the demo generator read `projects[0].id`).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAuthedBlobUrl: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import MonitorPage from "@/app/(dashboard)/monitor/page";
import { getApi } from "@/hooks/useApi";
import { mapApiActivities, type ApiActivityDto } from "@/lib/live-dataset";
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

function row(over: Partial<ApiActivityDto> = {}): ApiActivityDto {
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
  };
}

/** Route getApi by endpoint: the day's rows, and no screenshots. */
function serveDay(rows: ApiActivityDto[] | Error) {
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint.startsWith("/api/app/activity/screenshots")) return [];
    if (endpoint.startsWith("/api/app/activity?")) {
      if (rows instanceof Error) throw rows;
      return rows;
    }
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

function expectNoDemoPeople() {
  for (const name of DEMO_NAMES) expect(screen.queryByText(name)).not.toBeInTheDocument();
}

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  session.current = null;
});

describe("monitor — live session", () => {
  it("shows a worker only their own real day, fetched for that member", async () => {
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

    const dayCall = vi.mocked(getApi).mock.calls.map(([e]) => e).find((e) => e.startsWith("/api/app/activity?"));
    expect(dayCall).toContain(`From=${encodeURIComponent(`${today}T00:00:00.000Z`)}`);
    expect(dayCall).toContain(`UserId=${worker.id}`);
    expect(dayCall).toContain("MaxResultCount=1000");
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
  }, RENDER_BUDGET_MS);
});
