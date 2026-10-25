// Demo AI output must be deterministic, computed from the demo dataset with the
// canonical metrics (so its numbers agree with every other demo page), honest
// about what wasn't measured, and made without any network call.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoAiResult, demoAnswer, demoDataQualityCandidates, formatAiDuration } from "./ai-demo";
import { demoAiStatus } from "./ai-demo-status";
import { AI_PROMPT_CHIPS, DEMO_NOTE } from "./ai-assist";
import { activityPercent, dayKeyIn, minutesByProject, totalIdleMinutes, totalMinutes } from "./metrics";
import { DAY_MS, NOW, TODAY } from "./demo-clock";
import { datasetFor, type Dataset } from "./tenant-data";
import { formatInZone } from "./time-zone";
import { friendlyAppName } from "./apps";
import { resetTenantDataForTests } from "./demo-tenants";
import { buildActivity, buildProject, buildUser, buildWindowInfo, resetFactories } from "@/test/factories";
import type { Activity, Project, User } from "./types";

const WEEK = { from: new Date(NOW.getTime() - 7 * DAY_MS).toISOString(), to: new Date(NOW.getTime() + 1).toISOString() };

const inRange = (rows: Activity[], from: string, to: string) =>
  rows.filter((a) => Date.parse(a.endedAt) >= Date.parse(from) && Date.parse(a.endedAt) < Date.parse(to));

function topProject(ds: Dataset, rows: Activity[]): { title: string; minutes: number } {
  const [id, minutes] = [...minutesByProject(rows)].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0];
  return { title: ds.projects.find((p) => p.id === id)?.title ?? "Unassigned work", minutes };
}

/** A workspace of the test's own rows (an unknown demo id starts empty). */
function seed(activities: Activity[], users: User[] = [], projects: Project[] = [buildProject({ id: "p1", title: "Apollo" })]): Dataset {
  const ds = datasetFor("ai-test");
  ds.activities = activities;
  ds.users = users;
  ds.projects = projects;
  return ds;
}

const at = (iso: string, minutes: number, overrides: Partial<Activity> = {}) =>
  buildActivity({ startedAt: iso, endedAt: new Date(Date.parse(iso) + minutes * 60_000).toISOString(), ...overrides });

let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  resetTenantDataForTests();
  resetFactories();
  fetchSpy = vi.fn();
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  expect(fetchSpy, "demo AI output must never touch the network").not.toHaveBeenCalled();
  vi.unstubAllGlobals();
  resetTenantDataForTests();
});

describe("demoAiStatus", () => {
  it("opens every feature to owners and managers, with team scopes and the month's quota", () => {
    for (const role of ["owner", "admin"] as const) {
      const s = demoAiStatus(role);
      expect(s).toMatchObject({ available: true, reason: null, workspaceEnabled: true, canManage: role === "owner", disclosureVersion: "1.0" });
      expect(s.features.every((f) => f.allowed && f.reason === null)).toBe(true);
      expect(s.features.find((f) => f.featureId === "period-summary")?.scopes).toEqual(["team", "self"]);
      expect(s.features.find((f) => f.featureId === "member-day-summary")?.scopes).toEqual(["member", "self"]);
      expect(s.features.find((f) => f.featureId === "ask")?.scopes).toEqual([]);
      expect(s.quota?.resetsAt).toBe("2026-08-01T00:00:00.000Z");
    }
  });

  it("limits members to their own data and refuses report explanations", () => {
    const s = demoAiStatus("worker");
    expect(s.available).toBe(true);
    expect(s.quota).toBeNull();
    expect(s.features.find((f) => f.featureId === "report-explanation")).toEqual({ featureId: "report-explanation", allowed: false, reason: "role", scopes: [] });
    expect(s.features.find((f) => f.featureId === "data-quality-review")?.scopes).toEqual(["self"]);
    expect(s.features.find((f) => f.featureId === "note-suggestions")?.allowed).toBe(true);
  });

  it("refuses clients and the host everything", () => {
    for (const role of ["client", "host"] as const) {
      const s = demoAiStatus(role);
      expect(s).toMatchObject({ available: false, reason: "role-not-allowed", canManage: false, quota: null });
      expect(s.features.every((f) => !f.allowed && f.reason === "role" && f.scopes.length === 0)).toBe(true);
    }
  });
});

