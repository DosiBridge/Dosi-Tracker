// "Week review" on the timesheet in a LIVE session (AI assist spec §18.5):
// below the heatmap, for the week on screen in the reporting zone — the
// team's for the owner and managers (their own when the server allows them
// only that), a member's own — asked only once the zone is known and only on
// Generate; stepping back a week reviews that week. The session and HTTP are
// mocked; no request reaches an AI provider.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
}));
const ai = vi.hoisted(() => ({ value: null as unknown }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/ai-provider")>()),
  useAiAssistant: () => ai.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import TimesheetPage from "@/app/(dashboard)/timesheet/page";
import { getApi } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import type { AiStatusDto } from "@/lib/ai-contract";
import { TIMESHEET_DATA_QUALITY_KINDS } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { timesheetWeek } from "@/lib/timesheet";
import type { User } from "@/lib/types";
import { aiContext, dataQualityResult, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const ZONE = "Asia/Dhaka";

const person = (role: User["role"]): User => ({
  id: `${role}-1`,
  name: "Wendy Worker",
  email: "wendy@real.example",
  role,
  designation: "Member",
  status: "offline",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "",
});

function serve(zone: Promise<unknown> = Promise.resolve({ timeZone: ZONE })) {
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return zone;
    if (endpoint === "/api/app/workspace/settings") return {};
    return [];
  });
}

function live(role: User["role"], status: AiStatusDto | null = demoAiStatus(role)) {
  localStorage.setItem("dosi-token", "live-token"); // referenceNow() → the real clock
  const user = person(role);
  session.current = { user, isLive: true, status: "ready" };
  ai.value = aiContext({ role, status, userId: user.id, workspaceId: "ws-live" });
}

const review = () => screen.findByRole("region", { name: "Week review" });
const weekOf = (offset = 0) => timesheetWeek(new Date(), ZONE, offset);

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
  serve();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(dataQualityResult());
});

describe("Week review", () => {
  it("reviews the team's week for the owner, below the heatmap, only on Generate", async () => {
    const user = userEvent.setup();
    live("owner");
    render(<TimesheetPage />);
    const card = await review();
    expect(screen.getByText("Team heatmap").compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(requestAiFeature).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "data-quality-review",
      {
        from: weekOf().from.toISOString(),
        to: weekOf().to.toISOString(),
        scope: "team",
        projectId: null,
        timeZone: ZONE,
        kinds: [...TIMESHEET_DATA_QUALITY_KINDS],
        regenerate: false,
      },
    ]);
    expect(await screen.findByText("Gap in tracked days")).toBeInTheDocument();
  });

  it("reviews a member's own week", async () => {
    const user = userEvent.setup();
    live("worker");
    render(<TimesheetPage />);
    await review();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toMatchObject({ scope: "self" });
  });

  it("reviews a manager's own week when the server leaves out the team scope", async () => {
    const user = userEvent.setup();
    const status = demoAiStatus("admin");
    live("admin", { ...status, features: status.features.map((f) => (f.featureId === "data-quality-review" ? { ...f, scopes: ["self"] } : f)) });
    render(<TimesheetPage />);
    await review();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toMatchObject({ scope: "self" });
  });

  it("follows the week on screen", async () => {
    const user = userEvent.setup();
    live("owner");
    render(<TimesheetPage />);
    await review();
    await user.click(screen.getByRole("button", { name: "Previous week" }));
    await user.click(await screen.findByRole("button", { name: "Generate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(1));
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toMatchObject({ from: weekOf(-1).from.toISOString(), to: weekOf(-1).to.toISOString() });
  });

  it("waits for the reporting zone", async () => {
    const zone = deferred<unknown>();
    serve(zone.promise);
    live("owner");
    render(<TimesheetPage />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "Week review" })).toBeNull();
    zone.resolve({ timeZone: ZONE });
    expect(await review()).toBeInTheDocument();
  });

  it("is hidden where the server doesn't allow it", async () => {
    live("worker", { ...demoAiStatus("worker"), available: false, reason: "self-service-off" });
    render(<TimesheetPage />);
    await screen.findByText("Your week");
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "Week review" })).toBeNull();
  });
});
