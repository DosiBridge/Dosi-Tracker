// useApi: the declarative GET must behave like getApi (401 ends the session,
// items are unwrapped), stay off the network when disabled (demo mode), and
// abort its request on unmount so a late response can't touch a dead tree.
// The imperative helpers fail with a typed ApiError carrying ABP's error code,
// page lists without losing totalCount, tell sign-in failures apart, and never
// crash when the browser blocks site storage.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { SESSION_UNAUTHORIZED_EVENT } from "@/lib/live-session";
import {
  ApiError,
  ApiUnreachableError,
  InvalidCredentialsError,
  RateLimitedError,
  SignInServerError,
  StorageUnavailableError,
  activateSubscription,
  addProjectMember,
  changeMyPassword,
  changeWorkspacePlan,
  changeWorkspaceRole,
  createWorkspaceApi,
  getInvitations,
  getMyWorkspacesApi,
  leaveWorkspaceApi,
  registerWorkspaceApi,
  getInvitationApi,
  acceptInvitationApi,
  revokeInvitation,
  workspaceHeaderFor,
  delApi,
  getAllPagesApi,
  getApi,
  getAuthedBlob,
  getAuthedBlobUrl,
  getDeviceReport,
  getHourlyReport,
  getMyPreferences,
  getPagedApi,
  getPayrollReport,
  getProjectMembers,
  getTeamPresence,
  getWorkspaceSubscriptions,
  getWorkspaceSettings,
  inviteMember,
  loginApi,
  logoutApi,
  markInvoicePaid,
  postApi,
  putApi,
  removeMemberFromWorkspace,
  removeProjectMember,
  suspendSubscription,
  updateActivityNote,
  updateMyPreferences,
  updateProject,
  updateProjectMember,
  updateWorkspaceSettings,
  useApi,
} from "./useApi";

const STATUS_TEXT: Record<number, string> = {
  200: "OK",
  204: "No Content",
  400: "Bad Request",
  401: "Unauthorized",
  403: "Forbidden",
  429: "Too Many Requests",
  500: "Internal Server Error",
  503: "Service Unavailable",
};

function res(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: STATUS_TEXT[status] ?? "",
    headers: new Headers(headers),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

function Probe({ endpoint, enabled }: { endpoint: string; enabled?: boolean }) {
  const { data, error, isLoading } = useApi<unknown[]>(endpoint, enabled === undefined ? {} : { enabled });
  return (
    <div>
      <span data-testid="loading">{String(isLoading)}</span>
      <span data-testid="data">{JSON.stringify(data)}</span>
      <span data-testid="error">{error?.message ?? ""}</span>
    </div>
  );
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  localStorage.clear();
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("useApi", () => {
  it("unwraps ABP's items envelope", async () => {
    fetchMock.mockResolvedValue(res({ items: [1, 2] }));
    render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent("[1,2]"));
    expect(screen.getByTestId("loading")).toHaveTextContent("false");
  });

  it("makes no request at all when disabled (demo mode)", () => {
    render(<Probe endpoint="/api/app/x" enabled={false} />);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.getByTestId("loading")).toHaveTextContent("false");
  });

  it("treats 401 like getApi: clears the session and reports the error", async () => {
    localStorage.setItem("dosi-token", "expired");
    fetchMock.mockResolvedValue(res({}, 401));
    render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(screen.getByTestId("error")).toHaveTextContent("Unauthorized"));
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("aborts the in-flight request on unmount and never reports the abort as an error", async () => {
    let signal: AbortSignal | undefined;
    fetchMock.mockImplementation((_url, init) => {
      signal = init?.signal ?? undefined;
      return new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      });
    });
    const { unmount } = render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    unmount();
    expect(signal?.aborted).toBe(true);
  });

  it("supersedes a slower request when the endpoint changes", async () => {
    const resolvers: ((r: Response) => void)[] = [];
    fetchMock.mockImplementation(
      (_url, init) =>
        new Promise<Response>((resolve, reject) => {
          resolvers.push(resolve);
          init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
        }),
    );
    const { rerender } = render(<Probe endpoint="/api/app/a" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    rerender(<Probe endpoint="/api/app/b" />);
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    resolvers[1](res(["b"]));
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent('["b"]'));
    expect(screen.getByTestId("error")).toHaveTextContent("");
  });
});

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006";

/** The path + query of the nth fetch call, without the API base. */
function calledPath(n = 0): string {
  return String(fetchMock.mock.calls[n]?.[0]).replace(API_BASE, "");
}

function calledInit(n = 0): RequestInit {
  return (fetchMock.mock.calls[n]?.[1] ?? {}) as RequestInit;
}

