import { describe, expect, it } from "vitest";
import { ApiError } from "@/hooks/useApi";
import { membershipEditRules } from "@/lib/api-contract";
import {
  appRoleOf,
  editRulesFor,
  memberNameOf,
  membershipErrorText,
  parseHourlyRate,
  projectRoleLabel,
  projectRoleOf,
  rateLabel,
  roleChoices,
} from "./membership";

const apiError = (code: string | null, message: string | null = null, status = 409) =>
  new ApiError(`API POST Error: Conflict${message ? `: ${message}` : ""}`, status, { code, message, details: null, validationErrors: [] });

describe("project roles", () => {
  it("reads a membership's role whatever its case, unknown roles as Worker", () => {
    expect(projectRoleOf("admin")).toBe("Admin");
    expect(projectRoleOf("MANAGER")).toBe("Manager");
    expect(projectRoleOf("Client")).toBe("Client");
    expect(projectRoleOf("Lead")).toBe("Worker");
    expect(projectRoleOf(null)).toBe("Worker");
  });

  it("names roles as the rest of the app does (roleLabels): Worker is a Member, Admin an Administrator", () => {
    expect(projectRoleLabel("Worker")).toBe("Member");
    expect(projectRoleLabel("Admin")).toBe("Administrator");
    expect(projectRoleLabel("Manager")).toBe("Manager");
    expect(projectRoleLabel("Client")).toBe("Client");
  });

  it("maps project roles to the app's roles", () => {
    expect(appRoleOf("Admin")).toBe("admin");
    expect(appRoleOf("Manager")).toBe("admin");
    expect(appRoleOf("Client")).toBe("client");
    expect(appRoleOf("Worker")).toBe("worker");
  });
});

describe("who may change what (contract §7)", () => {
  it("an owner may grant every role and change rates — except on themselves", () => {
    expect(editRulesFor({ id: "me", role: "owner" }, "them")).toMatchObject({ canChangeRole: true, canChangeRate: true });
    expect(editRulesFor({ id: "me", role: "owner" }, "me")).toMatchObject({ canChangeRole: false, canChangeRate: false, roles: [] });
  });

  it("a manager may change roles below Administrator but never rates", () => {
    const rules = editRulesFor({ id: "me", role: "admin" }, "them");
    expect(rules.canChangeRate).toBe(false);
    expect(rules.roles).not.toContain("Admin");
    expect(rules.roles).not.toContain("Manager");
  });

  it("a worker may change nothing", () => {
    expect(editRulesFor({ id: "me", role: "worker" }, "them").roles).toEqual([]);
  });

  it("offers the everyday role first and never offers Manager for a new grant", () => {
    const owner = membershipEditRules({ canManageTeam: true, callerIsOwner: true, isSelf: false });
    expect(roleChoices(owner)).toEqual(["Worker", "Admin", "Client"]);
    // …but shows it when a membership already holds it.
    expect(roleChoices(owner, "Manager")).toEqual(["Worker", "Admin", "Client", "Manager"]);
    const manager = membershipEditRules({ canManageTeam: true, callerIsOwner: false, isSelf: false });
    expect(roleChoices(manager)).toEqual(["Worker", "Client"]);
  });
});

describe("member names", () => {
  it("prefers the roster's name, then the membership's names, never a bare id", () => {
    expect(memberNameOf({ userId: "abc" }, "Sam Lee")).toBe("Sam Lee");
    expect(memberNameOf({ userId: "abc", name: "Kim", surname: "Ito" })).toBe("Kim Ito");
    expect(memberNameOf({ userId: "abc", userName: "kim.ito" })).toBe("kim.ito");
    expect(memberNameOf({ userId: "3fa85f64-5717" })).toBe("Team member");
  });
});

describe("hourly rates", () => {
  it("accepts 0–10,000 with at most cents; blank is no rate", () => {
    expect(parseHourlyRate("")).toBe(0);
    expect(parseHourlyRate("25")).toBe(25);
    expect(parseHourlyRate("25.5")).toBe(25.5);
    expect(parseHourlyRate("10000")).toBe(10000);
    expect(parseHourlyRate("10000.01")).toBeNull();
    expect(parseHourlyRate("-1")).toBeNull();
    expect(parseHourlyRate("1e3")).toBeNull();
    expect(parseHourlyRate("2.555")).toBeNull();
  });

  it("formats a rate for display", () => {
    expect(rateLabel(25)).toBe("$25.00/h");
    expect(rateLabel(0)).toBe("No rate");
    expect(rateLabel(null)).toBe("No rate");
  });
});

describe("membershipErrorText", () => {
  it("turns the business errors this flow meets into a next step", () => {
    expect(membershipErrorText(apiError("Tracker:SeatLimitReached"), "x")).toMatch(/no free seat/i);
    expect(membershipErrorText(apiError("Tracker:DuplicateProjectMember"), "x")).toMatch(/already on this project/i);
    expect(membershipErrorText(apiError("Tracker:InvalidUser", null, 404), "x")).toMatch(/isn't in this workspace/i);
    expect(membershipErrorText(apiError("Tracker:CannotModifySelf", null, 403), "x")).toMatch(/your own role/i);
    expect(membershipErrorText(apiError("Tracker:RoleNotAllowed", null, 403), "x")).toMatch(/only the workspace owner/i);
  });

  it("reads the server's own message for anything else", () => {
    expect(membershipErrorText(apiError("Volo.Abp:Other", "Something specific."), "fallback")).toBe("Something specific.");
    expect(membershipErrorText(new Error("boom"), "Couldn't add them.")).toBe("Couldn't add them.");
  });
});
