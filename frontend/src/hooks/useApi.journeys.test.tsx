// The journeys' API wrappers (backend B1-B3): invitations the signed-in person
// answers, recovery (anonymous), devices, deletion, retention and billing.
// Each sends exactly the contract's route and body, keeps unscoped routes free
// of X-Workspace-Id, and reads an older server's missing endpoint as "not
// available" rather than an error.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ApiError,
  RECOVERY_APP_NAME,
  acceptMyInvitationApi,
  cancelMySubscriptionApi,
  changeMyPlanApi,
  completeRecoveryApi,
  confirmEmailApi,
  declineInvitationApi,
  declineMyInvitationApi,
  deleteActivityApi,
  deleteCaptureApi,
  deleteMyAccountApi,
  deleteWorkspaceApi,
  getAccountDeletionCheckApi,
  getActivityRetentionApi,
  getApi,
  getBillingModeApi,
  getCaptureRetentionApi,
  getMyInvitationsApi,
  getWorkspaceDevicesApi,
  resendInvitationApi,
  restoreMemberDeviceApi,
  revokeMemberDeviceApi,
  revokeMyDeviceApi,
  sendEmailConfirmationApi,
  sendMemberRecoveryEmailApi,
  sendRecoveryEmailApi,
  updateActivityRetentionApi,
  updateCaptureRetentionApi,
  verifyRecoveryLinkApi,
} from "./useApi";

const API_BASE = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

