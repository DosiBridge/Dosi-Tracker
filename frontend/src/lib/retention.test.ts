import { describe, expect, it } from "vitest";
import {
  memberWindowLabel,
  memberWindowMinutes,
  retentionDays,
  retentionDeletesMore,
  retentionDraftOf,
  retentionProblem,
} from "./retention";

describe("retention drafts", () => {
  it("starts from the server's policy, or its defaults for one never stored", () => {
    expect(retentionDraftOf("capture", { isEnabled: true, retentionDays: 14 })).toEqual({ isEnabled: true, days: "14" });
    expect(retentionDraftOf("capture", null)).toEqual({ isEnabled: false, days: "30" });
    expect(retentionDraftOf("activity", { isEnabled: false, retentionDays: 5 })).toEqual({ isEnabled: false, days: "365" });
  });

  it("reads whole days in each policy's range", () => {
    expect(retentionDays("capture", { isEnabled: true, days: " 730 " })).toBe(730);
    expect(retentionDays("capture", { isEnabled: true, days: "731" })).toBeNull();
    expect(retentionDays("activity", { isEnabled: true, days: "89" })).toBeNull();
    expect(retentionDays("activity", { isEnabled: true, days: "1.5" })).toBeNull();
  });

  it("checks the days only while the policy is on", () => {
    expect(retentionProblem("capture", { isEnabled: false, days: "x" })).toBeNull();
    expect(retentionProblem("capture", { isEnabled: true, days: "0" })).toMatch(/from 1 to 730/);
    expect(retentionProblem("activity", { isEnabled: true, days: "" })).toMatch(/from 90 to 3650/);
    expect(retentionProblem("activity", { isEnabled: true, days: "120" })).toBeNull();
  });

  it("asks before saving a change that deletes more: turning it on, or keeping fewer days", () => {
    const off = { isEnabled: false, days: "30" };
    const on30 = { isEnabled: true, days: "30" };
    expect(retentionDeletesMore("capture", off, on30)).toBe(true);
    expect(retentionDeletesMore("capture", on30, { isEnabled: true, days: "10" })).toBe(true);
    expect(retentionDeletesMore("capture", on30, { isEnabled: true, days: "60" })).toBe(false);
    expect(retentionDeletesMore("capture", on30, off)).toBe(false);
    expect(retentionDeletesMore("capture", on30, { isEnabled: true, days: "nope" })).toBe(false);
  });
});

describe("the members' own-capture window", () => {
  it("reads whole minutes up to a week", () => {
    expect(memberWindowMinutes("0")).toBe(0);
    expect(memberWindowMinutes(" 60 ")).toBe(60);
    expect(memberWindowMinutes("10080")).toBe(10080);
    expect(memberWindowMinutes("10081")).toBeNull();
    expect(memberWindowMinutes("-5")).toBeNull();
    expect(memberWindowMinutes("")).toBeNull();
  });

  it("reads as words", () => {
    expect(memberWindowLabel(0)).toBe("Not allowed");
    expect(memberWindowLabel(1)).toBe("1 minute");
    expect(memberWindowLabel(45)).toBe("45 minutes");
    expect(memberWindowLabel(60)).toBe("1 hour");
    expect(memberWindowLabel(120)).toBe("2 hours");
    expect(memberWindowLabel(1440)).toBe("1 day");
    expect(memberWindowLabel(10080)).toBe("7 days");
  });
});
