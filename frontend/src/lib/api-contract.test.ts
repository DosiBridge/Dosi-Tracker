// The v2 API contract's pure side: query builders that stay byte-compatible
// with the older ones, paging that never silently truncates, error bodies read
// in both ABP and OAuth shapes, and the small rules (notes, presence) the
// pages share. Routes must stay inside src/test/api-contract.json.
import { describe, expect, it, vi } from "vitest";
import contract from "@/test/api-contract.json";
import { reportQuery } from "./report-math";
import {
  ACTIVITY_NOTE_EDIT_DAYS,
  API_ROUTES,
  END_REASON_LABELS,
  MAX_PAGE_SIZE,
  OWNER_ONLY_ROLES,
  PRESENCE_ONLINE_SECONDS,
  PROJECT_ROLES,
  PROJECT_TASK_LIMITS,
  PROJECT_TASK_ROUTES,
  projectTaskRoute,
  projectTasksQuery,
  projectTasksRoute,
  PRESENCE_STATUSES,
  PRESENCE_STATUS_LABELS,
  REPORTING_ROUTES,
  TEAM_ROUTES,
  TRACKER_ERROR_STATUS,
  activateSubscriptionRoute,
  activitiesQuery,
  activitiesRoute,
  activityNoteRoute,
  canEditActivityNote,
  captureExtension,
  changeWorkspacePlanRoute,
  workspaceRoleRoute,
  revokeInvitationRoute,
  TEAM_INVITATIONS_ROUTE,
  WORKSPACE_ROUTES,
  leaveWorkspaceRoute,
  archiveWorkspaceRoute,
  restoreWorkspaceRoute,
  invitationRoute,
  acceptInvitationRoute,
  declineInvitationRoute,
  collectPages,
  endReasonLabel,
  isPresenceOnline,
  isProjectColor,
  markInvoicePaidRoute,
  memberAccountActions,
  membershipEditRules,
  normalizeActivityNote,
  parseApiErrorBody,
  parseRetryAfter,
  platformLabel,
  presenceStatusLabel,
  projectMembersRoute,
  projectRoute,
  removeFromWorkspaceRoute,
  removeMemberRoute,
  reportingQuery,
  reportingRoute,
  suspendSubscriptionRoute,
  WORKSPACE_SUBSCRIPTIONS_ROUTE,
  toPagedResult,
  updateMemberRoute,
  withPaging,
  type PagedResultDto,
  type ReportingEndpoint,
} from "./api-contract";

const FROM = new Date("2026-07-01T00:00:00.000Z");
const TO = new Date("2026-07-08T00:00:00.000Z");

/** The architecture spec's rule: a route is served when it equals or extends a contract entry. */
function served(route: string): boolean {
  const path = route.split("?")[0].replace(/\/+$/, "");
  return contract.endpoints.some((e) => {
    if (path === e || path.startsWith(`${e}/`)) return true;
    if (!e.includes("{")) return false;
    const pattern = new RegExp(`^${e.split("/").map((seg) => (/^\{[^/}]+\}$/.test(seg) ? "[^/]+" : seg)).join("/")}(?:/|$)`);
    return pattern.test(path);
  });
}

