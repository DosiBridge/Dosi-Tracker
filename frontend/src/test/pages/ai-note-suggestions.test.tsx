// "Suggest a note" on the Activities page in a LIVE session (AI assist spec
// §3.1, §18.5): in a block's report, the note editor offers AI suggestions
// for that block; choosing one only fills the textarea — the note is stored
// (PUT /api/app/activity/{id}/note) only when the person presses "Save
// note". HTTP and AI calls are mocked; no request reaches an AI provider.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAuthedBlobUrl: vi.fn(),
  getAuthedBlob: vi.fn(),
  getTeamPresence: vi.fn(),
  updateActivityNote: vi.fn(),
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
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ActivitiesPage from "@/app/(dashboard)/activities/page";
import { getApi, getAuthedBlobUrl, getTeamPresence, updateActivityNote } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import type { AiNoteSuggestionsDto } from "@/lib/ai-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { mapApiActivities, mapApiProject, type ApiActivityDto } from "@/lib/live-dataset";
import { installLiveDataset } from "@/lib/tenant-data";
import type { User } from "@/lib/types";
import { aiContext, aiMeta } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";
const BLOCK = "aaaaaaaa-0000-0000-0000-000000000001";

const member = (id: string, name: string, role: User["role"]): User => ({
  id,
  name,
  email: `${id}@real.example`,
  role,
  designation: "Member",
  status: "offline",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "",
});
const owner = member(OWNER, "Olivia Owner", "owner");

function row(): ApiActivityDto {
  const end = Date.now() - 30 * 60_000;
  return {
    id: BLOCK,
    userId: WORKER,
    projectId: PROJECT,
    startedAt: new Date(end - 300_000).toISOString(),
    endedAt: new Date(end).toISOString(),
    productivity: 82,
    mouseClicks: 21,
    keyboardHits: 325,
    description: "Release notes",
    activeWindowsJson: JSON.stringify([{ AppName: "code.exe", WindowTitle: "main.rs", Seconds: 180 }]),
    runningProgramsJson: "[]",
  };
}

function json(body: unknown): Response {
  const text = JSON.stringify(body);
  return { ok: true, status: 200, statusText: "OK", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

const SUGGESTIONS: AiNoteSuggestionsDto = {
  meta: aiMeta("note-suggestions"),
  suggestions: [
    { text: "Worked on Apollo in VS Code", basis: "apps" },
    { text: "Worked on Apollo", basis: "project" },
    { text: "Release notes", basis: "previous-note" },
  ],
};

beforeEach(() => {
  resetPrototypeState();
  localStorage.setItem("dosi-token", "live-token");
  const rows = [row()];
  installLiveDataset({
    workspaceId: "live-activities",
    users: [owner, member(WORKER, "Wendy Worker", "worker")],
    projects: [mapApiProject({ id: PROJECT, title: "Apollo", color: "#0d9488" }, [WORKER])],
    activities: mapApiActivities(rows),
    now: new Date(),
  });
  session.current = { user: owner, isLive: true, status: "ready" };
  ai.value = aiContext({ userId: OWNER, workspaceId: "live-activities" });
  vi.stubGlobal("fetch", vi.fn<typeof fetch>().mockImplementation(async () => json({ items: rows, totalCount: rows.length })));
  vi.mocked(getApi).mockReset();
  vi.mocked(getApi).mockImplementation(async (endpoint: string) => {
    if (endpoint === "/api/app/workspace/my-preferences") return { timeZone: "UTC" };
    if (endpoint === "/api/app/workspace/settings") return {};
    return [];
  });
  vi.mocked(getAuthedBlobUrl).mockReset();
  vi.mocked(getTeamPresence).mockReset();
  vi.mocked(getTeamPresence).mockResolvedValue([]);
  vi.mocked(updateActivityNote).mockReset();
  vi.mocked(updateActivityNote).mockResolvedValue({ ...row(), description: "Worked on Apollo" } as never);
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(SUGGESTIONS);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function openNoteEditor(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: /^Open session:/ }));
  const dialog = await screen.findByRole("dialog", { name: "Activity detail" });
  await user.click(within(dialog).getByRole("button", { name: "Edit note" }));
  return dialog;
}

describe("suggesting a block's note (live)", () => {
  it("fills the textarea with the chosen suggestion; the note is saved only on Save note", async () => {
    const user = userEvent.setup();
    render(<ActivitiesPage />);
    const dialog = await openNoteEditor(user);
    expect(requestAiFeature).not.toHaveBeenCalled();

    await user.click(await within(dialog).findByRole("button", { name: "Suggest" }));
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual(["note-suggestions", { activityId: BLOCK }]);
    const chips = await within(dialog).findByRole("group", { name: "Suggested notes" });
    await user.click(within(chips).getByRole("button", { name: "Worked on Apollo" }));
    expect(within(dialog).getByRole("textbox", { name: "Task note" })).toHaveValue("Worked on Apollo");
    expect(updateActivityNote).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Save note" }));
    expect(updateActivityNote).toHaveBeenCalledTimes(1);
    expect(updateActivityNote).toHaveBeenCalledWith(BLOCK, "Worked on Apollo");
    expect(await within(dialog).findByText("Note saved.")).toBeInTheDocument();
  }, 20_000);

  it("offers no suggestions when the server doesn't allow them, and the editor works as before", async () => {
    const user = userEvent.setup();
    const status = demoAiStatus("owner");
    ai.value = aiContext({ status: { ...status, features: status.features.map((f) => (f.featureId === "note-suggestions" ? { ...f, allowed: false } : f)) } });
    render(<ActivitiesPage />);
    const dialog = await openNoteEditor(user);
    await new Promise((r) => setTimeout(r, 20));
    expect(within(dialog).queryByRole("button", { name: "Suggest" })).toBeNull();
    await user.type(within(dialog).getByRole("textbox", { name: "Task note" }), " v2{Enter}");
    expect(updateActivityNote).toHaveBeenCalledWith(BLOCK, "Release notes v2");
  }, 20_000);
});