function res(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 404 ? "Not Found" : status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
const path = (n = 0) => String(fetchMock.mock.calls[n]?.[0]).replace(API_BASE, "");
const init = (n = 0) => (fetchMock.mock.calls[n]?.[1] ?? {}) as RequestInit;
const body = (n = 0) => JSON.parse(String(init(n).body));
const headers = (n = 0) => init(n).headers as Record<string, string>;

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("dosi-token", "tok");
  localStorage.setItem("dosi-workspace", WS);
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("invitations the signed-in person answers (no auto-join, E2E-01)", () => {
  it("lists them whole, keeping why the list is empty, without a workspace header", async () => {
    fetchMock.mockResolvedValueOnce(
      res({ emailConfirmed: true, items: [{ id: "i1", workspaceId: WS, workspaceName: "Acme", role: "member", expiresAt: "2026-11-01T00:00:00Z" }, null] }),
    );
    expect(await getMyInvitationsApi()).toEqual({
      emailConfirmed: true,
      items: [{ id: "i1", workspaceId: WS, workspaceName: "Acme", role: "member", expiresAt: "2026-11-01T00:00:00Z" }],
    });
    expect(path()).toBe("/api/app/invitations/mine");
    expect(headers()["X-Workspace-Id"]).toBeUndefined();

    fetchMock.mockResolvedValueOnce(res({ emailConfirmed: false, items: [] }));
    expect(await getMyInvitationsApi()).toEqual({ emailConfirmed: false, items: [] });
  });

  it("reads an older server's missing endpoint as not available, and rethrows anything else", async () => {
    fetchMock.mockResolvedValueOnce(res({}, 404));
    expect(await getMyInvitationsApi()).toBeNull();
    fetchMock.mockResolvedValueOnce(res({ error: { message: "boom" } }, 500));
    await expect(getMyInvitationsApi()).rejects.toBeInstanceOf(ApiError);
  });

  it("accepts and declines by id, and declines a linked invitation by token", async () => {
    fetchMock.mockResolvedValueOnce(res({ workspaceId: WS, workspaceName: "Acme", role: "member" }));
    expect(await acceptMyInvitationApi("i 1")).toMatchObject({ workspaceId: WS });
    expect(path(0)).toBe("/api/app/invitations/mine/i%201/accept");
    fetchMock.mockResolvedValueOnce(res(undefined, 204));
    await declineMyInvitationApi("i1");
    expect(path(1)).toBe("/api/app/invitations/mine/i1/decline");
    fetchMock.mockResolvedValueOnce(res(undefined, 204));
    await declineInvitationApi("tok");
    expect(path(2)).toBe("/api/app/invitations/tok/decline");
    expect([0, 1, 2].map((n) => init(n).method)).toEqual(["POST", "POST", "POST"]);
    expect([0, 1, 2].map((n) => headers(n)["X-Workspace-Id"])).toEqual([undefined, undefined, undefined]);
  });

  it("resends an invitation and emails a member a recovery link (team, workspace-scoped)", async () => {
    fetchMock.mockResolvedValueOnce(res({ added: false, workspaceRole: "member", invitePath: "/invite/new", emailSent: true }));
    expect(await resendInvitationApi("i1")).toMatchObject({ invitePath: "/invite/new", emailSent: true });
    expect(path(0)).toBe("/api/app/team/i1/resend-invitation");
    fetchMock.mockResolvedValueOnce(res({ emailSent: false }));
    expect(await sendMemberRecoveryEmailApi("u&1")).toEqual({ emailSent: false });
    expect(path(1)).toBe("/api/app/team/send-password-reset?userId=u%261");
    expect(headers(1)["X-Workspace-Id"]).toBe(WS);
  });
});

describe("recovery and confirmation (anonymous)", () => {
  it("sends no token and no workspace, and the reset-code body ABP requires", async () => {
    fetchMock.mockResolvedValue(res(undefined, 204));
    await sendRecoveryEmailApi("ana@acme.example", "/team");
    await sendRecoveryEmailApi("ana@acme.example");
    expect(path(0)).toBe("/api/account/send-password-reset-code");
    expect(body(0)).toEqual({ email: "ana@acme.example", appName: RECOVERY_APP_NAME, returnUrl: "/team" });
    expect(body(1)).toEqual({ email: "ana@acme.example", appName: RECOVERY_APP_NAME });
    expect(headers(0).Authorization).toBeUndefined();
    expect(headers(0)["X-Workspace-Id"]).toBeUndefined();
    expect(init(0).credentials).toBe("omit");
  });

  it("verifies a link, completes the reset and confirms an email", async () => {
    fetchMock.mockResolvedValueOnce(res(true)).mockResolvedValueOnce(res(false)).mockResolvedValue(res(undefined, 204));
    expect(await verifyRecoveryLinkApi("u1", "t1")).toBe(true);
    expect(await verifyRecoveryLinkApi("u1", "t2")).toBe(false);
    expect(path(0)).toBe("/api/account/verify-password-reset-token");
    expect(body(0)).toEqual({ userId: "u1", resetToken: "t1" });
    await completeRecoveryApi({ userId: "u1", resetToken: "t1", password: "new-one-1" });
    expect(path(2)).toBe("/api/account/reset-password");
    await confirmEmailApi({ userId: "u1", token: "c1" });
    expect(path(3)).toBe("/api/app/my-account/confirm-email");
    expect(body(3)).toEqual({ userId: "u1", token: "c1" });
  });

  it("asks for a confirmation email, checks and deletes the account (signed in, unscoped)", async () => {
    fetchMock
      .mockResolvedValueOnce(res({ alreadyConfirmed: false, emailSent: true }))
      .mockResolvedValueOnce(res({ canDelete: false, isPlatformAdministrator: false, soleOwnerOf: [{ id: WS, name: "Acme" }] }))
      .mockResolvedValueOnce(res(undefined, 204));
    expect(await sendEmailConfirmationApi()).toEqual({ alreadyConfirmed: false, emailSent: true });
    expect(await getAccountDeletionCheckApi()).toMatchObject({ canDelete: false });
    await deleteMyAccountApi("current-one");
    expect([0, 1, 2].map((n) => path(n))).toEqual([
      "/api/app/my-account/send-email-confirmation",
      "/api/app/my-account/deletion-check",
      "/api/app/my-account/delete",
    ]);
    expect(body(2)).toEqual({ password: "current-one" });
    expect([0, 1, 2].map((n) => headers(n)["X-Workspace-Id"])).toEqual([undefined, undefined, undefined]);
    expect(headers(2).Authorization).toBe("Bearer tok");
  });
});

describe("devices, deletion and retention (workspace-scoped)", () => {
  it("lists devices (null on an older server) and revokes or restores them", async () => {
    fetchMock.mockResolvedValueOnce(res({ items: [{ userId: "u1", deviceId: "d1", isRevoked: false }] }));
    expect(await getWorkspaceDevicesApi()).toEqual([{ userId: "u1", deviceId: "d1", isRevoked: false }]);
    expect(headers(0)["X-Workspace-Id"]).toBe(WS);
    fetchMock.mockResolvedValueOnce(res({}, 405));
    expect(await getWorkspaceDevicesApi()).toBeNull();
    fetchMock.mockResolvedValueOnce(res({ error: { message: "x" } }, 500));
    await expect(getWorkspaceDevicesApi()).rejects.toBeInstanceOf(ApiError);

    fetchMock.mockResolvedValue(res({ id: "r1" }));
    await revokeMyDeviceApi("d1", null);
    await revokeMemberDeviceApi("u2", "d2", "Lost laptop");
    await restoreMemberDeviceApi("u2", "d2");
    expect([3, 4, 5].map((n) => path(n))).toEqual(["/api/app/device/revoke", "/api/app/device/revoke-member", "/api/app/device/restore"]);
    expect([3, 4, 5].map((n) => body(n))).toEqual([
      { deviceId: "d1", reason: null },
      { userId: "u2", deviceId: "d2", reason: "Lost laptop" },
      { userId: "u2", deviceId: "d2" },
    ]);
  });

  it("deletes a block, a capture and a workspace with their confirmations", async () => {
    fetchMock.mockResolvedValue(res({ id: "e1" }));
    await deleteActivityApi("a1", "Personal banking page");
    await deleteCaptureApi("c1", null);
    await deleteWorkspaceApi(WS, "Acme");
    expect([0, 1, 2].map((n) => path(n))).toEqual(["/api/app/activity/a1/delete", "/api/app/activity/capture/c1/delete", `/api/app/workspaces/${WS}/delete`]);
    expect([0, 1, 2].map((n) => body(n))).toEqual([{ reason: "Personal banking page" }, { reason: null }, { confirmationName: "Acme" }]);
  });

  it("reads and saves both retention policies, sending only what the owner sets", async () => {
    fetchMock.mockResolvedValueOnce(res({ isEnabled: false, retentionDays: 30, isEnforced: true }));
    expect(await getCaptureRetentionApi(WS)).toEqual({ isEnabled: false, retentionDays: 30, isEnforced: true });
    fetchMock.mockResolvedValueOnce(res({}, 404));
    expect(await getActivityRetentionApi(WS)).toBeNull();
    fetchMock.mockResolvedValue(res({ isEnabled: true, retentionDays: 90 }));
    await updateCaptureRetentionApi(WS, { isEnabled: true, retentionDays: 14, isEnforced: false });
    await updateActivityRetentionApi(WS, { isEnabled: true, retentionDays: 400 });
    expect(path(2)).toBe(`/api/app/workspaces/${WS}/capture-retention`);
    expect(init(2).method).toBe("PUT");
    expect(body(2)).toEqual({ isEnabled: true, retentionDays: 14 });
    expect(path(3)).toBe(`/api/app/workspaces/${WS}/activity-retention`);
    expect(body(3)).toEqual({ isEnabled: true, retentionDays: 400 });
    fetchMock.mockResolvedValueOnce(res({ error: { message: "x" } }, 500));
    await expect(getCaptureRetentionApi(WS)).rejects.toBeInstanceOf(ApiError);
    fetchMock.mockResolvedValueOnce(res({ error: { message: "x" } }, 500));
    await expect(getActivityRetentionApi(WS)).rejects.toBeInstanceOf(ApiError);
  });
});

describe("billing", () => {
  it("reads the deployment's billing mode anonymously (null when the server can't say)", async () => {
    fetchMock.mockResolvedValueOnce(res({ enabled: false }));
    expect(await getBillingModeApi()).toEqual({ enabled: false });
    expect(path(0)).toBe("/api/app/workspace/billing-mode");
    expect(headers(0).Authorization).toBeUndefined();
    fetchMock.mockResolvedValueOnce(res({}, 404));
    expect(await getBillingModeApi()).toBeNull();
    fetchMock.mockResolvedValueOnce(res({ enabled: "yes" }));
    expect(await getBillingModeApi()).toBeNull();
    fetchMock.mockResolvedValueOnce(res({ error: { message: "x" } }, 500));
    await expect(getBillingModeApi()).rejects.toBeInstanceOf(ApiError);
  });

  it("moves to a free plan and cancels", async () => {
    fetchMock.mockResolvedValue(res({ id: "s1", planId: "free", status: "active" }));
    await changeMyPlanApi("free");
    await cancelMySubscriptionApi();
    expect([0, 1].map((n) => path(n))).toEqual(["/api/app/billing/change-plan", "/api/app/billing/cancel"]);
    expect(body(0)).toEqual({ planId: "free" });
  });
});

describe("a workspace refused for being archived", () => {
  it("forgets it and goes to the chooser, like a lost membership", async () => {
    const assign = vi.fn();
    vi.stubGlobal("location", { ...window.location, pathname: "/dashboard", assign });
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:WorkspaceArchived", message: "Archived" } }, 403));
    await expect(getApi("/api/app/project")).rejects.toMatchObject({ status: 403, code: "Tracker:WorkspaceArchived" });
    expect(localStorage.getItem("dosi-workspace")).toBeNull();
    expect(assign).toHaveBeenCalledWith("/workspaces");
  });
});