describe("routes", () => {
  it("every route the web builds is declared in api-contract.json", () => {
    const routes = [
      ...Object.values(API_ROUTES),
      ...Object.values(REPORTING_ROUTES),
      ...Object.values(TEAM_ROUTES),
      activitiesRoute({ from: FROM }),
      activityNoteRoute("a1"),
      projectRoute("p1"),
      projectMembersRoute("p1"),
      updateMemberRoute("p1", "u1"),
      removeMemberRoute("m1"),
      removeFromWorkspaceRoute("u1"),
      workspaceRoleRoute("u1", "manager"),
      TEAM_INVITATIONS_ROUTE,
      revokeInvitationRoute("i1"),
      ...Object.values(WORKSPACE_ROUTES),
      leaveWorkspaceRoute("w1"),
      archiveWorkspaceRoute("w1"),
      restoreWorkspaceRoute("w1"),
      invitationRoute("tok"),
      acceptInvitationRoute("tok"),
      changeWorkspacePlanRoute("t1", "p1"),
      activateSubscriptionRoute("t1"),
      suspendSubscriptionRoute("t1"),
      WORKSPACE_SUBSCRIPTIONS_ROUTE,
      markInvoicePaidRoute("i1"),
      PROJECT_TASK_ROUTES.collection,
      projectTaskRoute("task-1"),
      projectTasksRoute({ projectId: "p1" }),
    ];
    expect(routes.filter((r) => !served(r))).toEqual([]);
  });

  it("builds the platform's subscription routes with the workspace in the query", () => {
    expect(suspendSubscriptionRoute("t&1")).toBe("/api/app/platform/suspend-subscription?workspaceId=t%261");
    expect(WORKSPACE_SUBSCRIPTIONS_ROUTE).toBe("/api/app/platform/workspace-subscriptions");
  });

  it("puts ids where the backend binds them, encoded", () => {
    expect(projectRoute("p 1")).toBe("/api/app/project/p%201");
    expect(projectMembersRoute("p/1")).toBe("/api/app/team/project-members/p%2F1");
    expect(updateMemberRoute("p1", "u&1")).toBe("/api/app/team/member?projectId=p1&userId=u%261");
    expect(removeMemberRoute("m1")).toBe("/api/app/team/member/m1");
    expect(activityNoteRoute("a/1")).toBe("/api/app/activity/a%2F1/note");
    expect(workspaceRoleRoute("u 1", "owner")).toBe("/api/app/team/workspace-role?userId=u%201&role=owner");
    expect(revokeInvitationRoute("i1")).toBe("/api/app/team/i1/invitation");
    expect(leaveWorkspaceRoute("w1")).toBe("/api/app/workspaces/w1/leave");
    expect(archiveWorkspaceRoute("w 1")).toBe("/api/app/workspaces/w%201/archive");
    expect(restoreWorkspaceRoute("w 1")).toBe("/api/app/workspaces/w%201/restore");
    expect(acceptInvitationRoute("a b")).toBe("/api/app/invitations/a%20b/accept");
    expect(declineInvitationRoute("a b")).toBe("/api/app/invitations/a%20b/decline");
    expect(removeFromWorkspaceRoute("u1")).toBe("/api/app/team/from-workspace?userId=u1");
    expect(changeWorkspacePlanRoute("t&1", "p=2")).toBe("/api/app/platform/change-workspace-plan?workspaceId=t%261&planId=p%3D2");
    expect(activateSubscriptionRoute("t1")).toBe("/api/app/platform/activate-subscription?workspaceId=t1");
    expect(markInvoicePaidRoute("i1")).toBe("/api/app/platform/mark-invoice-paid?invoiceId=i1");
  });

  it("builds every reporting endpoint with its query", () => {
    const endpoints = Object.keys(REPORTING_ROUTES) as ReportingEndpoint[];
    expect(endpoints).toHaveLength(8);
    for (const endpoint of endpoints) {
      const route = reportingRoute(endpoint, { from: FROM, to: TO, timeZone: "Asia/Dhaka" });
      expect(route).toBe(`/api/app/reporting/${endpoint}?${reportingQuery({ from: FROM, to: TO, timeZone: "Asia/Dhaka" })}`);
    }
  });
});

describe("reportingQuery", () => {
  it("is byte-identical to the existing reportQuery when no zone or day basis is given", () => {
    expect(reportingQuery({ from: FROM, to: TO })).toBe(reportQuery({ from: FROM, to: TO }));
    expect(reportingQuery({ from: FROM, to: TO, projectId: "p1", userId: "u1" })).toBe(
      reportQuery({ from: FROM, to: TO, projectId: "p1", userId: "u1" }),
    );
  });

  it("adds the reporting zone and day basis (an older server ignores them)", () => {
    const qs = new URLSearchParams(reportingQuery({ from: FROM, to: TO, timeZone: "America/New_York", dayBasis: "member" }));
    expect(qs.get("TimeZone")).toBe("America/New_York");
    expect(qs.get("DayBasis")).toBe("member");
    expect(qs.get("From")).toBe(FROM.toISOString());
  });

  it("never sends the 'all' or empty filters", () => {
    const qs = new URLSearchParams(reportingQuery({ from: FROM, to: TO, projectId: "all", userId: "", timeZone: null, dayBasis: null }));
    expect([...qs.keys()]).toEqual(["From", "To"]);
  });
});

