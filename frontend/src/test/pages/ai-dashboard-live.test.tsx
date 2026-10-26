// AI assist on the dashboards in a LIVE session (AI assist spec §18.5): the
// owner and managers get "This week at a glance" for the team below the KPI
// row (their own week when the server allows them only "self"), a member
// gets "My week" after "Your week"; the week is Monday to today in the
// reporting zone, and nothing is asked before that zone is known. While the
// workspace hasn't opted in, the owner is invited to turn it on and a
// manager learns the owner can. The session and HTTP are mocked; no request
// ever reaches an AI provider.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getHourlyReport: vi.fn(),
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

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AdminDashboard } from "@/components/dashboard/admin-dashboard";
import { WorkerDashboard } from "@/components/dashboard/worker-dashboard";
import { getApi, getHourlyReport, getTeamPresence } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import { clearStoredChoices } from "@/hooks/use-stored-choice";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { zonedPeriod } from "@/lib/time-zone";
import type { AiStatusDto } from "@/lib/ai-contract";
import type { User } from "@/lib/types";
import { aiContext, aiStatusWith, deferred, periodResult } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const ZONE = "Asia/Dhaka";

const member: User = {
  id: "worker-1",
  name: "Wendy Worker",
  email: "wendy@real.example",
  role: "worker",
  designation: "Member",
  status: "offline",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "",
};

/** The reporting zone loads when `zone` resolves; every report request stays pending (the AI card doesn't need them). */
function serve(zone: Promise<unknown> = Promise.resolve({ timeZone: ZONE })) {
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return zone;
    if (endpoint === "/api/app/workspace/settings") return {};
    return new Promise(() => {});
  });
  vi.mocked(getHourlyReport).mockReturnValue(new Promise(() => {}));
  vi.mocked(getTeamPresence).mockReturnValue(new Promise(() => {}));
}

function live(role: User["role"], status: AiStatusDto | null = demoAiStatus(role)) {
  localStorage.setItem("dosi-token", "live-token"); // referenceNow() → the real clock
  session.current = { isLive: true, liveCoverage: null, user: { ...member, role }, workspace: { id: "ws-live" } };
  ai.value = aiContext({ role, status, userId: member.id, workspaceId: "ws-live" });
}

const week = () => zonedPeriod("week", new Date(), ZONE);

beforeEach(() => {
  resetPrototypeState();
  clearStoredChoices();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(periodResult());
  serve();
});

describe("owner and manager dashboard", () => {
  it("offers the team's week below the KPI row, for Monday to today in the reporting zone", async () => {
    const user = userEvent.setup();
    live("owner");
    render(<AdminDashboard userName="Olivia" isOwner />);
    const card = await screen.findByRole("region", { name: "This week at a glance" });
    // Below the KPI row: after the "Tracked this week" tile.
    expect(screen.getByText("Tracked this week").compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "period-summary",
      { from: week().from.toISOString(), to: week().to.toISOString(), scope: "team", projectId: null, timeZone: ZONE, regenerate: false },
    ]);
    expect(await screen.findByText("The team tracked 38 h 12 min this week.")).toBeInTheDocument();
  });

  it("waits for the reporting zone before offering the card", async () => {
    const zone = deferred<unknown>();
    serve(zone.promise);
    live("owner");
    render(<AdminDashboard userName="Olivia" isOwner />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "This week at a glance" })).toBeNull();
    zone.resolve({ timeZone: ZONE });
    expect(await screen.findByRole("region", { name: "This week at a glance" })).toBeInTheDocument();
  });

  it("gives a manager without the team scope their own week instead", async () => {
    const user = userEvent.setup();
    const status = demoAiStatus("admin");
    live("admin", { ...status, features: status.features.map((f) => (f.featureId === "period-summary" ? { ...f, scopes: ["self"] } : f)) });
    render(<AdminDashboard userName="Max" isOwner={false} />);
    await screen.findByRole("region", { name: "My week" });
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toMatchObject({ scope: "self" });
  });

  it("invites the owner to turn AI assist on while the workspace hasn't opted in, with no summary card", async () => {
    live("owner", aiStatusWith("workspace-disabled", { workspaceEnabled: false }));
    render(<AdminDashboard userName="Olivia" isOwner />);
    expect(await screen.findByRole("link", { name: "Turn on AI assist" })).toHaveAttribute("href", "/settings?tab=workspace");
    expect(screen.queryByRole("region", { name: "This week at a glance" })).toBeNull();
  });

  it("tells a manager the owner can turn it on", async () => {
    live("admin", aiStatusWith("workspace-disabled", { canManage: false }));
    render(<AdminDashboard userName="Max" isOwner={false} />);
    expect(await screen.findByText("Your workspace owner can turn on AI assist")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Turn on AI assist" })).toBeNull();
  });

  it("shows nothing about AI on a server without it", async () => {
    live("owner", null);
    render(<AdminDashboard userName="Olivia" isOwner />);
    await waitFor(() => expect(getApi).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "This week at a glance" })).toBeNull();
    expect(screen.queryByText(/turn on AI assist/i)).toBeNull();
  });
});

describe("member dashboard", () => {
  it("offers My week after Your week, asking for the member's own week", async () => {
    const user = userEvent.setup();
    live("worker");
    render(<WorkerDashboard user={member} />);
    const card = await screen.findByRole("region", { name: "My week" });
    expect(screen.getByText("Your week").compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toEqual({
      from: week().from.toISOString(),
      to: week().to.toISOString(),
      scope: "self",
      projectId: null,
      timeZone: ZONE,
      regenerate: false,
    });
  });

  it("is hidden when the owner switched member self-service off", async () => {
    live("worker", aiStatusWith("self-service-off"));
    render(<WorkerDashboard user={member} />);
    await waitFor(() => expect(getApi).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "My week" })).toBeNull();
    // A member never sees the owner's invitation either.
    expect(screen.queryByText(/turn on AI assist/i)).toBeNull();
  });
});
