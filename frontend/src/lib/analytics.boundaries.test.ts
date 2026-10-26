// Boundary + branch behaviour of the analytics derivations.
//
// analytics.test.ts covers what each chart is built from; this file pins down
// the parts a well-formed fixture never reaches:
//   • payload fields in the shapes a server or an older agent may send them —
//     counts and offsets as text, blank or missing fields, numeric ids,
//   • a zone whose offset changes in the middle of a local hour,
//   • member-days whose bounds only some sources send,
//   • the order of ties (members, projects, apps, lanes): a ranking of the same
//     data never reshuffles between two renders,
//   • overlapping blocks of one member (two devices), devices told apart by
//     whatever they report, and agent versions that aren't plain numbers,
//   • the defaults the flow layout falls back to.
//
// Instants are written in UTC and read in a named zone, so every expectation
// holds in whatever time zone the suite runs in.
import { describe, expect, it } from "vitest";
import {
  OTHER,
  agentVersions,
  appStats,
  appUsageFromBlocks,
  blocksFromApi,
  dayStats,
  dayTimeline,
  devicesFromBlocks,
  hourPieces,
  layoutFlow,
  memberClock,
  memberProjectFlow,
  memberStats,
  parseDevices,
  projectDays,
  radarRows,
  toBlocks,
  withBlockBounds,
  zoneCalendar,
  type DeviceRow,
  type MemberStat,
} from "./analytics";
import type { MemberDayAggregate, ReportAggregates } from "./work-report";
import type { WindowInfo } from "./types";
import { buildActivity, buildWindowInfo } from "@/test/factories";

const at = (iso: string) => Date.parse(iso);
const UTC = zoneCalendar("UTC");
const DAY = "2026-09-28"; // a Monday

/** A GET /api/app/activity item of member u1 on project p1; `extra` overrides any field. */
function dto(id: unknown, startIso: string, minutes: number, extra: Record<string, unknown> = {}) {
  return {
    id,
    userId: "u1",
    projectId: "p1",
    startedAt: startIso,
    endedAt: new Date(at(startIso) + minutes * 60_000).toISOString(),
    productivity: 70,
    activeWindowsJson: JSON.stringify([{ AppName: "code.exe", WindowTitle: "main.rs", Seconds: minutes * 60 }]),
    ...extra,
  };
}

function aggregates(over: Partial<ReportAggregates>): ReportAggregates {
  return {
    totals: { minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: 0, clicks: 0 },
    daily: [],
    members: [],
    projects: [],
    memberDays: [],
    memberProjects: null,
    ...over,
  };
}

function memberDay(userId: string, day: string, minutes: number, bounds: Partial<Pick<MemberDayAggregate, "firstStart" | "lastEnd">> = {}): MemberDayAggregate {
  return { userId, name: null, day, minutes, sessions: minutes > 0 ? 1 : 0, idleMinutes: null, firstStart: null, lastEnd: null, ...bounds };
}

function memberStat(over: Partial<MemberStat> & Pick<MemberStat, "userId">): MemberStat {
  return {
    name: null,
    minutes: 60,
    activity: null,
    idleMinutes: null,
    sessions: 1,
    daysActive: 1,
    daily: [60],
    avgStretch: null,
    longestStretch: null,
    keys: 0,
    clicks: 0,
    inputPerHour: null,
    consistency: null,
    typicalStart: null,
    typicalEnd: null,
    localEstimated: false,
    ...over,
  };
}

/* ----------------------------- Blocks: how the fields arrive ----------------------------- */