describe("failed responses are typed ApiErrors", () => {
  it("carry ABP's code, message and validation errors, and keep the message shape callers show", async () => {
    fetchMock.mockResolvedValue(
      res(
        {
          error: {
            code: "Tracker:CannotModifySelf",
            message: "You can't change your own rate.",
            validationErrors: [{ message: "HourlyRate is invalid.", members: ["hourlyRate"] }],
          },
        },
        403,
      ),
    );
    const error = await putApi("/api/app/team/member?projectId=p&userId=u", { hourlyRate: 1 }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toBeInstanceOf(Error);
    expect(error).toMatchObject({
      name: "ApiError",
      status: 403,
      code: "Tracker:CannotModifySelf",
      serverMessage: "You can't change your own rate.",
      validationErrors: [{ message: "HourlyRate is invalid.", members: ["hourlyRate"] }],
      message: "API PUT Error: Forbidden: You can't change your own rate.",
    });
  });

  it("use each verb's prefix, falling back to the status code when there is no status text", async () => {
    fetchMock.mockResolvedValue(res(undefined, 500));
    await expect(getApi("/api/app/x")).rejects.toThrow("API GET Error: Internal Server Error");
    await expect(postApi("/api/app/x", {})).rejects.toThrow("API POST Error: Internal Server Error");
    await expect(delApi("/api/app/x")).rejects.toThrow("API DELETE Error: Internal Server Error");
    fetchMock.mockResolvedValue({ ...res(undefined, 502), statusText: "" } as Response);
    await expect(getApi("/api/app/x")).rejects.toThrow("API GET Error: HTTP 502");
  });

  it("read Retry-After, and survive a response body that can't be read", async () => {
    fetchMock.mockResolvedValue(res({}, 429, { "Retry-After": "30" }));
    await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 429, retryAfterSeconds: 30, code: null });
    fetchMock.mockResolvedValue({ ok: false, status: 500, statusText: "Internal Server Error" } as Response);
    await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 500, serverMessage: null });
  });

  it("still end the session on 401", async () => {
    localStorage.setItem("dosi-token", "expired");
    fetchMock.mockResolvedValue(res({}, 401));
    await expect(postApi("/api/app/x", {})).rejects.toBeInstanceOf(ApiError);
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("keep a successful POST/PUT 204-safe and return the parsed body otherwise", async () => {
    fetchMock.mockResolvedValue(res(undefined, 204));
    expect(await putApi("/api/app/x", {})).toBeNull();
    fetchMock.mockResolvedValue(res({ id: "1" }));
    expect(await postApi("/api/app/x", {})).toEqual({ id: "1" });
  });
});

describe("paged lists", () => {
  it("getPagedApi keeps the server's totalCount", async () => {
    fetchMock.mockResolvedValue(res({ items: [{ id: "a" }], totalCount: 4321 }));
    expect(await getPagedApi("/api/app/activity?MaxResultCount=1")).toEqual({ items: [{ id: "a" }], totalCount: 4321 });
  });

  it("getAllPagesApi steps SkipCount until it has every row, and says when it stopped early", async () => {
    const rows = Array.from({ length: 2300 }, (_, i) => ({ id: `r${i}` }));
    fetchMock.mockImplementation(async (url) => {
      const qs = new URLSearchParams(String(url).split("?")[1]);
      const skip = Number(qs.get("SkipCount"));
      const max = Number(qs.get("MaxResultCount"));
      return res({ items: rows.slice(skip, skip + max), totalCount: rows.length });
    });
    const all = await getAllPagesApi<{ id: string }>("/api/app/activity?From=x");
    expect(all.items).toHaveLength(2300);
    expect(all).toMatchObject({ totalCount: 2300, truncated: false });
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(calledPath(2)).toBe("/api/app/activity?From=x&SkipCount=2000&MaxResultCount=1000");

    fetchMock.mockClear();
    const capped = await getAllPagesApi<{ id: string }>("/api/app/activity", { maxItems: 1000 });
    expect(capped).toMatchObject({ totalCount: 2300, truncated: true });
    expect(capped.items).toHaveLength(1000);
  });

  it("passes the abort signal to every page request", async () => {
    const controller = new AbortController();
    fetchMock.mockResolvedValue(res({ items: [], totalCount: 0 }));
    await getAllPagesApi("/api/app/activity", { signal: controller.signal });
    expect(calledInit(0).signal).toBe(controller.signal);
  });
});

