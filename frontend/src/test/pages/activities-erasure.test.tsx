// The activity drawer's deletions (E2E-07) and unmeasured input (API-05), live:
// owners and managers delete a block (with a reason) or one capture; a member
// deletes their own capture within the workspace's window, reason optional;
// a block whose keyboard and mouse weren't counted reads "Not measured".
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getApi: vi.fn(),
  getAuthedBlobUrl: vi.fn(),
  getAuthedBlob: vi.fn(),
  getTeamPresence: vi.fn(),
  getWorkspaceSettings: vi.fn(),
  deleteActivityApi: vi.fn(),
  deleteCaptureApi: vi.fn(),
}));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import ActivitiesPage from "@/app/(dashboard)/activities/page";
import { ToastViewport } from "@/components/toast";
import { ApiError, deleteActivityApi, deleteCaptureApi, getApi, getAuthedBlob, getAuthedBlobUrl, getTeamPresence, getWorkspaceSettings } from "@/hooks/useApi";
import { mapApiActivities, mapApiProject, type ApiActivityDto } from "@/lib/live-dataset";
import { installLiveDataset } from "@/lib/tenant-data";
import type { User } from "@/lib/types";
import { resetPrototypeState } from "@/test/harness";

const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";
const PROJECT = "33333333-3333-3333-3333-333333333333";
const BLOCK = "aaaaaaaa-0000-0000-0000-000000000001";
const UNMEASURED = "aaaaaaaa-0000-0000-0000-000000000002";
const SCREEN = "aaaaaaaa-0000-0000-0000-000000000101";
const THUMB = "aaaaaaaa-0000-0000-0000-000000000102";
const WEBCAM = "aaaaaaaa-0000-0000-0000-000000000103";
const SCREEN_B = "aaaaaaaa-0000-0000-0000-000000000104";
const THUMB_B = "aaaaaaaa-0000-0000-0000-000000000105";

function member(id: string, name: string, role: User["role"]): User {
  return { id, name, email: `${id}@real.example`, role, designation: "", status: "offline", timezone: "UTC", trackedToday: 0, productivity: 0, joinedAt: "" };
}
const owner = member(OWNER, "Olivia Owner", "owner");
const worker = member(WORKER, "Wendy Worker", "worker");

function row(over: Partial<ApiActivityDto> & { creationTime?: string } = {}): ApiActivityDto {
  const end = Date.now() - 20 * 60_000;
  return {
    id: BLOCK,
    userId: WORKER,
    projectId: PROJECT,
    startedAt: new Date(end - 600_000).toISOString(),
    endedAt: new Date(end).toISOString(),
    productivity: 70,
    mouseClicks: 12,
    keyboardHits: 300,
    description: "Release notes",
    activeWindowsJson: JSON.stringify([{ AppName: "code.exe", WindowTitle: "notes.md", Seconds: 600 }]),
    captures: [
      { id: SCREEN, kind: "screen" },
      { id: THUMB, kind: "thumb" },
      { id: WEBCAM, kind: "webcam" },
    ],
    ...({ creationTime: new Date(end + 30_000).toISOString() } as object),
    ...over,
  } as ApiActivityDto;
}

/** A block whose interval took two screenshots — one per screen+thumb pair. */
function rowWithTwoScreenshots(): ApiActivityDto {
  return {
    ...row(),
    captures: [
      { id: SCREEN, kind: "screen" },
      { id: THUMB, kind: "thumb" },
      { id: SCREEN_B, kind: "screen" },
      { id: THUMB_B, kind: "thumb" },
      { id: WEBCAM, kind: "webcam" },
    ],
  } as ApiActivityDto;
}

function unmeasuredRow(): ApiActivityDto {
  return { ...row(), id: UNMEASURED, description: "Wayland session", productivity: null, mouseClicks: null, keyboardHits: null, captures: [] };
}

function json(body: unknown): Response {
  const text = JSON.stringify(body);
  return { ok: true, status: 200, statusText: "OK", json: async () => JSON.parse(text), text: async () => text } as Response;
}

function goLive(rows: ApiActivityDto[], viewer: User, policies: Record<string, boolean>) {
  localStorage.setItem("dosi-token", "live-token");
  installLiveDataset({
    workspaceId: "live-erasure",
    users: [owner, worker],
    projects: [mapApiProject({ id: PROJECT, title: "Apollo", color: "#0d9488" }, [WORKER, OWNER])],
    activities: mapApiActivities(rows),
    now: new Date(),
  });
  session.current = { user: viewer, isLive: true, status: "ready", livePolicies: policies };
  vi.stubGlobal("fetch", vi.fn<typeof fetch>().mockImplementation(async () => json({ items: rows, totalCount: rows.length })));
}