describe("blocks — fields in every shape a server or an older agent sends them", () => {
  it("reads counts sent as numeric text; a blank, missing or null count is not measured (null), never 0 or NaN", () => {
    const [a, b, c] = blocksFromApi([
      dto("a", `${DAY}T09:00:00Z`, 10, { keyboardHits: "42", mouseClicks: "  " }),
      dto("b", `${DAY}T10:00:00Z`, 10),
      dto("c", `${DAY}T11:00:00Z`, 10, { keyboardHits: 0, mouseClicks: null }),
    ]);
    expect(a).toMatchObject({ keys: 42, clicks: null });
    expect(b).toMatchObject({ keys: null, clicks: null });
    // A measured zero stays zero.
    expect(c).toMatchObject({ keys: 0, clicks: null });
  });

  it("reads a UTC offset sent as text, and blank device fields as not reported", () => {
    const [b] = blocksFromApi([dto("a", `${DAY}T09:00:00Z`, 10, { utcOffsetMinutes: "330", deviceName: "   ", osVersion: "", agentVersion: " 0.3.1 " })]);
    expect(b).toMatchObject({ utcOffset: 330, deviceName: null, osVersion: null, agentVersion: "0.3.1" });
  });

  it("keeps a numeric id as text and drops a row whose id is neither text nor a number", () => {
    const blocks = blocksFromApi([dto(42, `${DAY}T09:00:00Z`, 10), dto({ value: "x" }, `${DAY}T10:00:00Z`, 10)]);
    expect(blocks.map((b) => b.id)).toEqual(["42"]);
  });

  it("still counts a row that names no member or project (it lands under an empty id, never dropped)", () => {
    const [b] = blocksFromApi([{ ...dto("orphan", `${DAY}T11:00:00Z`, 10), userId: undefined, projectId: null }]);
    expect(b).toMatchObject({ id: "orphan", userId: "", projectId: "", minutes: 10 });
  });

  it("demo rows: windows that aren't a list, a window without an app name, and a row without an id", () => {
    const noWindows = { ...buildActivity({ id: "w", startedAt: `${DAY}T09:00:00Z`, endedAt: `${DAY}T09:10:00Z` }), activeWindows: undefined };
    const unnamed = buildActivity({
      id: "n",
      startedAt: `${DAY}T10:00:00Z`,
      endedAt: `${DAY}T10:10:00Z`,
      activeWindows: [{ seconds: 300 } as unknown as WindowInfo, buildWindowInfo({ appName: "Slack", seconds: 120 })],
    });
    const anonymous = { ...buildActivity({ startedAt: `${DAY}T11:00:00Z`, endedAt: `${DAY}T11:10:00Z` }), id: undefined };
    const blocks = toBlocks([noWindows, unnamed, anonymous] as unknown as Parameters<typeof toBlocks>[0]);
    expect(blocks.map((b) => [b.id, b.apps])).toEqual([
      ["w", []],
      ["n", [{ name: "Slack", seconds: 120 }]],
      ["", [{ name: "Visual Studio Code", seconds: 300 }]],
    ]);
  });
});

/* ----------------------------- Hours ----------------------------- */

describe("hours — a zone whose offset changes inside a local hour", () => {
  it("ends the piece at the change (Caracas, 1 May 2016: UTC−04:30 → UTC−04:00 at 02:30), so no minute is counted twice or lost", () => {
    const caracas = zoneCalendar("America/Caracas");
    const pieces = hourPieces(at("2016-05-01T06:00:00Z"), at("2016-05-01T08:00:00Z"), caracas);
    expect(pieces.map((p) => [p.hour, p.minutes, p.from, p.to])).toEqual([
      [1, 30, 90, 120],
      [2, 30, 120, 150], // 02:00–02:30, then the clock jumps to 03:00
      [3, 60, 180, 240],
    ]);
    expect(pieces.reduce((s, p) => s + p.minutes, 0)).toBe(120);
  });
});

/* ----------------------------- Days & member-days ----------------------------- */

describe("days and member-days", () => {
  it("a member listed with no time on a day isn't counted among that day's members", () => {
    const stats = dayStats(
      aggregates({
        daily: [{ day: DAY, minutes: 60, sessions: 2, activity: 70, idleMinutes: 5, keys: 10, clicks: 4 }],
        memberDays: [memberDay("u1", DAY, 60), memberDay("u2", DAY, 0)],
      }),
      [DAY],
    );
    expect(stats[0]).toMatchObject({ minutes: 60, sessions: 2, members: 1 });
  });

  it("fills in only the bounds a source didn't send, from the earliest start and latest end of that member's day", () => {
    // Loaded oldest first: 08:00–08:30, 09:00–09:30, 13:00–13:45 — all member u1 on DAY.
    const blocks = blocksFromApi([dto("a", `${DAY}T09:00:00Z`, 30), dto("b", `${DAY}T08:00:00Z`, 30), dto("c", `${DAY}T13:00:00Z`, 45)]);
    const filled = withBlockBounds(
      [
        memberDay("u1", DAY, 105, { firstStart: at(`${DAY}T07:55:00Z`) }), // its first block wasn't loaded: the known start stays
        memberDay("u2", DAY, 30, { firstStart: at(`${DAY}T10:00:00Z`), lastEnd: at(`${DAY}T10:30:00Z`) }),
        memberDay("u3", "2026-09-29", 20), // no blocks to go on
      ],
      blocks,
      UTC,
    );
    expect(filled.map((d) => [d.userId, d.firstStart, d.lastEnd])).toEqual([
      ["u1", at(`${DAY}T07:55:00Z`), at(`${DAY}T13:45:00Z`)],
      ["u2", at(`${DAY}T10:00:00Z`), at(`${DAY}T10:30:00Z`)],
      ["u3", null, null],
    ]);
  });
});