describe("contract v2 endpoints", () => {
  beforeEach(() => {
    fetchMock.mockResolvedValue(res({}));
  });

  it("presence: unwraps the list (and reads anything else as empty)", async () => {
    fetchMock.mockResolvedValue(res({ items: [{ userId: "u1", deviceId: "d1", online: true }] }));
    expect(await getTeamPresence()).toEqual([{ userId: "u1", deviceId: "d1", online: true }]);
    expect(calledPath()).toBe("/api/app/presence/team");
    fetchMock.mockResolvedValue(res({ unexpected: true }));
    expect(await getTeamPresence()).toEqual([]);
  });

  it("captures: a download comes back with its object URL and the content type the server sent", async () => {
    const createObjectURL = vi.fn(() => "blob:capture");
    vi.stubGlobal("URL", Object.assign(Object.create(URL), { createObjectURL, revokeObjectURL: vi.fn() }));
    const binary = (type: string, header?: string) =>
      ({ ...res(undefined, 200, header ? { "Content-Type": header } : {}), blob: async () => new Blob(["x"], { type }) }) as Response;

    fetchMock.mockResolvedValue(binary("image/jpeg"));
    expect(await getAuthedBlob("/api/app/activity/screenshot/c1/content")).toEqual({ url: "blob:capture", type: "image/jpeg" });
    expect(calledPath()).toBe("/api/app/activity/screenshot/c1/content");
    // A blob without a type falls back to the response's header.
    fetchMock.mockResolvedValue(binary("", "image/png"));
    expect((await getAuthedBlob("/api/app/activity/screenshot/c2/content")).type).toBe("image/png");
    expect(await getAuthedBlobUrl("/api/app/activity/screenshot/c1/content")).toBe("blob:capture");
  });

  it("workspace subscriptions (platform): the list, or null where an older server has no such endpoint", async () => {
    fetchMock.mockResolvedValue(res({ items: [{ workspaceId: "t1", workspaceName: "Acme", seatsUsed: 8 }] }));
    expect(await getWorkspaceSubscriptions()).toEqual([{ workspaceId: "t1", workspaceName: "Acme", seatsUsed: 8 }]);
    expect(calledPath()).toBe("/api/app/platform/workspace-subscriptions");
    fetchMock.mockResolvedValue(res({ error: { message: "Not found" } }, 404));
    expect(await getWorkspaceSubscriptions()).toBeNull();
    fetchMock.mockResolvedValue(res({ error: { message: "Server error" } }, 500));
    await expect(getWorkspaceSubscriptions()).rejects.toBeInstanceOf(ApiError);
  });

  it("suspend (platform): POSTs with the workspace in the query", async () => {
    fetchMock.mockResolvedValue(res(undefined, 204));
    await suspendSubscription("t&1");
    expect(calledPath()).toBe("/api/app/platform/suspend-subscription?workspaceId=t%261");
    expect(calledInit().method).toBe("POST");
  });

  it("notes: PUT the description to the block's note route", async () => {
    fetchMock.mockResolvedValue(res({ id: "a1", description: "Fixing login" }));
    expect(await updateActivityNote("a1", "Fixing login")).toEqual({ id: "a1", description: "Fixing login" });
    expect(calledPath()).toBe("/api/app/activity/a1/note");
    expect(calledInit().method).toBe("PUT");
    expect(JSON.parse(String(calledInit().body))).toEqual({ description: "Fixing login" });
  });

  it("team: change a workspace role, remove from the workspace, change your own password", async () => {
    fetchMock.mockResolvedValue(res({ userId: "u 1", role: "manager" }));
    expect(await changeWorkspaceRole("u 1", "manager")).toEqual({ userId: "u 1", role: "manager" });
    expect(calledPath(0)).toBe("/api/app/team/workspace-role?userId=u%201&role=manager");
    expect(calledInit(0).method).toBe("PUT");

    fetchMock.mockResolvedValue(res(undefined, 204));
    await removeMemberFromWorkspace("u1");
    expect(calledPath(1)).toBe("/api/app/team/from-workspace?userId=u1");
    expect(calledInit(1).method).toBe("DELETE");

    await changeMyPassword({ currentPassword: "old", newPassword: "new" });
    expect(calledPath(2)).toBe("/api/account/my-profile/change-password");
    expect(JSON.parse(String(calledInit(2).body))).toEqual({ currentPassword: "old", newPassword: "new" });
  });

  it("projects and memberships: edit a project, list, add, invite, update and remove members", async () => {
    const project = {
      title: "Website",
      description: null,
      color: "#006bff",
      intervalMinutes: 10,
      allowScreenshot: false,
      allowWebcam: false,
      allowKeyboard: true,
      allowMouse: true,
      allowActiveWindow: true,
      allowRunningPrograms: true,
      idleThresholdMinutes: 5,
      blurScreenshots: true,
    };
    fetchMock.mockResolvedValue(res({ id: "p1", ...project }));
    expect(await updateProject("p1", project)).toMatchObject({ id: "p1", blurScreenshots: true });
    expect(calledPath(0)).toBe("/api/app/project/p1");
    expect(calledInit(0).method).toBe("PUT");
    expect(JSON.parse(String(calledInit(0).body))).toEqual(project);

    const membership = { id: "m1", projectId: "p1", userId: "u2", role: "Worker", hourlyRate: 25 };
    fetchMock.mockResolvedValue(res({ items: [membership] }));
    expect(await getProjectMembers("p1")).toEqual([membership]);
    expect(calledPath(1)).toBe("/api/app/team/project-members/p1");
    fetchMock.mockResolvedValue(res({ unexpected: true }));
    expect(await getProjectMembers("p1")).toEqual([]);

    fetchMock.mockResolvedValue(res(membership));
    await addProjectMember({ projectId: "p1", userId: "u2", role: "Worker", hourlyRate: 25 });
    expect([calledPath(3), calledInit(3).method]).toEqual(["/api/app/team/member", "POST"]);

    fetchMock.mockResolvedValue(res({ member: membership, userCreated: true, initialPassword: "one-time" }));
    expect(await inviteMember({ projectId: "p1", email: "new@acme.test", role: "Client", hourlyRate: 0 })).toMatchObject({
      userCreated: true,
      initialPassword: "one-time",
    });
    expect([calledPath(4), calledInit(4).method]).toEqual(["/api/app/team/invite-member", "POST"]);

    fetchMock.mockResolvedValue(res({ ...membership, role: "Manager" }));
    await updateProjectMember("p1", "u2", { role: "Manager", hourlyRate: 25 });
    expect([calledPath(5), calledInit(5).method]).toEqual(["/api/app/team/member?projectId=p1&userId=u2", "PUT"]);

    fetchMock.mockResolvedValue(res(undefined, 204));
    await removeProjectMember("m1");
    expect([calledPath(6), calledInit(6).method]).toEqual(["/api/app/team/member/m1", "DELETE"]);
  });

  it("memberships: a refused change carries the business rule's code", async () => {
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:CannotModifySelf", message: "You can't change your own rate." } }, 403));
    await expect(updateProjectMember("p1", "me", { role: "Worker", hourlyRate: 99 })).rejects.toMatchObject({
      status: 403,
      code: "Tracker:CannotModifySelf",
    });
  });

  it("workspace: settings and my preferences round-trip", async () => {
    const settings = {
      defaultTimeZone: "Asia/Dhaka",
      newProjectDefaults: {
        intervalMinutes: 10,
        allowScreenshot: true,
        allowWebcam: false,
        allowKeyboard: true,
        allowMouse: true,
        allowActiveWindow: true,
        allowRunningPrograms: true,
        blurScreenshots: false,
        idleThresholdMinutes: 5,
      },
    };
    fetchMock.mockResolvedValue(res(settings));
    expect(await getWorkspaceSettings()).toEqual(settings);
    expect(await updateWorkspaceSettings(settings)).toEqual(settings);
    expect(calledPath(1)).toBe("/api/app/workspace/settings");
    expect(calledInit(1).method).toBe("PUT");

    fetchMock.mockResolvedValue(res({ timeZone: "Europe/London" }));
    expect(await getMyPreferences()).toEqual({ timeZone: "Europe/London" });
    expect(await updateMyPreferences({ timeZone: "Europe/London" })).toEqual({ timeZone: "Europe/London" });
    expect(calledPath(3)).toBe("/api/app/workspace/my-preferences");
    expect(calledInit(3).method).toBe("PUT");
  });

  it("host: change a plan, activate a subscription, mark an invoice paid", async () => {
    fetchMock.mockResolvedValue(res(undefined, 204));
    await changeWorkspacePlan("t1", "p2");
    await activateSubscription("t1");
    await markInvoicePaid("i1");
    expect([0, 1, 2].map((n) => calledPath(n))).toEqual([
      "/api/app/platform/change-workspace-plan?workspaceId=t1&planId=p2",
      "/api/app/platform/activate-subscription?workspaceId=t1",
      "/api/app/platform/mark-invoice-paid?invoiceId=i1",
    ]);
    expect([0, 1, 2].map((n) => calledInit(n).method)).toEqual(["POST", "POST", "POST"]);
  });

  it("reports: payroll, hourly and devices carry the reporting zone", async () => {
    const query = { from: new Date("2026-07-01T00:00:00Z"), to: new Date("2026-07-02T00:00:00Z"), timeZone: "Asia/Dhaka" };
    fetchMock.mockResolvedValue(res({ rows: [], totalMinutes: 0, totalAmount: 0 }));
    expect(await getPayrollReport(query)).toEqual({ rows: [], totalMinutes: 0, totalAmount: 0 });
    expect(calledPath(0).startsWith("/api/app/reporting/payroll?From=")).toBe(true);
    expect(new URLSearchParams(calledPath(0).split("?")[1]).get("TimeZone")).toBe("Asia/Dhaka");

    fetchMock.mockResolvedValue(res([{ dayOfWeek: 1, hour: 9, trackedMinutes: 60, activeMinutes: 50 }]));
    expect(await getHourlyReport(query)).toEqual([{ dayOfWeek: 1, hour: 9, trackedMinutes: 60, activeMinutes: 50 }]);
    expect(calledPath(1).startsWith("/api/app/reporting/hourly?")).toBe(true);

    fetchMock.mockResolvedValue(res({ unexpected: true }));
    expect(await getDeviceReport(query)).toEqual([]);
    expect(await getHourlyReport(query)).toEqual([]);
    expect(calledPath(2).startsWith("/api/app/reporting/devices?")).toBe(true);
  });
});