describe("formatAiDuration", () => {
  it("writes the server's display format", () => {
    expect(formatAiDuration(412)).toBe("6 h 52 min");
    expect(formatAiDuration(45)).toBe("45 min");
    expect(formatAiDuration(120)).toBe("2 h");
    expect(formatAiDuration(0)).toBe("0 min");
    expect(formatAiDuration(-5)).toBe("0 min");
    expect(formatAiDuration(59.6)).toBe("1 h");
  });
});

describe("period-summary", () => {
  it("summarises the demo team's week with the dataset's own figures", () => {
    const ds = datasetFor("w1");
    const rows = inRange(ds.activities, WEEK.from, WEEK.to);
    const prevFrom = new Date(Date.parse(WEEK.from) - (Date.parse(WEEK.to) - Date.parse(WEEK.from))).toISOString();
    const previous = totalMinutes(inRange(ds.activities, prevFrom, WEEK.from));
    const total = totalMinutes(rows);
    const result = demoAiResult("period-summary", { ...WEEK, scope: "team", timeZone: "UTC" }, "w1");

    expect(result.meta).toMatchObject({ featureId: "period-summary", status: "demo", disclaimer: DEMO_NOTE, model: null, generatedAt: NOW.toISOString() });
    expect(rows.length).toBeGreaterThan(0);
    // The demo tracks its last few days only, so the week before is empty: no change to report.
    expect(previous).toBe(0);
    expect(result.headline).toBe(`The team tracked ${formatAiDuration(total)} in this period.`);

    const top = topProject(ds, rows);
    expect(result.insights[0].text).toBe(`${top.title} took the most time: ${formatAiDuration(top.minutes)} (${Math.round((top.minutes / total) * 100)}%).`);
    expect(result.insights[0].routeId).toBe("projects");
    expect(result.insights.find((i) => i.kind === "coverage")?.text).toContain(`${activityPercent(rows)}%`);
    expect(result.insights.length).toBeLessThanOrEqual(5);
    expect(result.nextSteps.map((s) => s.routeId)).toEqual(["reports.weekly", "timesheet"]);
    expect(result.caveats).toContain("This period includes today, so its figures can still change.");
  });

  it("compares with the period before when it had tracked time", () => {
    seed([at("2026-07-06T09:00:00Z", 60), at("2026-07-07T09:00:00Z", 90), at("2026-07-08T09:00:00Z", 30)]);
    const up = demoAiResult("period-summary", { from: "2026-07-07T00:00:00Z", to: "2026-07-08T00:00:00Z", scope: "team" }, "ai-test");
    expect(up.headline).toBe("The team tracked 1 h 30 min in this period, +50% vs the period before.");
    expect(up.insights[0].facts.map((f) => f.display)).toEqual(["1 h 30 min", "100%"]);
    const down = demoAiResult("period-summary", { from: "2026-07-08T00:00:00Z", to: "2026-07-09T00:00:00Z", scope: "team" }, "ai-test");
    expect(down.headline).toBe("The team tracked 30 min in this period, -67% vs the period before.");
  });

  it("is deterministic", () => {
    const input = { ...WEEK, scope: "team" as const };
    expect(demoAiResult("period-summary", input, "w1")).toEqual(demoAiResult("period-summary", input, "w1"));
    expect(demoAiResult("period-summary", input, "w1").meta.resultId).not.toBe(demoAiResult("period-summary", { ...input, scope: "self" }, "w1").meta.resultId);
  });

  it("covers only the viewer's own rows for scope self, and one project's when asked", () => {
    const ds = datasetFor("w1");
    const own = inRange(ds.activities, WEEK.from, WEEK.to).filter((a) => a.userId === "u1");
    const self = demoAiResult("period-summary", { ...WEEK, scope: "self" }, "w1", { userId: "u1" });
    expect(self.headline.startsWith(`You tracked ${formatAiDuration(totalMinutes(own))}`)).toBe(true);
    expect(self.insights.find((i) => i.kind === "coverage")?.routeId).toBe("timesheet");

    const projectRows = inRange(ds.activities, WEEK.from, WEEK.to).filter((a) => a.projectId === "p1");
    const project = demoAiResult("period-summary", { ...WEEK, scope: "team", projectId: "p1" }, "w1");
    expect(project.headline.startsWith(`This project's team tracked ${formatAiDuration(totalMinutes(projectRows))}`)).toBe(true);
    expect(project.insights[0].routeId).toBeNull();
  });

  it("says nothing was tracked for an empty period", () => {
    const result = demoAiResult("period-summary", { from: "2020-01-01T00:00:00Z", to: "2020-01-08T00:00:00Z", scope: "team" }, "w1");
    expect(result).toMatchObject({ headline: "Nothing was tracked for this period.", insights: [], nextSteps: [], caveats: [] });
    expect(result.meta.status).toBe("no-data");
  });

  it("leaves out the change without a previous period, and flags unmeasured input", () => {
    seed([
      at("2026-07-06T09:00:00Z", 60, { activeWindows: [] }),
      at("2026-07-06T11:00:00Z", 30, { activityMeasured: false, productivity: 0, activeWindows: [] }),
    ]);
    const result = demoAiResult("period-summary", { from: "2026-07-06T00:00:00Z", to: "2026-07-07T00:00:00Z", scope: "team" }, "ai-test");
    expect(result.headline).toBe("The team tracked 1 h 30 min in this period.");
    expect(result.insights.map((i) => i.kind)).toEqual(["distribution", "coverage", "rhythm"]);
    expect(result.insights[2].text).toBe("The busiest day was Mon, 6 Jul, with 1 h 30 min.");
    expect(result.caveats).toEqual([expect.stringMatching(/no keyboard or mouse measurement/)]);
  });

  it("never turns an unmeasured period into 0% activity", () => {
    seed([at("2026-07-06T09:00:00Z", 60, { activityMeasured: false, productivity: 0 })]);
    const result = demoAiResult("period-summary", { from: "2026-07-06T00:00:00Z", to: "2026-07-07T00:00:00Z", scope: "team" }, "ai-test");
    const coverage = result.insights.find((i) => i.kind === "coverage")!;
    expect(coverage.text).toBe("Keyboard and mouse input was not measured for this period.");
    expect(coverage.facts[0].display).toBe("not measured");
  });

  it("reads days in the input's zone", () => {
    seed([at("2026-07-06T22:30:00Z", 60)]);
    const input = { from: "2026-07-06T00:00:00Z", to: "2026-07-08T00:00:00Z", scope: "team" as const };
    expect(demoAiResult("period-summary", { ...input, timeZone: "Asia/Dhaka" }, "ai-test").insights.at(-1)?.text).toMatch(/Tue, 7 Jul/);
    expect(demoAiResult("period-summary", input, "ai-test", { timeZone: "UTC" }).insights.at(-1)?.text).toMatch(/Mon, 6 Jul/);
  });
});