/* ----------------------------- Members ----------------------------- */

describe("members", () => {
  it("ranks members with equal time by id, and one without per-day rows gets an empty sparkline", () => {
    const stats = memberStats(
      aggregates({
        members: [
          { userId: "u2", name: null, minutes: 60, sessions: 1, activity: 50, idleMinutes: null },
          { userId: "u1", name: null, minutes: 60, sessions: 1, activity: 70, idleMinutes: null },
        ],
        memberDays: [memberDay("u1", DAY, 60, { firstStart: at(`${DAY}T09:00:00Z`), lastEnd: at(`${DAY}T10:00:00Z`) })],
      }),
      [],
      [DAY, "2026-09-29"],
      memberClock([], UTC),
    );
    expect(stats.map((s) => s.userId)).toEqual(["u1", "u2"]);
    expect(stats[0]).toMatchObject({ daily: [60, 0], daysActive: 1, typicalStart: 540 });
    expect(stats[1]).toMatchObject({ daily: [0, 0], daysActive: 0, consistency: null, typicalStart: null, avgStretch: null, inputPerHour: null });
  });

  it("plots the focus and input of a member without loaded blocks at 0, without dividing by zero", () => {
    const rows = radarRows([memberStat({ userId: "u1", minutes: 120, activity: 80 })], 1);
    expect(Object.fromEntries(rows.map((r) => [r.dimension, r.u1]))).toEqual({
      Hours: 100,
      Activity: 80,
      "Days active": 100,
      Focus: 0,
      Input: 0,
      Consistency: 0,
    });
  });
});

/* ----------------------------- Projects & apps ----------------------------- */

describe("projects and apps", () => {
  it("leaves out of the daily project split the blocks that started outside the period", () => {
    const blocks = blocksFromApi([dto("in", `${DAY}T09:00:00Z`, 30), dto("out", "2026-09-30T09:00:00Z", 30)]);
    expect(projectDays(blocks, [DAY], ["p1"], UTC)).toEqual([{ day: DAY, label: "Mon 28", p1: 0.5, [OTHER]: 0 }]);
  });

  it("orders members and projects with equal hours by id, so the flow diagram never reshuffles", () => {
    const flow = memberProjectFlow(
      [
        { userId: "u2", projectId: "p2", minutes: 60 },
        { userId: "u1", projectId: "p1", minutes: 60 },
      ],
      (id) => id.toUpperCase(),
      (id) => id.toUpperCase(),
    );
    expect(flow.nodes.map((n) => n.id)).toEqual(["u1", "u2", "p1", "p2"]);
  });

  it("an app named nothing but '.exe' has no name and is left out; apps with equal time are listed by name", () => {
    const [block] = blocksFromApi([
      dto("a", `${DAY}T09:00:00Z`, 10, {
        activeWindowsJson: JSON.stringify([
          { AppName: ".exe", WindowTitle: "?", Seconds: 120 },
          { AppName: "slack.exe", WindowTitle: "general", Seconds: 240 },
          { AppName: "Figma", WindowTitle: "Board", Seconds: 240 },
        ]),
      }),
    ]);
    const usage = appUsageFromBlocks([block]);
    expect(usage.map((u) => [u.name, u.minutes])).toEqual([
      ["Slack", 4],
      ["Figma", 4],
    ]);
    expect(appStats(usage).map((s) => [s.app, s.share])).toEqual([
      ["Figma", 50],
      ["Slack", 50],
    ]);
  });
});

/* ----------------------------- Day timeline ----------------------------- */