describe("workspaces and X-Workspace-Id", () => {
  const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
  const header = (n = 0) => (calledInit(n).headers as Record<string, string>)["X-Workspace-Id"];

  it("sends the current workspace on every scoped call, and never on unscoped ones", async () => {
    localStorage.setItem("dosi-token", "tok");
    localStorage.setItem("dosi-workspace", WS);
    fetchMock.mockImplementation(async () => res([]));
    await getApi("/api/app/project");
    await getApi("/api/abp/application-configuration?IncludeLocalizationResources=false");
    await getAuthedBlobUrl("/api/app/activity/screenshot/c1/content").catch(() => null);
    await getMyWorkspacesApi();
    await postApi("/api/app/workspaces", { name: "x" });
    await getApi("/api/account/my-profile");
    await postApi("/api/app/invitations/tok/accept", {});
    expect([0, 1, 2].map(header)).toEqual([WS, WS, WS]);
    expect([3, 4, 5, 6].map(header)).toEqual([undefined, undefined, undefined, undefined]);
    expect(workspaceHeaderFor("/connect/token")).toBeNull();
    expect(workspaceHeaderFor("/api/app/workspace/register")).toBeNull();
    expect(workspaceHeaderFor("/api/app/workspace/settings")).toBe(WS);
  });

  it("sends no header without a chosen workspace (or with a demo id)", async () => {
    localStorage.setItem("dosi-token", "tok");
    fetchMock.mockResolvedValue(res([]));
    await getApi("/api/app/project");
    localStorage.setItem("dosi-workspace", "w1");
    await getApi("/api/app/project");
    expect([0, 1].map(header)).toEqual([undefined, undefined]);
  });

  it("a 403 NotWorkspaceMember forgets the workspace and goes to the chooser", async () => {
    localStorage.setItem("dosi-token", "tok");
    localStorage.setItem("dosi-workspace", WS);
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/dashboard", assign });
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:NotWorkspaceMember", message: "Not a member" } }, 403));
    await expect(getApi("/api/app/project")).rejects.toMatchObject({ status: 403, code: "Tracker:NotWorkspaceMember" });
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
    expect(assign).toHaveBeenCalledWith("/workspaces");
  });

  it("lists, creates and leaves workspaces", async () => {
    localStorage.setItem("dosi-token", "tok");
    fetchMock.mockResolvedValueOnce(res([{ id: WS, name: "Acme", role: "owner", planName: "Free", isOwner: true }]));
    expect(await getMyWorkspacesApi()).toEqual([{ id: WS, name: "Acme", role: "owner", planName: "Free", isOwner: true }]);
    expect(calledPath(0)).toBe("/api/app/workspaces/mine");
    fetchMock.mockResolvedValueOnce(res({ id: WS, name: "Beta" }));
    expect(await createWorkspaceApi("Beta", "p1")).toEqual({ id: WS, name: "Beta" });
    expect(calledPath(1)).toBe("/api/app/workspaces");
    expect(JSON.parse(String(calledInit(1).body))).toEqual({ name: "Beta", planId: "p1" });
    fetchMock.mockResolvedValueOnce(res(undefined, 204));
    await leaveWorkspaceApi(WS);
    expect(calledPath(2)).toBe(`/api/app/workspaces/${WS}/leave`);
    expect(calledInit(2).method).toBe("POST");
  });

  it("registers anonymously and reads/accepts invitations", async () => {
    localStorage.setItem("dosi-token", "tok");
    localStorage.setItem("dosi-workspace", WS);
    fetchMock.mockResolvedValueOnce(res({ workspaceId: WS, workspaceName: "Acme", userName: "ana" }));
    const input = { email: "ana@acme.test", password: "pw", workspaceName: "Acme", planName: "Free" };
    expect(await registerWorkspaceApi(input)).toEqual({ workspaceId: WS, workspaceName: "Acme", userName: "ana" });
    expect(calledPath(0)).toBe("/api/app/workspace/register");
    expect((calledInit(0).headers as Record<string, string>).Authorization).toBeUndefined();
    expect(header(0)).toBeUndefined();
    fetchMock.mockResolvedValueOnce(res({ error: { code: "Tracker:EmailAlreadyRegistered", message: "taken" } }, 409));
    await expect(registerWorkspaceApi(input)).rejects.toMatchObject({ status: 409, code: "Tracker:EmailAlreadyRegistered" });

    fetchMock.mockResolvedValueOnce(res({ workspaceName: "Acme", email: "b@x.test", role: "member", expired: false }));
    expect(await getInvitationApi("t 1")).toMatchObject({ workspaceName: "Acme" });
    expect(calledPath(2)).toBe("/api/app/invitations/t%201");
    fetchMock.mockResolvedValueOnce(res({ workspaceId: WS, workspaceName: "Acme", role: "member" }));
    expect(await acceptInvitationApi("t1")).toMatchObject({ workspaceId: WS });
    expect(calledPath(3)).toBe("/api/app/invitations/t1/accept");
  });

  it("lists and revokes pending invitations", async () => {
    localStorage.setItem("dosi-token", "tok");
    fetchMock.mockResolvedValueOnce(res({ items: [{ id: "i1", email: "b@x.test", role: "member", expiresAt: "2026-10-20T00:00:00Z" }] }));
    expect(await getInvitations()).toHaveLength(1);
    expect(calledPath(0)).toBe("/api/app/team/invitations");
    fetchMock.mockResolvedValueOnce(res(undefined, 204));
    await revokeInvitation("i1");
    expect(calledPath(1)).toBe("/api/app/team/i1/invitation");
    expect(calledInit(1).method).toBe("DELETE");
  });
});

