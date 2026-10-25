// Boundary + branch behaviour of the Work Report's derivations.
//
// work-report.test.ts covers how each section of the report is built; this
// file pins down what a tidy, chronological fixture never reaches:
//   • offsets and day keys that name nothing,
//   • rows given newest first (as the API pages them) or in no order at all —
//     every list still reads oldest first, ties in a stable order,
//   • rows the report can't use (no activity, no time, no id, no day),
//   • a member's day reported in two parts, and two spellings of one note,
//   • the full-days highlight and the timesheet CSV's idle column.
import { describe, expect, it } from "vitest";
import {
  aggregateTracked,
  buildDailyLog,
  buildHighlights,
  buildTimesheetMatrix,
  matrixCsv,
  membersOnDay,
  noteBreakdown,
  notesByDay,
  notesByMember,
  parseSummary,
  periodTitle,
  summarizeLog,
  trackedRowFromActivity,
  type MemberDayAggregate,
  type NoteBlock,
  type TrackedRow,
} from "./work-report";
import type { Activity } from "./types";
import { buildActivity } from "@/test/factories";

const NOW = new Date("2026-07-14T15:30:00.000Z");
const DAY = "2026-07-13"; // a Monday
const ms = (iso: string) => Date.parse(iso);
const utcDay = (t: number) => new Date(t).toISOString().slice(0, 10);

/* ----------------------------- Periods & days ----------------------------- */

describe("periods and days that name nothing", () => {
  it("an unreadable offset is the current period; a 7-day period further back reads as such", () => {
    expect(periodTitle("week", NOW, Number.NaN)).toBe("This week");
    expect(periodTitle("7d", NOW, -1)).toBe("7-day period");
  });

  it("buildDailyLog lists the days oldest first whatever order the period's days and the points come in", () => {
    const rows = buildDailyLog(["2026-07-15", "2026-07-13", "2026-07-14"], [{ date: "2026-07-12", minutes: 30 }]);
    expect(rows.map((r) => [r.date, r.minutes])).toEqual([
      ["2026-07-12", 30], // outside the period but tracked: kept, in its place
      ["2026-07-13", 0],
      ["2026-07-14", 0],
      ["2026-07-15", 0],
    ]);
  });

  it("buildTimesheetMatrix leaves off the axis a day of the period that isn't a readable date", () => {
    const matrix = buildTimesheetMatrix([DAY, "13/07/2026"], [{ userId: "u1", date: DAY, minutes: 60 }], (id) => id);
    expect(matrix.days).toEqual([DAY]);
    expect(matrix.rows.map((r) => r.cells)).toEqual([[60]]);
  });
});

/* ----------------------------- Aggregates ----------------------------- */

describe("aggregateTracked — rows in the order the API pages them", () => {
  const row = (userId: string, projectId: string, startIso: string, minutes: number): TrackedRow => ({
    userId,
    projectId,
    start: ms(startIso),
    end: ms(startIso) + minutes * 60_000,
    minutes,
    activity: null,
    idleMinutes: null,
    keys: 0,
    clicks: 0,
  });

  it("skips rows without time, and lists days, member-days and member × project pairs in a stable order", () => {
    const agg = aggregateTracked(
      [
        row("u1", "p2", "2026-07-15T09:00:00Z", 30), // newest first
        row("u1", "p1", "2026-07-14T09:00:00Z", 30),
        null as unknown as TrackedRow,
        row("u2", "p1", "2026-07-13T09:00:00Z", 0),
        row("u1", "p3", "2026-07-13T09:00:00Z", 15),
      ],
      utcDay,
    );
    expect(agg.totals).toMatchObject({ minutes: 75, sessions: 3 });
    expect(agg.daily.map((d) => [d.day, d.minutes])).toEqual([
      ["2026-07-13", 15],
      ["2026-07-14", 30],
      ["2026-07-15", 30],
    ]);
    expect(agg.memberDays.map((d) => [d.userId, d.day])).toEqual([
      ["u1", "2026-07-13"],
      ["u1", "2026-07-14"],
      ["u1", "2026-07-15"],
    ]);
    expect(agg.members.map((m) => m.userId)).toEqual(["u1"]); // u2's empty row makes no member
    // Equal time on two projects: by project id.
    expect(agg.memberProjects?.map((p) => [p.projectId, p.minutes])).toEqual([
      ["p1", 30],
      ["p2", 30],
      ["p3", 15],
    ]);
  });

  it("trackedRowFromActivity: no row for something that isn't an activity; one without member or project ids still counts", () => {
    expect(trackedRowFromActivity(null as unknown as Activity)).toBeNull();
    const row = trackedRowFromActivity({ ...buildActivity(), userId: undefined, projectId: undefined } as unknown as Activity);
    expect(row).toMatchObject({ userId: "", projectId: "", minutes: 10 });
  });

  it("parseSummary skips a project row without an id", () => {
    const summary = parseSummary({
      totalTrackedMinutes: 60,
      perUser: [],
      perProject: [
        { projectTitle: "Ghost", trackedMinutes: 60 },
        { projectId: "p1", trackedMinutes: 60 },
      ],
    });
    expect(summary.projects.map((p) => p.projectId)).toEqual(["p1"]);
  });
});

