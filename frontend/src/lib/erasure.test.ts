import { describe, expect, it } from "vitest";
import {
  REASON_PROBLEM_TEXT,
  blockErasure,
  erasureErrorText,
  erasureReasonProblem,
  purgeDateLabel,
  workspaceNameConfirmed,
  type ErasureViewer,
} from "./erasure";

const NOW = new Date("2026-10-26T10:00:00Z");
const manager: ErasureViewer = { canDeleteActivities: true, isOwner: false, userId: "m" };
const owner: ErasureViewer = { canDeleteActivities: true, isOwner: true, userId: "o" };
const member: ErasureViewer = { canDeleteActivities: false, isOwner: false, userId: "u" };

describe("blockErasure — which controls a block offers", () => {
  it("lets owners and managers delete a member's block and any capture", () => {
    expect(blockErasure({ viewer: manager, blockUserId: "u", blockOwnerIsOwner: false, now: NOW })).toEqual({ block: true, captures: "any", ownUntil: null });
    expect(blockErasure({ viewer: owner, blockUserId: "u", blockOwnerIsOwner: false, now: NOW }).block).toBe(true);
  });

  it("leaves an owner's data to owners", () => {
    expect(blockErasure({ viewer: manager, blockUserId: "o", blockOwnerIsOwner: true, now: NOW })).toEqual({ block: false, captures: "none", ownUntil: null });
    expect(blockErasure({ viewer: owner, blockUserId: "o2", blockOwnerIsOwner: true, now: NOW }).block).toBe(true);
  });

  it("lets a member delete their own captures only within the window after upload", () => {
    const uploadedAt = "2026-10-26T09:30:00Z";
    expect(blockErasure({ viewer: member, blockUserId: "u", blockOwnerIsOwner: false, uploadedAt, now: NOW })).toEqual({
      block: false,
      captures: "own",
      ownUntil: Date.parse("2026-10-26T10:30:00Z"),
    });
    expect(blockErasure({ viewer: member, blockUserId: "u", blockOwnerIsOwner: false, uploadedAt, windowMinutes: 15, now: NOW }).captures).toBe("none");
    expect(blockErasure({ viewer: member, blockUserId: "u", blockOwnerIsOwner: false, uploadedAt, windowMinutes: 0, now: NOW }).captures).toBe("none");
  });

  it("offers a member nothing on someone else's block, or when the upload time is unknown", () => {
    expect(blockErasure({ viewer: member, blockUserId: "x", blockOwnerIsOwner: false, uploadedAt: "2026-10-26T09:59:00Z", now: NOW }).captures).toBe("none");
    expect(blockErasure({ viewer: member, blockUserId: "u", blockOwnerIsOwner: false, uploadedAt: null, now: NOW }).captures).toBe("none");
    expect(blockErasure({ viewer: member, blockUserId: "u", blockOwnerIsOwner: false, uploadedAt: "garbage", now: NOW }).captures).toBe("none");
  });
});

describe("erasureReasonProblem — the reason kept with every deletion", () => {
  it("requires one from owners and managers, within the server's bounds", () => {
    expect(erasureReasonProblem("  ", true)).toBe("missing");
    expect(erasureReasonProblem("ab", true)).toBe("short");
    expect(erasureReasonProblem("x".repeat(513), true)).toBe("long");
    expect(erasureReasonProblem("Banking page", true)).toBeNull();
    for (const p of ["missing", "short", "long"] as const) expect(REASON_PROBLEM_TEXT[p]).toBeTruthy();
  });

  it("lets a member leave it out, but still checks one they give", () => {
    expect(erasureReasonProblem("", false)).toBeNull();
    expect(erasureReasonProblem("no", false)).toBe("short");
  });
});

describe("workspaceNameConfirmed — compared as the server compares it", () => {
  it("ignores surrounding spaces and letter case, and never matches an empty name", () => {
    expect(workspaceNameConfirmed(" acme corp ", "Acme Corp")).toBe(true);
    expect(workspaceNameConfirmed("Acme", "Acme Corp")).toBe(false);
    expect(workspaceNameConfirmed("", " ")).toBe(false);
  });
});

describe("erasureErrorText", () => {
  it.each([
    ["Tracker:CaptureDeleteNotAllowed", /time to delete your own capture has passed/],
    ["Tracker:RoleNotAllowed", /only a workspace owner/i],
    ["Tracker:WorkspaceDeleteConfirmationMismatch", /exactly/],
    ["Tracker:InvalidPassword", /current password/],
    ["Tracker:AccountDeletionNotAllowed", /platform administrator/],
    ["Tracker:SoleWorkspaceOwner", /only owner/],
  ])("explains %s", (code, text) => {
    expect(erasureErrorText({ code }, "x")).toMatch(text);
  });

  it("prefers the server's own words for a sole owner (it names the workspaces), and falls back by status", () => {
    expect(erasureErrorText({ code: "Tracker:SoleWorkspaceOwner", serverMessage: "You are the only owner of Acme." }, "x")).toBe("You are the only owner of Acme.");
    expect(erasureErrorText({ status: 404 }, "x")).toMatch(/already deleted/);
    expect(erasureErrorText({ status: 403 }, "x")).toMatch(/aren't allowed/);
    expect(erasureErrorText({ status: 500, serverMessage: "Boom" }, "x")).toBe("Boom");
    expect(erasureErrorText(null, "Fallback")).toBe("Fallback");
  });
});

describe("purgeDateLabel", () => {
  it("reads the server's UTC instant, with or without its zone", () => {
    expect(purgeDateLabel("2026-11-02T08:00:00Z")).toBe("Nov 2, 2026");
    expect(purgeDateLabel("2026-11-02T23:30:00")).toBe("Nov 2, 2026");
    expect(purgeDateLabel(null)).toBeNull();
    expect(purgeDateLabel("soon")).toBeNull();
  });
});