describe("loginApi", () => {
  it("stores the token (no workspace in it) and sets the middleware cookie; sign-out keeps the last workspace", async () => {
    localStorage.setItem("dosi-workspace", "3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    fetchMock.mockResolvedValue(res({ access_token: "tok", expires_in: 120 }));
    expect(await loginApi("ana@acme.test", "pw")).toBe("tok");
    expect(localStorage.getItem("dosi-token")).toBe("tok");
    expect(calledPath()).toBe("/connect/token");
    expect(String(calledInit().body)).not.toContain("__tenant");
    expect(document.cookie).toContain("dosi-token=1");
    logoutApi();
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(localStorage.getItem("dosi-workspace")).toBe("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    expect(document.cookie).not.toContain("dosi-token=1");
  });

  it("reports rejected credentials as such, with the server's reason", async () => {
    fetchMock.mockResolvedValue(res({ error: "invalid_grant", error_description: "Invalid username or password!" }, 400));
    const error = await loginApi("ana@acme.test", "wrong").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(InvalidCredentialsError);
    expect(error).toMatchObject({
      message: "Invalid credentials or login failed",
      status: 400,
      error: "invalid_grant",
      errorDescription: "Invalid username or password!",
    });
  });

  it("reports a rate limit as 'try again later', never as a wrong password", async () => {
    fetchMock.mockResolvedValue(res({}, 429, { "Retry-After": "45" }));
    const error = await loginApi("ana@acme.test", "pw").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(RateLimitedError);
    expect(error).not.toBeInstanceOf(InvalidCredentialsError);
    expect(error).toMatchObject({ retryAfterSeconds: 45, message: "Too many sign-in attempts. Try again in 45 seconds." });
    fetchMock.mockResolvedValue(res({}, 429));
    await expect(loginApi("ana@acme.test", "pw")).rejects.toThrow("Too many sign-in attempts. Try again in a moment.");
  });

  it("reports a server failure as the server's, carrying its description", async () => {
    fetchMock.mockResolvedValue(res({ error: "server_error", error_description: "Database unavailable" }, 503));
    await expect(loginApi("ana@acme.test", "pw")).rejects.toMatchObject({
      name: "SignInServerError",
      status: 503,
      message: "The server couldn't sign you in: Database unavailable",
    });
    fetchMock.mockResolvedValue({ ...res(undefined, 500), text: async () => "<html>oops</html>" } as Response);
    const error = await loginApi("ana@acme.test", "pw").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(SignInServerError);
    expect((error as Error).message).toBe("The server couldn't sign you in (HTTP 500). Try again shortly.");
  });

  it("reports a missing server as unreachable", async () => {
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"));
    await expect(loginApi("ana@acme.test", "pw")).rejects.toBeInstanceOf(ApiUnreachableError);
  });
});

describe("session renewal (refresh token)", () => {
  const tokens = (access: string, refresh?: string, expiresIn = 3600) =>
    res({ access_token: access, expires_in: expiresIn, ...(refresh ? { refresh_token: refresh } : {}) });

  /** A signed-in tab: access token `old`, refresh token `r1` (unless none), expiring in `expiresInMs`. */
  function signedIn({ refresh = "r1", expiresInMs = 30 * 60_000 }: { refresh?: string; expiresInMs?: number } = {}) {
    localStorage.setItem("dosi-token", "old");
    if (refresh) localStorage.setItem("dosi-refresh-token", refresh);
    localStorage.setItem("dosi-token-expires-at", String(Date.now() + expiresInMs));
  }

  /** A server whose token endpoint issues `new` (+ refresh `r2`) and whose API accepts only `Bearer new`. */
  function renewingServer() {
    fetchMock.mockImplementation(async (url, init) => {
      if (String(url).includes("/connect/token")) return tokens("new", "r2");
      const auth = (init?.headers as Record<string, string> | undefined)?.Authorization;
      return auth === "Bearer new" ? res({ items: [1] }) : res({}, 401);
    });
  }

  const paths = () => fetchMock.mock.calls.map(([url]) => String(url).replace(API_BASE, ""));
  const tokenCalls = () => paths().filter((p) => p.startsWith("/connect/token"));

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("signs in with offline_access and keeps the refresh token; the session flag outlives the hour", async () => {
    const cookieWrites = vi.spyOn(Document.prototype, "cookie", "set");
    fetchMock.mockResolvedValue(tokens("tok", "r1", 3600));
    await loginApi("ana@acme.test", "pw");
    expect(new URLSearchParams(String(calledInit().body)).get("scope")).toBe("openid profile email roles offline_access Tracker");
    expect(localStorage.getItem("dosi-refresh-token")).toBe("r1");
    expect(Number(localStorage.getItem("dosi-token-expires-at"))).toBeGreaterThan(Date.now() + 3_500_000);
    expect(cookieWrites.mock.calls.at(-1)?.[0]).toContain(`Max-Age=${14 * 24 * 3600}`);

    // A server that issues no refresh token: the flag lasts as long as the access token, as before.
    fetchMock.mockResolvedValue(tokens("tok2", undefined, 120));
    await loginApi("ana@acme.test", "pw");
    expect(localStorage.getItem("dosi-refresh-token")).toBeNull();
    expect(cookieWrites.mock.calls.at(-1)?.[0]).toContain("Max-Age=120");
  });

  it("a 401 renews the session once and retries the request with the new token", async () => {
    signedIn();
    renewingServer();
    expect(await getApi("/api/app/x")).toEqual([1]);
    expect(paths()).toEqual(["/api/app/x", "/connect/token", "/api/app/x"]);
    expect(Object.fromEntries(new URLSearchParams(String(calledInit(1).body)))).toEqual({
      grant_type: "refresh_token",
      refresh_token: "r1",
      client_id: "Tracker_App",
    });
    expect(localStorage.getItem("dosi-token")).toBe("new");
    expect(localStorage.getItem("dosi-refresh-token")).toBe("r2");
  });

  it("renews a token about to expire before sending, so the request never meets a 401", async () => {
    signedIn({ expiresInMs: 30_000 });
    renewingServer();
    await postApi("/api/app/x", { a: 1 });
    expect(paths()).toEqual(["/connect/token", "/api/app/x"]);
    expect((calledInit(1).headers as Record<string, string>).Authorization).toBe("Bearer new");
    expect(JSON.parse(String(calledInit(1).body))).toEqual({ a: 1 });
  });

  it("renews once for any number of requests refused together", async () => {
    signedIn();
    renewingServer();
    await Promise.all([getApi("/api/app/a"), getApi("/api/app/b"), putApi("/api/app/c", {}), delApi("/api/app/d")]);
    expect(tokenCalls()).toHaveLength(1);
  });

  it("uses a token another tab already renewed instead of spending the refresh token again", async () => {
    signedIn();
    fetchMock.mockImplementation(async (url, init) => {
      if (String(url).includes("/connect/token")) throw new Error("must not renew");
      const auth = (init?.headers as Record<string, string> | undefined)?.Authorization;
      if (auth === "Bearer old") {
        localStorage.setItem("dosi-token", "from-other-tab");
        return res({}, 401);
      }
      return auth === "Bearer from-other-tab" ? res({ items: [2] }) : res({}, 401);
    });
    expect(await getApi("/api/app/x")).toEqual([2]);
    expect(tokenCalls()).toHaveLength(0);
  });

  it("a refused renewal ends the session, as an expired one always did", async () => {
    signedIn();
    fetchMock.mockImplementation(async (url) =>
      String(url).includes("/connect/token") ? res({ error: "invalid_grant" }, 400) : res({}, 401),
    );
    await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 401 });
    expect(localStorage.getItem("dosi-token")).toBeNull();
    expect(localStorage.getItem("dosi-refresh-token")).toBeNull();
    expect(localStorage.getItem("dosi-token-expires-at")).toBeNull();
  });

  it("a renewal that fails for now (rate-limited) keeps the session, and the next request renews", async () => {
    signedIn();
    let tokenEndpointBusy = true;
    fetchMock.mockImplementation(async (url, init) => {
      if (String(url).includes("/connect/token")) return tokenEndpointBusy ? res({}, 429, { "Retry-After": "5" }) : tokens("new", "r2");
      const auth = (init?.headers as Record<string, string> | undefined)?.Authorization;
      return auth === "Bearer new" ? res({ items: [1] }) : res({}, 401);
    });
    await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 401 });
    expect(localStorage.getItem("dosi-token")).toBe("old");
    expect(localStorage.getItem("dosi-refresh-token")).toBe("r1");

    tokenEndpointBusy = false;
    expect(await getApi("/api/app/x")).toEqual([1]);
    expect(localStorage.getItem("dosi-token")).toBe("new");
  });

  it("a 401 that can't be renewed asks the page first: one that asks for the password in place keeps the session", async () => {
    signedIn({ refresh: "" });
    fetchMock.mockResolvedValue(res({}, 401));
    const keepPage = vi.fn((event: Event) => event.preventDefault());
    window.addEventListener(SESSION_UNAUTHORIZED_EVENT, keepPage);
    try {
      await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 401 });
      expect(keepPage).toHaveBeenCalledTimes(1);
      expect(localStorage.getItem("dosi-token")).toBe("old"); // not signed out: the page re-authenticates in place
    } finally {
      window.removeEventListener(SESSION_UNAUTHORIZED_EVENT, keepPage);
    }
  });

  it("an older session without a refresh token ends at its first 401, with no renewal attempt", async () => {
    signedIn({ refresh: "" });
    fetchMock.mockResolvedValue(res({}, 401));
    await expect(getApi("/api/app/x")).rejects.toMatchObject({ status: 401 });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem("dosi-token")).toBeNull();
  });

  it("the declarative hook renews the same way", async () => {
    signedIn();
    renewingServer();
    render(<Probe endpoint="/api/app/x" />);
    await waitFor(() => expect(screen.getByTestId("data")).toHaveTextContent("[1]"));
    expect(screen.getByTestId("error")).toHaveTextContent("");
    expect(tokenCalls()).toHaveLength(1);
  });

  it("signing out forgets the refresh token and the expiry", () => {
    signedIn();
    logoutApi();
    expect(localStorage.getItem("dosi-refresh-token")).toBeNull();
    expect(localStorage.getItem("dosi-token-expires-at")).toBeNull();
  });
});

describe("blocked site storage", () => {
  function blockStorage() {
    const blocked = () => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    };
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(blocked);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(blocked);
    vi.spyOn(Storage.prototype, "removeItem").mockImplementation(blocked);
  }

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requests still go out (unauthenticated) instead of crashing", async () => {
    blockStorage();
    fetchMock.mockResolvedValue(res({ items: [1] }));
    expect(await getApi("/api/app/x")).toEqual([1]);
    expect((calledInit().headers as Record<string, string>).Authorization).toBeUndefined();
  });

  it("signing out still clears the cookie", () => {
    blockStorage();
    document.cookie = "dosi-token=1; Path=/";
    expect(() => logoutApi()).not.toThrow();
    expect(document.cookie).not.toContain("dosi-token=1");
  });

  it("a sign-in that can't be stored says so, and sets no cookie", async () => {
    blockStorage();
    fetchMock.mockResolvedValue(res({ access_token: "tok", expires_in: 120 }));
    await expect(loginApi("ana@acme.test", "pw")).rejects.toBeInstanceOf(StorageUnavailableError);
    expect(document.cookie).not.toContain("dosi-token=1");
  });
});