/* ----------------------------- Who worked on a day ----------------------------- */

describe("membersOnDay — a member's day reported in parts", () => {
  const part = (userId: string, minutes: number, firstStart: number | null, lastEnd: number | null, idleMinutes: number | null = null, day = DAY): MemberDayAggregate => ({
    userId,
    name: null,
    day,
    minutes,
    sessions: 1,
    idleMinutes,
    firstStart,
    lastEnd,
  });

  it("merges a member's parts (earliest start, latest finish) and lists members with equal time by name", () => {
    const names: Record<string, string> = { u1: "Bea", u2: "Ana", u3: "Cy" };
    const list = membersOnDay(
      [
        part("u1", 30, ms(`${DAY}T09:00:00Z`), ms(`${DAY}T09:30:00Z`), 5),
        part("u1", 30, ms(`${DAY}T08:00:00Z`), ms(`${DAY}T10:00:00Z`)),
        part("u2", 60, null, null),
        part("u3", 60, ms(`${DAY}T11:00:00Z`), ms(`${DAY}T12:00:00Z`)),
        part("u4", 99, null, null, null, "2026-07-14"), // another day
      ],
      DAY,
      (id) => names[id] ?? id,
    );
    expect(list.map((m) => [m.name, m.minutes, m.sessions, m.idleMinutes, m.firstStart, m.lastEnd])).toEqual([
      ["Ana", 60, 1, null, null, null],
      ["Bea", 60, 2, 5, ms(`${DAY}T08:00:00Z`), ms(`${DAY}T10:00:00Z`)],
      ["Cy", 60, 1, null, ms(`${DAY}T11:00:00Z`), ms(`${DAY}T12:00:00Z`)],
    ]);
  });
});

/* ----------------------------- What was worked on ----------------------------- */

describe("notes — spellings, ties and blocks that can't be placed", () => {
  it("two spellings with equal time: the one that sorts first is shown; members with equal time by id; no-note blocks last", () => {
    const rows = noteBreakdown([
      { userId: "u2", day: DAY, minutes: 30, note: "fix Login" },
      { userId: "u1", day: DAY, minutes: 30, note: "Fix login" },
      null as unknown as NoteBlock,
      { userId: "u1", day: DAY, minutes: 30, note: null },
    ]);
    expect(rows.map((r) => [r.note, r.minutes, r.members, r.share])).toEqual([
      ["Fix login", 60, ["u1", "u2"], 67],
      [null, 30, ["u1"], 33],
    ]);
  });

  it("notesByDay lists days oldest first whatever order the blocks come in; blocks without a day or member are left out", () => {
    const blocks: NoteBlock[] = [
      { userId: "u2", day: "2026-07-15", minutes: 30, note: "Review" },
      { userId: "u1", day: "2026-07-14", minutes: 30, note: "Review" },
      { userId: "", day: "", minutes: 45, note: "Lost" },
      null as unknown as NoteBlock,
    ];
    expect(notesByDay(blocks).map((s) => [s.id, s.minutes])).toEqual([
      ["2026-07-14", 30],
      ["2026-07-15", 30],
    ]);
    // Equal time: by member id.
    expect(notesByMember(blocks).map((s) => [s.id, s.minutes])).toEqual([
      ["u1", 30],
      ["u2", 30],
    ]);
  });
});

/* ----------------------------- Highlights & CSV ----------------------------- */

describe("highlights and the timesheet CSV", () => {
  const fullDays = (minutes: readonly number[]) => {
    const days = minutes.map((_, i) => `2026-07-1${3 + i}`);
    const rows = buildDailyLog(days, days.map((date, i) => ({ date, minutes: minutes[i] })));
    const highlights = buildHighlights({
      summary: summarizeLog(rows),
      projects: [],
      matrix: buildTimesheetMatrix(days, [], (id) => id),
      team: false,
      weekendDays: 0,
      fmt: (m) => `${m}m`,
    });
    return highlights.find((h) => h.kind === "fullDays");
  };

  it("for one person, says how many of the worked days reached 8 hours", () => {
    expect(fullDays([500, 120])).toEqual({ kind: "fullDays", label: "Full days (8h+)", value: "1 of 2", detail: "worked days reached 8 hours" });
    expect(fullDays([540])).toMatchObject({ value: "1 of 1", detail: "worked day reached 8 hours" });
  });

  it("matrixCsv leaves the idle cell blank for a member whose idle time is unknown", () => {
    const matrix = buildTimesheetMatrix(
      [DAY],
      [
        { userId: "u1", date: DAY, minutes: 60, idleMinutes: 6 },
        { userId: "u2", date: DAY, minutes: 30 },
      ],
      (id) => id,
    );
    const csv = matrixCsv(matrix);
    expect(csv.headers).toEqual(["Member", `${DAY} (Mon)`, "Total (hours)", "Idle (hours)"]);
    expect(csv.rows).toEqual([
      ["u1", 1, 1, 0.1],
      ["u2", 0.5, 0.5, ""],
      ["Total", 1.5, 1.5, 0.1],
    ]);
  });
});