describe("activitiesQuery / activitiesRoute", () => {
  it("sends only the filters given", () => {
    expect(activitiesRoute()).toBe("/api/app/activity");
    const qs = new URLSearchParams(activitiesQuery({ projectId: "p1", userId: "all", from: FROM, to: TO, skipCount: 2000.7, maxResultCount: 0 }));
    expect(Object.fromEntries(qs)).toEqual({
      ProjectId: "p1",
      From: FROM.toISOString(),
      To: TO.toISOString(),
      SkipCount: "2000",
      MaxResultCount: "1",
    });
  });
});

describe("paging", () => {
  it("replaces paging parameters in any letter case and keeps the rest byte for byte", () => {
    expect(withPaging("/api/app/activity?From=2026-07-01T00%3A00%3A00.000Z&maxresultcount=1000&ProjectId=p1", 1000, 500)).toBe(
      "/api/app/activity?From=2026-07-01T00%3A00%3A00.000Z&ProjectId=p1&SkipCount=1000&MaxResultCount=500",
    );
    expect(withPaging("/api/app/activity", 0, 1000)).toBe("/api/app/activity?SkipCount=0&MaxResultCount=1000");
    expect(withPaging("/api/app/activity?SkipCount=5&", -3, 0)).toBe("/api/app/activity?SkipCount=0&MaxResultCount=1");
  });

  it("reads ABP's paged envelope or a bare array", () => {
    expect(toPagedResult({ items: [1, 2], totalCount: 40 })).toEqual({ items: [1, 2], totalCount: 40 });
    expect(toPagedResult([1, 2, 3])).toEqual({ items: [1, 2, 3], totalCount: 3 });
    // A total smaller than what came back (or missing) can't be right: the items are the floor.
    expect(toPagedResult({ items: [1, 2], totalCount: 1 })).toEqual({ items: [1, 2], totalCount: 2 });
    expect(toPagedResult({ items: [1] })).toEqual({ items: [1], totalCount: 1 });
    expect(toPagedResult(null)).toEqual({ items: [], totalCount: 0 });
    expect(toPagedResult("nope")).toEqual({ items: [], totalCount: 0 });
  });

  /** A server holding `total` rows (ids r0…), optionally gaining rows while it is read. */
  function server(total: number, grow: (call: number) => number = () => 0) {
    let rows = Array.from({ length: total }, (_, i) => ({ id: `r${i}` }));
    let call = 0;
    const getPage = vi.fn(async (endpoint: string): Promise<PagedResultDto<{ id: string }>> => {
      const added = grow(call++);
      // New rows arrive newest-first, shifting every offset (the list is ordered newest first).
      rows = [...Array.from({ length: added }, (_, i) => ({ id: `new${call}-${i}` })), ...rows];
      const qs = new URLSearchParams(endpoint.split("?")[1]);
      const skip = Number(qs.get("SkipCount"));
      const max = Number(qs.get("MaxResultCount"));
      return { items: rows.slice(skip, skip + max), totalCount: rows.length };
    });
    return getPage;
  }

  it("reads every page until the server's total is reached", async () => {
    const getPage = server(2500);
    const result = await collectPages(getPage, "/api/app/activity?From=x");
    expect(result.items).toHaveLength(2500);
    expect(result).toMatchObject({ totalCount: 2500, truncated: false });
    expect(getPage).toHaveBeenCalledTimes(3);
    expect(getPage.mock.calls.map(([url]) => new URLSearchParams(url.split("?")[1]).get("SkipCount"))).toEqual(["0", "1000", "2000"]);
  });

  it("stops at maxItems and says the list is partial", async () => {
    const result = await collectPages(server(2500), "/api/app/activity", { maxItems: 1200 });
    expect(result.items).toHaveLength(1200);
    expect(result).toMatchObject({ totalCount: 2500, truncated: true });
  });

  it("drops a row a growing list returns twice", async () => {
    // One upload lands before the second page: every offset shifts by one.
    const result = await collectPages(server(1500, (call) => (call === 1 ? 1 : 0)), "/api/app/activity");
    const ids = result.items.map((r) => r.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toContain("r999");
    expect(ids.filter((id) => id === "r999")).toHaveLength(1);
  });

  it("clamps the page size to ABP's limit and honours a custom identity", async () => {
    const getPage = server(30);
    const result = await collectPages(getPage, "/api/app/x", { pageSize: 5000, idOf: () => undefined });
    expect(new URLSearchParams(getPage.mock.calls[0][0].split("?")[1]).get("MaxResultCount")).toBe(String(MAX_PAGE_SIZE));
    expect(result.items).toHaveLength(30);
    const small = server(7);
    await collectPages(small, "/api/app/x", { pageSize: 3 });
    expect(small).toHaveBeenCalledTimes(3);
  });

  it("stops on a short or empty page even when the reported total is higher", async () => {
    const getPage = vi.fn(async () => ({ items: [{ id: "a" }], totalCount: 10 }));
    const result = await collectPages(getPage, "/api/app/x", { pageSize: 5 });
    expect(getPage).toHaveBeenCalledTimes(1);
    expect(result).toEqual({ items: [{ id: "a" }], totalCount: 10, truncated: true });
    const empty = vi.fn(async () => ({ items: [], totalCount: 0 }));
    expect(await collectPages(empty, "/api/app/x")).toEqual({ items: [], totalCount: 0, truncated: false });
  });
});

describe("parseApiErrorBody", () => {
  it("reads an ABP business error", () => {
    const body = JSON.stringify({ error: { code: "Tracker:CannotModifySelf", message: "You can't change your own rate.", details: "rate", data: {} } });
    expect(parseApiErrorBody(body)).toEqual({
      code: "Tracker:CannotModifySelf",
      message: "You can't change your own rate.",
      details: "rate",
      validationErrors: [],
    });
  });

  it("reads ABP validation errors, dropping malformed entries", () => {
    const body = JSON.stringify({
      error: {
        message: "Your request is not valid!",
        validationErrors: [{ message: "Description is too long.", members: ["description", 3] }, { members: ["x"] }, null, { message: "No members" }],
      },
    });
    expect(parseApiErrorBody(body)).toEqual({
      code: null,
      message: "Your request is not valid!",
      details: null,
      validationErrors: [
        { message: "Description is too long.", members: ["description"] },
        { message: "No members", members: [] },
      ],
    });
  });

  it("reads an OAuth token error", () => {
    expect(parseApiErrorBody(JSON.stringify({ error: "invalid_grant", error_description: "Invalid username or password!" }))).toEqual({
      code: "invalid_grant",
      message: "Invalid username or password!",
      details: null,
      validationErrors: [],
    });
  });

  it("keeps error.data only when it is a non-empty object", () => {
    const body = (data: unknown) => JSON.stringify({ error: { code: "Tracker:AiQuotaExceeded", message: "Used up.", data } });
    expect(parseApiErrorBody(body({ scope: "workspace-month", resetsAt: "2026-08-01T00:00:00Z" }))).toEqual({
      code: "Tracker:AiQuotaExceeded",
      message: "Used up.",
      details: null,
      validationErrors: [],
      data: { scope: "workspace-month", resetsAt: "2026-08-01T00:00:00Z" },
    });
    for (const empty of [{}, [], ["x"], null, "reason", 3, true]) {
      const info = parseApiErrorBody(body(empty));
      expect(info).not.toHaveProperty("data");
      expect(info.code).toBe("Tracker:AiQuotaExceeded");
    }
    expect(parseApiErrorBody(JSON.stringify({ error: { code: "Tracker:AiDisabled" } }))).not.toHaveProperty("data");
  });

  it("yields empty fields for anything else", () => {
    const empty = { code: null, message: null, details: null, validationErrors: [] };
    for (const body of ["", null, undefined, "<html>Bad gateway</html>", "42", "null", JSON.stringify({ error: { code: "  " } })]) {
      expect(parseApiErrorBody(body)).toEqual(empty);
    }
  });
});

describe("parseRetryAfter", () => {
  const now = Date.parse("2026-07-14T10:00:00Z");

  it("reads delay-seconds and HTTP dates, never negative", () => {
    expect(parseRetryAfter("120", now)).toBe(120);
    expect(parseRetryAfter(" 0 ", now)).toBe(0);
    expect(parseRetryAfter("Tue, 14 Jul 2026 10:01:30 GMT", now)).toBe(90);
    expect(parseRetryAfter("Tue, 14 Jul 2026 09:00:00 GMT", now)).toBe(0);
  });

  it("is null when absent or unreadable", () => {
    expect(parseRetryAfter(null, now)).toBeNull();
    expect(parseRetryAfter("", now)).toBeNull();
    expect(parseRetryAfter("soon", now)).toBeNull();
  });
});

describe("labels and rules", () => {
  it("names a capture file for its content type (JPEG preferred, PNG accepted)", () => {
    expect(captureExtension("image/jpeg")).toBe("jpg");
    expect(captureExtension("IMAGE/PNG; charset=binary")).toBe("png");
    expect(captureExtension("image/png")).toBe("png");
    expect(captureExtension("image/webp")).toBe("webp");
    expect(captureExtension("")).toBe("jpg"); // unknown: what agents send by default
    expect(captureExtension(null)).toBe("jpg");
  });

  it("names platforms, end reasons and presence statuses, and nothing unknown", () => {
    expect(platformLabel("windows")).toBe("Windows");
    expect(platformLabel("macos")).toBe("macOS");
    expect(platformLabel("linux")).toBe("Linux");
    expect(platformLabel("freebsd")).toBeNull();
    expect(platformLabel(null)).toBeNull();
    expect(endReasonLabel("lock")).toBe("Screen locked");
    expect(endReasonLabel("toString")).toBeNull();
    expect(endReasonLabel(undefined)).toBeNull();
    expect(Object.keys(END_REASON_LABELS)).toHaveLength(11);
    expect(PRESENCE_STATUSES.map(presenceStatusLabel)).toEqual(PRESENCE_STATUSES.map((s) => PRESENCE_STATUS_LABELS[s]));
    expect(presenceStatusLabel("hacking")).toBe("Unknown");
    expect(presenceStatusLabel(null)).toBe("Unknown");
  });

  it("ages presence with the server's 150-second rule, reading zone-less times as UTC", () => {
    const now = new Date("2026-07-14T10:00:00Z");
    expect(PRESENCE_ONLINE_SECONDS).toBe(150);
    expect(isPresenceOnline("2026-07-14T09:57:30Z", now)).toBe(true);
    expect(isPresenceOnline("2026-07-14T09:57:29.999Z", now)).toBe(false);
    expect(isPresenceOnline("2026-07-14T09:59:00", now)).toBe(true);
    expect(isPresenceOnline("2026-07-14T09:59:00", new Date("2026-07-14T16:00:00+06:00"))).toBe(true);
    expect(isPresenceOnline("garbage", now)).toBe(false);
    expect(isPresenceOnline(null, now)).toBe(false);
  });

  it("normalizes a note the way the server stores it", () => {
    expect(normalizeActivityNote("  Fixing\tthe\n\nlogin   bug  ")).toBe("Fixing the login bug");
    expect(normalizeActivityNote(`Deploy${String.fromCharCode(7)}ed${String.fromCharCode(0x85)}!`)).toBe("Deployed!");
    expect(normalizeActivityNote(" \n\t ")).toBeNull();
    expect(normalizeActivityNote("")).toBeNull();
    expect(normalizeActivityNote(null)).toBeNull();
    expect(normalizeActivityNote("Ünïcode ✓ রিপোর্ট")).toBe("Ünïcode ✓ রিপোর্ট");
  });

  it("lets the block's owner edit its note for 30 days, and team managers always", () => {
    const now = new Date("2026-07-31T12:00:00Z");
    const startedAt = new Date(now.getTime() - ACTIVITY_NOTE_EDIT_DAYS * 86_400_000).toISOString();
    expect(canEditActivityNote({ isOwner: true, canManageTeam: false, startedAt, now })).toBe(true);
    expect(canEditActivityNote({ isOwner: true, canManageTeam: false, startedAt: "2026-07-01T11:59:59Z", now })).toBe(false);
    expect(canEditActivityNote({ isOwner: false, canManageTeam: false, startedAt, now })).toBe(false);
    expect(canEditActivityNote({ isOwner: false, canManageTeam: true, startedAt: "2020-01-01T00:00:00Z", now })).toBe(true);
    expect(canEditActivityNote({ isOwner: true, canManageTeam: false, startedAt: "bad", now })).toBe(false);
  });

  it("lets only owners grant Admin/Manager or change rates, and nobody edit their own membership", () => {
    const none = { roles: [], canChangeRole: false, canChangeRate: false };
    expect(membershipEditRules({ canManageTeam: true, callerIsOwner: true, isSelf: false })).toEqual({
      roles: PROJECT_ROLES,
      canChangeRole: true,
      canChangeRate: true,
    });
    expect(membershipEditRules({ canManageTeam: true, callerIsOwner: false, isSelf: false })).toEqual({
      roles: ["Worker", "Client"],
      canChangeRole: true,
      canChangeRate: false,
    });
    expect(membershipEditRules({ canManageTeam: true, callerIsOwner: true, isSelf: true })).toEqual(none);
    expect(membershipEditRules({ canManageTeam: false, callerIsOwner: false, isSelf: false })).toEqual(none);
    expect(OWNER_ONLY_ROLES).toEqual(["Admin", "Manager"]);
  });

  it("offers a role change (owners) and removal only where the server allows them", () => {
    const base = { canManageTeam: true, callerIsOwner: false, targetIsOwner: false, isSelf: false };
    expect(memberAccountActions(base)).toEqual({ canChangeWorkspaceRole: false, canRemoveFromWorkspace: true });
    expect(memberAccountActions({ ...base, targetIsOwner: true })).toEqual({ canChangeWorkspaceRole: false, canRemoveFromWorkspace: false });
    expect(memberAccountActions({ ...base, targetIsOwner: true, callerIsOwner: true })).toEqual({
      canChangeWorkspaceRole: true,
      canRemoveFromWorkspace: false,
    });
    expect(memberAccountActions({ ...base, isSelf: true, callerIsOwner: true })).toEqual({ canChangeWorkspaceRole: false, canRemoveFromWorkspace: false });
    expect(memberAccountActions({ ...base, canManageTeam: false })).toEqual({ canChangeWorkspaceRole: false, canRemoveFromWorkspace: false });
  });

  it("accepts only #rrggbb project colours", () => {
    expect(isProjectColor("#006BFF")).toBe(true);
    for (const value of ["#06f", "006bff", "#006bffaa", "red", null]) expect(isProjectColor(value)).toBe(false);
  });

  it("maps business errors to the statuses agents and pages branch on", () => {
    expect(TRACKER_ERROR_STATUS["Tracker:CannotModifySelf"]).toBe(403);
    expect(TRACKER_ERROR_STATUS["Tracker:RoleNotAllowed"]).toBe(403);
    expect(TRACKER_ERROR_STATUS["Tracker:InvalidUser"]).toBe(404);
    expect(TRACKER_ERROR_STATUS["Tracker:ActivityInFuture"]).toBe(425);
    expect(Object.keys(TRACKER_ERROR_STATUS)).toHaveLength(29);
    expect(TRACKER_ERROR_STATUS["Tracker:NotWorkspaceMember"]).toBe(403);
  });
});

describe("projectTasksQuery", () => {
  it("builds query with ProjectId, optional IsCompleted, SkipCount, and caps MaxResultCount at 100", () => {
    expect(projectTasksQuery({ projectId: "p-123" })).toBe("ProjectId=p-123");
    expect(projectTasksQuery({ projectId: "p-123", isCompleted: true })).toBe("ProjectId=p-123&IsCompleted=true");
    expect(projectTasksQuery({ projectId: "p-123", isCompleted: false, skipCount: 20, maxResultCount: 20 })).toBe(
      "ProjectId=p-123&IsCompleted=false&SkipCount=20&MaxResultCount=20"
    );
    // Capping maxResultCount at 100 per ProjectTaskConsts
    expect(projectTasksQuery({ projectId: "p-123", maxResultCount: 500 })).toBe("ProjectId=p-123&MaxResultCount=100");
    expect(projectTasksRoute({ projectId: "p-123" })).toBe("/api/app/project-task?ProjectId=p-123");
    expect(projectTaskRoute("task-uuid-1")).toBe("/api/app/project-task/task-uuid-1");
    expect(PROJECT_TASK_LIMITS.titleMaxLength).toBe(256);
    expect(PROJECT_TASK_LIMITS.descriptionMaxLength).toBe(2048);
  });
});