describe("day timeline — overlapping blocks and ties", () => {
  it("a shorter block inside a longer one (a second device) never cuts the next gap short, and members starting together are ordered by id", () => {
    const blocks = blocksFromApi([
      dto("long", `${DAY}T09:00:00Z`, 60, { userId: "u2", endReason: "break" }),
      dto("inner", `${DAY}T09:10:00Z`, 20, { userId: "u2", endReason: "lock" }),
      dto("twin", `${DAY}T09:00:00Z`, 30, { userId: "u2" }),
      dto("after", `${DAY}T10:30:00Z`, 10, { userId: "u2" }),
      dto("other", `${DAY}T09:00:00Z`, 10),
    ]);
    const timeline = dayTimeline(blocks, DAY, UTC);
    expect(timeline.lanes.map((l) => l.userId)).toEqual(["u1", "u2"]);
    const lane = timeline.lanes[1];
    // Same start: the shorter block first.
    expect(lane.segments.map((s) => [s.start, s.end])).toEqual([
      [at(`${DAY}T09:00:00Z`), at(`${DAY}T09:30:00Z`)],
      [at(`${DAY}T09:00:00Z`), at(`${DAY}T10:00:00Z`)],
      [at(`${DAY}T09:10:00Z`), at(`${DAY}T09:30:00Z`)],
      [at(`${DAY}T10:30:00Z`), at(`${DAY}T10:40:00Z`)],
    ]);
    // The gap opens when the LONGEST block ends, and reads as that block's reason, not the inner one's.
    expect(lane.gaps).toEqual([{ start: at(`${DAY}T10:00:00Z`), end: at(`${DAY}T10:30:00Z`), reason: "break" }]);
  });
});

/* ----------------------------- Devices ----------------------------- */

describe("devices", () => {
  it("parseDevices skips entries that aren't rows", () => {
    const rows = parseDevices([null, 7, "device", { userId: "u1", trackedMinutes: 30, platform: "Linux" }]);
    expect(rows.map((r) => [r.userId, r.platform, r.minutes])).toEqual([["u1", "linux", 30]]);
  });

  it("devicesFromBlocks tells devices apart by id, else name, else platform, and keeps what a newer block left out", () => {
    const blocks = blocksFromApi([
      dto("1", `${DAY}T08:00:00Z`, 10, { deviceId: "D1", platform: "windows", agentVersion: "0.2.0" }),
      dto("2", `${DAY}T09:00:00Z`, 10, { deviceId: "D1" }), // D1's newest block reports nothing else
      dto("3", `${DAY}T10:00:00Z`, 10, { deviceName: "RINA-MAC", platform: "macos" }),
      dto("4", `${DAY}T11:00:00Z`, 10, { platform: "linux" }),
      dto("5", `${DAY}T12:00:00Z`, 10, { agentVersion: "0.1.0" }),
    ]);
    const rows = devicesFromBlocks(blocks);
    expect(rows.map((r) => [r.deviceId, r.deviceName, r.platform, r.agentVersion, r.minutes])).toEqual([
      ["D1", null, "windows", "0.2.0", 20],
      [null, "RINA-MAC", "macos", null, 10],
      [null, null, "linux", null, 10],
      [null, null, null, "0.1.0", 10],
    ]);
    expect(rows[0].lastActivityAt).toBe(at(`${DAY}T09:10:00Z`));
  });

  it("agentVersions lists the newest version first, reading it part by part, and devices without one last", () => {
    const row = (agentVersion: string | null): DeviceRow => ({
      userId: "u1",
      userName: null,
      deviceId: null,
      deviceName: null,
      platform: null,
      osVersion: null,
      agentVersion,
      lastActivityAt: null,
      minutes: 10,
    });
    const versions = agentVersions([row(null), row("0.2"), row("0.10.0"), row("0.2.1"), row("0.2.x"), row(null)]);
    expect(versions.map((v) => [v.version, v.devices])).toEqual([
      ["0.10.0", 1],
      ["0.2.1", 1],
      ["0.2.x", 1], // a part that isn't a number reads as 0, so it sits between 0.2.1 and 0.2
      ["0.2", 1],
      [null, 2],
    ]);
  });
});

/* ----------------------------- Layout ----------------------------- */

describe("flow layout defaults", () => {
  it("without options: 12 px nodes flush with both edges", () => {
    const flow = memberProjectFlow([{ userId: "u1", projectId: "p1", minutes: 60 }], (id) => id, (id) => id);
    const { nodes, links } = layoutFlow(flow, 300, 100);
    expect(nodes.map((n) => [n.node.id, n.x])).toEqual([
      ["u1", 0],
      ["p1", 288],
    ]);
    expect(links[0]).toMatchObject({ x0: 12, x1: 288 });
  });

  it("a flow without time still places its nodes, at the minimum height, and draws no bands", () => {
    const { nodes, links } = layoutFlow(
      {
        nodes: [
          { name: "A", kind: "member", id: "a" },
          { name: "P", kind: "project", id: "p" },
        ],
        links: [],
      },
      300,
      100,
    );
    expect(nodes.map((n) => n.height)).toEqual([2, 2]);
    expect(links).toEqual([]);
  });
});