const renderPage = () =>
  render(
    <>
      <ActivitiesPage />
      <ToastViewport />
    </>,
  );

async function openDrawer(user: ReturnType<typeof userEvent.setup>, label: RegExp) {
  await user.click(await screen.findByRole("button", { name: label }));
  return screen.findByRole("dialog", { name: "Activity detail" });
}

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(getApi).mockReset();
  vi.mocked(getApi).mockImplementation(async (endpoint: string) =>
    endpoint.startsWith("/api/app/activity/screenshots/")
      ? [
          { id: SCREEN, kind: "screen", capturedAt: new Date().toISOString(), contentType: "image/jpeg" },
          { id: WEBCAM, kind: "webcam", capturedAt: new Date().toISOString(), contentType: "image/jpeg" },
        ]
      : [],
  );
  vi.mocked(getAuthedBlobUrl).mockReset();
  vi.mocked(getAuthedBlobUrl).mockImplementation(async (endpoint: string) => `blob:${endpoint}`);
  vi.mocked(getAuthedBlob).mockReset();
  vi.mocked(getAuthedBlob).mockImplementation(async (endpoint: string) => ({ url: `blob:${endpoint}`, type: "image/jpeg" }));
  vi.mocked(getTeamPresence).mockReset();
  vi.mocked(getTeamPresence).mockResolvedValue([]);
  vi.mocked(getWorkspaceSettings).mockReset();
  vi.mocked(getWorkspaceSettings).mockResolvedValue({ defaultTimeZone: null, newProjectDefaults: {} as never, memberCaptureDeleteMinutes: 60 });
  vi.mocked(deleteActivityApi).mockReset();
  vi.mocked(deleteActivityApi).mockResolvedValue({} as never);
  vi.mocked(deleteCaptureApi).mockReset();
  vi.mocked(deleteCaptureApi).mockResolvedValue({} as never);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
  window.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  session.current = null;
  localStorage.clear();
});

