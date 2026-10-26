// "Explain this day" on the Member Monitor in a LIVE session (AI assist spec
// §18.5): the card sits above the activity band of the detail panel, only
// once the selected day has loaded — never while it is loading or after a
// failed read — and asks for exactly that member and day (a member's own day
// as the caller). The session and HTTP are mocked; no request reaches an AI
// provider.
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
const ai = vi.hoisted(() => ({ value: null as unknown }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/ai-provider")>()),
  useAiAssistant: () => ai.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
}));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import MonitorPage from "@/app/(dashboard)/monitor/page";
import { getAllPagesApi, getApi, getTeamPresence } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import { demoAiStatus } from "@/lib/ai-demo-status";
import type { ApiActivityDto } from "@/lib/live-dataset";
import { installLiveDataset } from "@/lib/tenant-data";
import type { User } from "@/lib/types";
import { aiContext, dayResult, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const RENDER_BUDGET_MS = 20_000;
const today = new Date().toISOString().slice(0, 10);

function liveUser(id: string, name: string, role: User["role"]): User {
  return { id, name, email: `${id}@real.example`, role, designation: role === "worker" ? "Member" : "Workspace Owner", status: "offline", timezone: "UTC", trackedToday: 0, productivity: 0, joinedAt: "" };
}

const owner = liveUser("owner-1", "Olivia Owner", "owner");
const worker = liveUser("worker-1", "Wendy Worker", "worker");

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
  } as ApiActivityDto;
}

/** Each request for the selected day settles as `day()` does; the reporting zone is UTC. */
function serveDay(day: () => Promise<ApiActivityDto[]>) {
  vi.mocked(getAllPagesApi).mockImplementation(async () => {
    const rows = await day();
    return { items: rows, totalCount: rows.length, truncated: false };
  });
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return { timeZone: "UTC" };
    if (endpoint === "/api/app/workspace/settings") return {};
    throw new Error(`unexpected endpoint ${endpoint}`);
  });
}

function goLive(self: User, members: User[]) {
  localStorage.setItem("dosi-token", "live-token");
  installLiveDataset({ workspaceId: "live-monitor", users: members, projects: [], activities: [], now: new Date() });
  session.current = { user: self, isLive: true, status: "ready" };
  ai.value = aiContext({ role: self.role, userId: self.id, workspaceId: "live-monitor", status: demoAiStatus(self.role) });
}

const card = () => screen.queryByRole("region", { name: "Explain this day" });

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
  vi.mocked(getAllPagesApi).mockReset();
  vi.mocked(getTeamPresence).mockReset();
  vi.mocked(getTeamPresence).mockResolvedValue([]);
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(dayResult());
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  session.current = null;
});

describe("monitor — Explain this day", () => {
  it("appears for a member's own day only after the live day loaded, and asks for it as the caller", async () => {
    const user = userEvent.setup();
    goLive(worker, [worker]);
    const day = deferred<ApiActivityDto[]>();
    serveDay(() => day.promise);
    render(<MonitorPage />);

    expect(await screen.findByText(/Loading live activity/)).toBeInTheDocument();
    expect(card()).toBeNull();

    day.resolve([row()]);
    expect(await screen.findByRole("region", { name: "Explain this day" })).toBeInTheDocument();
    // Above the activity band.
    const band = screen.getByText("Activity overview");
    expect(card()!.compareDocumentPosition(band) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "member-day-summary",
      { userId: null, date: today, timeZone: "UTC", regenerate: false },
    ]);
  }, RENDER_BUDGET_MS);

  it("never appears when the day couldn't be loaded", async () => {
    goLive(worker, [worker]);
    serveDay(() => Promise.reject(new Error("offline")));
    render(<MonitorPage />);
    expect(await screen.findByText("Couldn't load activity")).toBeInTheDocument();
    expect(card()).toBeNull();
  }, RENDER_BUDGET_MS);

  it("explains the member an owner opens, by that member's id", async () => {
    const user = userEvent.setup();
    goLive(owner, [owner, worker]);
    serveDay(async () => [row()]);
    render(<MonitorPage />);

    await user.click(await screen.findByRole("button", { name: /Wendy Worker/ }));
    expect(await screen.findByRole("region", { name: "Explain this day" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toEqual({ userId: worker.id, date: today, timeZone: "UTC", regenerate: false });
  }, RENDER_BUDGET_MS);

  it("is not offered on the roster, only on a member's day", async () => {
    goLive(owner, [owner, worker]);
    serveDay(async () => [row()]);
    render(<MonitorPage />);
    expect(await screen.findByRole("button", { name: /Wendy Worker/ })).toBeInTheDocument();
    expect(card()).toBeNull();
  }, RENDER_BUDGET_MS);

  it("is hidden when the server doesn't allow day summaries", async () => {
    goLive(worker, [worker]);
    ai.value = aiContext({ role: "worker", userId: worker.id, status: null });
    serveDay(async () => [row()]);
    render(<MonitorPage />);
    expect(await screen.findByText("Activity overview")).toBeInTheDocument();
    expect(card()).toBeNull();
  }, RENDER_BUDGET_MS);
});