describe("member-day-summary", () => {
  it("describes a member's day with the dataset's own figures", () => {
    const ds = datasetFor("w1");
    const rows = ds.activities.filter((a) => a.userId === "u2" && dayKeyIn(a, "UTC") === TODAY);
    expect(rows.length).toBeGreaterThan(0);
    const result = demoAiResult("member-day-summary", { userId: "u2", date: TODAY, timeZone: "UTC" }, "w1", { userId: "u1" });
    const name = ds.users.find((u) => u.id === "u2")!.name;
    expect(result.headline).toBe(`${name} tracked ${formatAiDuration(totalMinutes(rows))} on Tue, 14 Jul.`);
    const first = formatInZone(Math.min(...rows.map((a) => Date.parse(a.startedAt))), "UTC", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
    expect(result.summary.startsWith(`Work ran from ${first} to `)).toBe(true);
    expect(result.summary).toContain(topProject(ds, rows).title);
    expect(result.highlights.find((h) => h.kind === "breaks")?.facts[0].display).toBe(formatAiDuration(totalIdleMinutes(rows)));
    expect(result.highlights.length).toBeLessThanOrEqual(5);
    expect(result.meta.status).toBe("demo");
  });

  it("speaks to the viewer about their own day, and defaults to them", () => {
    const result = demoAiResult("member-day-summary", { date: TODAY }, "w1", { userId: "u1", timeZone: "UTC" });
    expect(result.headline.startsWith("You tracked ")).toBe(true);
  });

  it("joins back-to-back blocks into one span, and leaves out what it can't know", () => {
    seed([
      at("2026-07-06T09:00:00Z", 30, { userId: "m1", activeWindows: [] }),
      at("2026-07-06T09:31:00Z", 29, { userId: "m1", activeWindows: [] }),
      at("2026-07-06T11:00:00Z", 10, { userId: "m1", activeWindows: [], activityMeasured: false, productivity: 0 }),
    ]);
    const result = demoAiResult("member-day-summary", { userId: "m1", date: "2026-07-06", timeZone: "UTC" }, "ai-test");
    expect(result.headline.startsWith("This member tracked 1 h 9 min")).toBe(true);
    expect(result.highlights.find((h) => h.kind === "focus")?.text).toBe("The longest continuous span was 1 h.");
    expect(result.highlights.some((h) => h.kind === "apps")).toBe(false);
    expect(result.summary).toBe("Work ran from 09:00 to 11:10. Most time went to Apollo.");
    expect(result.caveats).toHaveLength(1);
  });

  it("has a rhythm line only when input was measured", () => {
    seed([at("2026-07-06T09:00:00Z", 30, { userId: "m1", activityMeasured: false, productivity: 0 })]);
    const result = demoAiResult("member-day-summary", { userId: "m1", date: "2026-07-06" }, "ai-test");
    expect(result.highlights.some((h) => h.kind === "rhythm")).toBe(false);
    expect(result.highlights.some((h) => h.kind === "apps")).toBe(true);
  });

  it("says nothing was tracked on an empty day", () => {
    const result = demoAiResult("member-day-summary", { userId: "u2", date: "2020-01-01" }, "w1");
    expect(result).toMatchObject({ headline: "Nothing was tracked on this day.", summary: "", highlights: [], caveats: [] });
    expect(result.meta.status).toBe("no-data");
  });
});

describe("report-explanation", () => {
  it("explains a report's period with its totals and per-report caveat", () => {
    const ds = datasetFor("w1");
    const rows = inRange(ds.activities, WEEK.from, WEEK.to);
    const result = demoAiResult("report-explanation", { reportSlug: "attendance", ...WEEK }, "w1");
    const members = new Set(rows.map((a) => a.userId)).size;
    const days = new Set(rows.map((a) => a.endedAt.slice(0, 10))).size;
    expect(result.summary).toBe(`This report covers ${formatAiDuration(totalMinutes(rows))} of tracked time from ${members} members over ${days} days.`);
    expect(result.caveats).toEqual(['"Absent" here means no tracked time, not a missed shift.']);
    expect(result.keyFindings[0].routeId).toBe("reports.projects");
    expect(result.questionsToExplore).toHaveLength(2);
  });

  it("narrows to a member, and flags unmeasured input", () => {
    seed([at("2026-07-06T09:00:00Z", 30, { userId: "m1", activityMeasured: false, productivity: 0 }), at("2026-07-06T10:00:00Z", 30, { userId: "m2" })]);
    const result = demoAiResult("report-explanation", { reportSlug: "weekly", from: "2026-07-06T00:00:00Z", to: "2026-07-07T00:00:00Z", userId: "m1" }, "ai-test");
    expect(result.summary).toBe("This report covers 30 min of tracked time from 1 members over 1 days.");
    expect(result.caveats).toEqual([expect.stringMatching(/no keyboard or mouse measurement/)]);
  });

  it("says nothing was tracked for an empty period", () => {
    const result = demoAiResult("report-explanation", { reportSlug: "weekly", from: "2020-01-01T00:00:00Z", to: "2020-01-02T00:00:00Z" }, "w1");
    expect(result).toMatchObject({ keyFindings: [], caveats: [], questionsToExplore: [] });
    expect(result.meta.status).toBe("no-data");
  });
});

describe("data quality", () => {
  const to = Date.parse("2026-07-11T00:00:00Z"); // Saturday

  it("finds each rule's candidate, and only real ones", () => {
    const rows = [
      // m1: input unmeasured for most of the time
      at("2026-07-06T09:00:00Z", 60, { userId: "m1", activityMeasured: false, productivity: 0 }),
      at("2026-07-10T09:00:00Z", 20, { userId: "m1" }),
      // m2: a device clock 10 minutes out, and software input in three blocks
      at("2026-07-06T09:00:00Z", 10, { userId: "m2", clockSkewSeconds: -600, injectedInputEvents: 4 }),
      at("2026-07-07T09:00:00Z", 10, { userId: "m2", injectedInputEvents: 1 }),
      at("2026-07-08T09:00:00Z", 10, { userId: "m2", injectedInputEvents: 2 }),
      at("2026-07-09T09:00:00Z", 10, { userId: "m2" }),
      at("2026-07-10T09:00:00Z", 10, { userId: "m2" }),
      // m3: tracked Monday only (Tue-Fri missing), on an older Windows agent
      at("2026-07-06T09:00:00Z", 10, { userId: "m3", platform: "windows", agentVersion: "0.1.4" }),
      // m4: newest Windows agent; an unparsable macOS build is skipped
      at("2026-07-07T09:00:00Z", 10, { userId: "m4", platform: "windows", agentVersion: "0.2.0" }),
      at("2026-07-08T09:00:00Z", 10, { userId: "m4", platform: "windows", agentVersion: "0.2.0" }),
      at("2026-07-09T09:00:00Z", 10, { userId: "m4", platform: "macos", agentVersion: "beta" }),
      at("2026-07-10T09:00:00Z", 10, { userId: "m4", platform: "linux" }),
    ];
    const members = [
      buildUser({ id: "m1" }),
      buildUser({ id: "m5", joinedAt: "2026-06-01" }), // never tracked
      buildUser({ id: "m6", joinedAt: "2026-07-10" }), // joined too recently to count
      buildUser({ id: "m7", role: "client", joinedAt: "2026-01-01" }), // clients never track
      buildUser({ id: "m8", joinedAt: "" }), // unknown join date: not flagged
    ];
    const found = demoDataQualityCandidates(rows, members, { scope: "team", to });
    expect(found).toEqual([
      { kind: "data-gap", userId: "m1", value: "3 weekdays" },
      { kind: "unmeasured-activity", userId: "m1", value: "75%" },
      { kind: "clock-skew", userId: "m2", value: "10 min" },
      { kind: "software-input", userId: "m2", value: "3" },
      { kind: "data-gap", userId: "m3", value: "4 weekdays" },
      { kind: "outdated-agent", userId: "m3", value: "0.1.4" },
      { kind: "never-tracked", userId: "m5", value: "this period" },
    ]);
  });

  it("keeps team-only rules out of a self review", () => {
    const rows = [at("2026-07-06T09:00:00Z", 10, { userId: "m3", platform: "windows", agentVersion: "0.1.4" }), at("2026-07-07T09:00:00Z", 10, { userId: "m4", platform: "windows", agentVersion: "0.2.0" })];
    expect(demoDataQualityCandidates(rows, [buildUser({ id: "m5", joinedAt: "2026-01-01" })], { scope: "self", to }).map((c) => `${c.kind}:${c.userId}`)).toEqual([
      "data-gap:m3",
      "data-gap:m4",
    ]);
  });

  it("writes up the candidates, notices first, at most five, and filters by kind", () => {
    const rows = ["m1", "m2", "m3", "m4", "m5", "m6"].map((userId) => at("2026-07-06T09:00:00Z", 30, { userId, activityMeasured: false, productivity: 0 }));
    seed(rows, [buildUser({ id: "m1", name: "Rafi Karim" })]);
    const input = { from: "2026-07-06T00:00:00Z", to: "2026-07-11T00:00:00Z", scope: "team" as const, timeZone: "UTC" };
    const result = demoAiResult("data-quality-review", input, "ai-test", { userId: "m2" });
    expect(result.candidateCount).toBe(12); // each member: unmeasured input and a gap
    expect(result.items).toHaveLength(5);
    expect(result.items.every((i) => i.kind === "data-gap" && i.severity === "notice")).toBe(true);
    expect(result.items[0]).toMatchObject({
      candidateId: "data-gap:m1",
      title: "Gap in tracked days",
      explanation: "No time was tracked for 4 weekdays in a row.",
      actionId: "check-agent-running",
      actionRouteId: "download",
      subjectName: "Rafi Karim",
      subjectUserId: "m1",
      facts: [{ id: "F1", label: "Gap in tracked days", display: "4 weekdays" }],
    });
    expect(result.items[1].subjectName).toBe("you");
    expect(result.items[2].subjectName).toBeNull();

    const unmeasured = demoAiResult("data-quality-review", { ...input, kinds: ["unmeasured-activity"] }, "ai-test");
    expect(unmeasured.items.map((i) => i.kind)).toEqual(Array(5).fill("unmeasured-activity"));
    expect(unmeasured.items[0]).toMatchObject({ severity: "info", explanation: "Keyboard and mouse input wasn't measured for 100% of tracked time." });
  });

  it("reviews only the viewer's rows for scope self", () => {
    seed([at("2026-07-06T09:00:00Z", 30, { userId: "m1", clockSkewSeconds: 900 }), at("2026-07-06T09:00:00Z", 30, { userId: "m2", clockSkewSeconds: 900 })]);
    const result = demoAiResult(
      "data-quality-review",
      { from: "2026-07-06T00:00:00Z", to: "2026-07-07T00:00:00Z", scope: "self", kinds: ["clock-skew", "software-input"] },
      "ai-test",
      { userId: "m2" },
    );
    expect(result.items.map((i) => i.candidateId)).toEqual(["clock-skew:m2"]);
    expect(result.items[0].explanation).toBe("A device's clock was out by up to 15 min, which can shift block times.");
    expect(result.items[0].actionRouteId).toBeNull();
  });

  it("finds the demo team's outdated agent", () => {
    const result = demoAiResult("data-quality-review", { ...WEEK, scope: "team", kinds: ["outdated-agent"] }, "w1");
    expect(result.items.map((i) => i.subjectUserId)).toContain("u4");
    expect(result.items.find((i) => i.subjectUserId === "u4")?.explanation).toBe("This device runs agent 0.1.4, older than the newest one in the workspace.");
  });

  it("is no-data with nothing to flag", () => {
    seed([]);
    const result = demoAiResult("data-quality-review", { from: "2026-07-06T00:00:00Z", to: "2026-07-07T00:00:00Z", scope: "team" }, "ai-test");
    expect(result).toMatchObject({ items: [], candidateCount: 0 });
    expect(result.meta.status).toBe("no-data");
  });

  it("explains the remaining rules in neutral words", () => {
    seed(
      [
        at("2026-07-06T09:00:00Z", 10, { userId: "m1", injectedInputEvents: 1 }),
        at("2026-07-07T09:00:00Z", 10, { userId: "m1", injectedInputEvents: 1 }),
        at("2026-07-08T09:00:00Z", 10, { userId: "m1", injectedInputEvents: 1 }),
        at("2026-07-08T10:00:00Z", 10, { userId: "m2", platform: "linux", agentVersion: "1.0" }),
        at("2026-07-08T11:00:00Z", 10, { userId: "m3", platform: "linux", agentVersion: "1.0.1" }),
      ],
      [buildUser({ id: "m9", joinedAt: "2026-01-01" })],
    );
    const result = demoAiResult(
      "data-quality-review",
      { from: "2026-07-06T00:00:00Z", to: "2026-07-09T00:00:00Z", scope: "team", kinds: ["software-input", "outdated-agent", "never-tracked"] },
      "ai-test",
    );
    const text = result.items.map((i) => `${i.kind}: ${i.explanation}`);
    expect(text).toEqual([
      "never-tracked: This member hasn't tracked any time in this period.",
      "software-input: 3 blocks had input generated by software, which is left out of activity %.",
      "outdated-agent: This device runs agent 1.0, older than the newest one in the workspace.",
    ]);
    for (const i of result.items) expect(`${i.title} ${i.explanation} ${i.possibleReasons.join(" ")}`).not.toMatch(/cheat|fraud|fak|lazy|slack/i);
  });
});

describe("note-suggestions", () => {
  it("suggests notes from the block's project, app and the previous note", () => {
    seed([
      at("2026-07-06T08:00:00Z", 50, { id: "prev", note: "Sprint planning" }),
      at("2026-07-06T09:00:00Z", 30, { id: "b1", activeWindows: [buildWindowInfo({ appName: "figma.exe", seconds: 900 }), buildWindowInfo({ appName: "Code", seconds: 300 })] }),
    ]);
    const result = demoAiResult("note-suggestions", { activityId: "b1" }, "ai-test");
    expect(result.suggestions).toEqual([
      { text: "Worked on Apollo in Figma", basis: "apps" },
      { text: "Worked on Apollo", basis: "project" },
      { text: "Sprint planning", basis: "previous-note" },
    ]);
    expect(result.meta.status).toBe("demo");
  });

  it("skips what it doesn't have, and an unknown block is no-data", () => {
    seed([at("2026-07-06T03:00:00Z", 10, { id: "old", note: "Too long ago" }), at("2026-07-06T09:00:00Z", 30, { id: "b1", projectId: "gone", activeWindows: [] })]);
    expect(demoAiResult("note-suggestions", { activityId: "b1" }, "ai-test").suggestions).toEqual([{ text: "Worked on Unassigned work", basis: "project" }]);
    const missing = demoAiResult("note-suggestions", { activityId: "nope" }, "ai-test");
    expect(missing.suggestions).toEqual([]);
    expect(missing.meta.status).toBe("no-data");
  });

  it("works on a demo block", () => {
    const block = datasetFor("w1").activities[0];
    expect(demoAiResult("note-suggestions", { activityId: block.id }, "w1").suggestions.length).toBeGreaterThan(0);
  });
});

describe("task-drafts", () => {
  it("drafts up to the asked number of tasks for the project, never more than eight", () => {
    const project = datasetFor("w1").projects[0];
    const three = demoAiResult("task-drafts", { projectId: project.id, goal: "  Launch the beta  ", maxTasks: 3 }, "w1");
    expect(three.drafts).toHaveLength(3);
    expect(three.drafts[0]).toEqual({
      title: "Define the scope: Launch the beta",
      description: `List what "Launch the beta" covers for ${project.title}, and what stays out of scope.`,
      rationale: "A clear scope keeps the other tasks small.",
      possibleDuplicateOfTaskId: null,
    });
    expect(demoAiResult("task-drafts", { projectId: project.id, goal: "Launch" }, "w1").drafts).toHaveLength(5);
    expect(demoAiResult("task-drafts", { projectId: project.id, goal: "Launch", maxTasks: 20 }, "w1").drafts).toHaveLength(8);
    expect(demoAiResult("task-drafts", { projectId: project.id, goal: "Launch", maxTasks: 0 }, "w1").drafts).toHaveLength(1);
    const long = demoAiResult("task-drafts", { projectId: project.id, goal: "x".repeat(400), maxTasks: 8 }, "w1");
    expect(long.drafts.every((d) => d.title.length <= 256 && (d.description ?? "").length <= 600)).toBe(true);
    expect(long.drafts.map((d) => d.description).every(Boolean)).toBe(true);
  });

  it("is no-data for an unknown project or an empty goal", () => {
    expect(demoAiResult("task-drafts", { projectId: "nope", goal: "Launch" }, "w1")).toMatchObject({ drafts: [], meta: { status: "no-data" } });
    expect(demoAiResult("task-drafts", { projectId: datasetFor("w1").projects[0].id, goal: "   " }, "w1").drafts).toEqual([]);
  });
});

describe("demoAnswer", () => {
  it("answers the week overview from the dataset", () => {
    const ds = datasetFor("w1");
    const week = inRange(ds.activities, WEEK.from, WEEK.to);
    const answer = demoAnswer("week-overview", "dashboard", "w1");
    expect(answer.answer.split("\n")[0]).toBe(`The team tracked ${formatAiDuration(totalMinutes(week))} in the last 7 days. Most time went to:`);
    expect(answer.answer.split("\n").slice(1).every((l) => l.startsWith("- "))).toBe(true);
    expect(answer.answer).toContain(`- ${topProject(ds, week).title}: ${formatAiDuration(topProject(ds, week).minutes)}`);
    expect(answer).toMatchObject({ conversationId: "demo-conversation", isClarification: false, meta: { status: "demo" } });
    expect(answer.facts[0]).toEqual({ id: "F1", label: "Tracked in the last 7 days", display: formatAiDuration(totalMinutes(week)) });
    expect(answer.sources).toEqual([{ label: "Weekly summary", routeId: "reports.weekly", routeArg: null }]);
  });

  it("answers every chip, deterministically", () => {
    for (const chip of new Set(Object.values(AI_PROMPT_CHIPS).flatMap((c) => c.map((x) => x.id)))) {
      const a = demoAnswer(chip, "dashboard");
      expect(a.isClarification, chip).toBe(false);
      expect(a.answer.length).toBeGreaterThan(10);
      expect(a).toEqual(demoAnswer(chip, "dashboard", "w1"));
    }
  });

  it("narrows today's overview to the viewer", () => {
    const today = inRange(datasetFor("w1").activities, new Date(NOW.getTime() - DAY_MS).toISOString(), WEEK.to).filter((a) => a.userId === "u1");
    expect(demoAnswer("day-overview", "monitor", "w1", { userId: "u1" }).answer.split("\n")[0]).toBe(
      `${formatAiDuration(totalMinutes(today))} was tracked today. By project:`,
    );
  });

  it("says so when there's nothing to report", () => {
    seed([]);
    expect(demoAnswer("week-overview", "dashboard", "ai-test").answer).toBe("Nothing was tracked in the last 7 days.");
    expect(demoAnswer("top-projects", "projects", "ai-test").answer).toBe("Nothing was tracked in the last 7 days.");
    expect(demoAnswer("day-overview", "monitor", "ai-test").answer).toBe("Nothing was tracked today yet.");
    expect(demoAnswer("top-apps", "activities", "ai-test").answer).toBe("No app time was recorded recently.");
    expect(demoAnswer("data-gaps", "team", "ai-test").answer).toBe("Nothing in this week's tracking looks unusual.");
  });

  it("falls back to the week's apps when nothing was tracked today", () => {
    // Wednesday 8 July: nothing since, so also a four-weekday gap.
    seed([at(new Date(NOW.getTime() - 6 * DAY_MS).toISOString(), 30)]);
    expect(demoAnswer("top-apps", "activities", "ai-test").answer).toBe(
      `The apps with the most focused time in the last 7 days:\n- ${friendlyAppName("Visual Studio Code")}: 5 min`,
    );
    expect(demoAnswer("data-gaps", "team", "ai-test").answer).toBe("1 thing in this week's tracking is worth a look:\n- Gap in tracked days");
  });

  it("answers only chips: anything else lists the page's questions", () => {
    const a = demoAnswer("what is the meaning of life", "settings", "w1");
    expect(a.isClarification).toBe(true);
    expect(a.answer).toBe(
      ["In the demo, Ask AI answers the suggested questions only. Try one of these:", ...AI_PROMPT_CHIPS.settings.map((c) => `- ${c.label}`)].join("\n"),
    );
    expect(a.meta.dataCategories).toEqual([]);
  });
});