describe("owners and managers (Tracker.Activities.Delete)", () => {
  it("delete a block with its captures, giving the reason the record keeps", async () => {
    goLive([row()], owner, { "Tracker.Activities.Delete": true });
    const user = userEvent.setup();
    renderPage();
    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    await user.click(within(drawer).getByRole("button", { name: /delete block/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete this time block?" });
    expect(dialog).toHaveTextContent(/removed from every report, timesheet and payroll figure/i);
    await user.click(within(dialog).getByRole("button", { name: "Delete block" }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(/say why it is deleted/i);
    expect(deleteActivityApi).not.toHaveBeenCalled();

    await user.type(within(dialog).getByLabelText(/reason/i), "Personal banking page");
    await user.click(within(dialog).getByRole("button", { name: "Delete block" }));
    await waitFor(() => expect(deleteActivityApi).toHaveBeenCalledWith(BLOCK, "Personal banking page"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Activity detail" })).not.toBeInTheDocument());
    expect(screen.queryByRole("button", { name: /^Open session: .*Release notes/ })).not.toBeInTheDocument();
    expect(await screen.findByText("Time block deleted")).toBeInTheDocument();
  });

  it("delete one screenshot (the block stays), and see the server's refusal in place", async () => {
    goLive([row()], owner, { "Tracker.Activities.Delete": true });
    const user = userEvent.setup();
    renderPage();
    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    await user.click(await within(drawer).findByRole("button", { name: /^delete screenshot/i }));
    let dialog = screen.getByRole("dialog", { name: "Delete this screenshot?" });
    vi.mocked(deleteCaptureApi).mockRejectedValueOnce(
      new ApiError("API POST Error", 403, { code: "Tracker:RoleNotAllowed", message: "no", details: null, validationErrors: [] }),
    );
    await user.type(within(dialog).getByLabelText(/reason/i), "Wrong window");
    await user.click(within(dialog).getByRole("button", { name: "Delete capture" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/only a workspace owner can delete an owner's data/i);

    await user.click(within(dialog).getByRole("button", { name: "Delete capture" }));
    await waitFor(() => expect(deleteCaptureApi).toHaveBeenLastCalledWith(SCREEN, "Wrong window"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Delete this screenshot?" })).not.toBeInTheDocument());
    expect(within(drawer).queryByRole("button", { name: /^delete screenshot/i })).not.toBeInTheDocument();
    expect(within(drawer).getByText(/no screenshot captured/i)).toBeInTheDocument();
    // The webcam frame is a separate capture: it is still there, deletable on its own.
    dialog = screen.queryByRole("dialog", { name: "Delete this webcam frame?" }) as HTMLElement;
    expect(dialog).toBeNull();
    expect(within(drawer).getByRole("button", { name: /^delete webcam frame/i })).toBeInTheDocument();
  });

  it("deleting one of several screenshots leaves the list tile showing the others, not 'no screenshot'", async () => {
    goLive([rowWithTwoScreenshots()], owner, { "Tracker.Activities.Delete": true });
    vi.mocked(getApi).mockImplementation(async (endpoint: string) =>
      endpoint.startsWith("/api/app/activity/screenshots/")
        ? [
            { id: SCREEN, kind: "screen", capturedAt: "2026-01-01T10:00:00.000Z", contentType: "image/jpeg" },
            { id: SCREEN_B, kind: "screen", capturedAt: "2026-01-01T10:10:00.000Z", contentType: "image/jpeg" },
          ]
        : [],
    );
    const user = userEvent.setup();
    renderPage();
    const card = await screen.findByRole("button", { name: /^Open session: .*Release notes/ });
    expect(within(card).queryByText(/no screenshot for this session/i)).not.toBeInTheDocument();

    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    // Both screenshots loaded (oldest, SCREEN, first); delete that one.
    const deleteButtons = await within(drawer).findAllByRole("button", { name: /^delete screenshot/i });
    expect(deleteButtons).toHaveLength(2);
    await user.click(deleteButtons[0]);
    const dialog = screen.getByRole("dialog", { name: "Delete this screenshot?" });
    await user.type(within(dialog).getByLabelText(/reason/i), "Wrong window");
    await user.click(within(dialog).getByRole("button", { name: "Delete capture" }));
    await waitFor(() => expect(deleteCaptureApi).toHaveBeenLastCalledWith(SCREEN, "Wrong window"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Delete this screenshot?" })).not.toBeInTheDocument());
    // The drawer still has the other screenshot (its own, separately-fetched list).
    expect(within(drawer).getAllByRole("button", { name: /^delete screenshot/i })).toHaveLength(1);
    // So does the list tile behind it: the other screen+thumb pair wasn't erased with it.
    const cardAfter = await screen.findByRole("button", { name: /^Open session: .*Release notes/ });
    expect(within(cardAfter).queryByText(/no screenshot for this session/i)).not.toBeInTheDocument();
  });

  it("a11y: the drawer with deletion controls has no violations", async () => {
    goLive([row()], owner, { "Tracker.Activities.Delete": true });
    const user = userEvent.setup();
    renderPage();
    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    await within(drawer).findByRole("button", { name: /^delete screenshot/i });
    expect(await axe(drawer)).toHaveNoViolations();
  });
});

describe("a member's own captures", () => {
  it("can be deleted within the window, reason optional, but never the block", async () => {
    goLive([row()], worker, {});
    const user = userEvent.setup();
    renderPage();
    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    expect(within(drawer).queryByRole("button", { name: /delete block/i })).not.toBeInTheDocument();
    expect(within(drawer).getByText(/you can delete this block's captures yourself until/i)).toBeInTheDocument();
    await user.click(await within(drawer).findByRole("button", { name: /^delete webcam frame/i }));
    const dialog = screen.getByRole("dialog", { name: "Delete this webcam frame?" });
    await user.click(within(dialog).getByRole("button", { name: "Delete capture" }));
    await waitFor(() => expect(deleteCaptureApi).toHaveBeenCalledWith(WEBCAM, null));
  });

  it("are not offered once the owner turned self-deletion off", async () => {
    vi.mocked(getWorkspaceSettings).mockResolvedValue({ defaultTimeZone: null, newProjectDefaults: {} as never, memberCaptureDeleteMinutes: 0 });
    goLive([row()], worker, {});
    const user = userEvent.setup();
    renderPage();
    await waitFor(() => expect(getWorkspaceSettings).toHaveBeenCalled());
    const drawer = await openDrawer(user, /^Open session: .*Release notes/);
    await within(drawer).findByRole("img", { name: /screen capture/i });
    expect(within(drawer).queryByRole("button", { name: /^delete/i })).not.toBeInTheDocument();
  });
});

describe("unmeasured keyboard and mouse (API-05)", () => {
  it("read 'Not measured' on the card and in the report, never 0", async () => {
    goLive([unmeasuredRow()], owner, {});
    const user = userEvent.setup();
    renderPage();
    const card = await screen.findByRole("button", { name: /^Open session: .*Wayland session/ });
    expect(within(card).getAllByText("Not measured").length).toBeGreaterThan(0);
    expect(within(card).getAllByText("—").length).toBeGreaterThan(0);
    const drawer = await openDrawer(user, /^Open session: .*Wayland session/);
    expect(within(drawer).getByText("Keyboard presses").parentElement?.parentElement).toHaveTextContent("Not measured");
    expect(within(drawer).getByText("Mouse clicks").parentElement?.parentElement).toHaveTextContent("Not measured");
  });
});
