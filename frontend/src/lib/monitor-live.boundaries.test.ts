// Boundary + branch behaviour of the live Member Monitor.
//
// monitor-live.test.ts covers the monitor's rules on well-formed rows and
// heartbeats; this file pins down the edges:
//   • a day key that names no day, and a block that only grazes the day,
//   • rows the app breakdown can't place, carry no timed windows, or tie,
//   • heartbeats with an unknown status, a zone-less, unreadable or missing
//     time — and a member's weaker second device,
//   • blocks without a project, or whose project has no known interval.
import { describe, expect, it } from "vitest";
import { activityToSegment, dayActivityEndpoint, liveAppBreakdown, liveBlockIds, mergePresence, presenceStateOf, uploadPresence } from "./monitor-live";
import type { PresenceDto } from "./api-contract";
import type { Activity } from "./types";

const DAY = "2026-09-23";
const NOW = new Date(`${DAY}T12:00:00.000Z`);

function act(over: Partial<Activity> = {}): Activity {
  return {
    id: "a1",
    userId: "u1",
    projectId: "p1",
    startedAt: `${DAY}T09:00:00.000Z`,
    endedAt: `${DAY}T09:10:00.000Z`,
    description: "Tracked session",
    productivity: 80,
    mouseClicks: 12,
    keyboardHits: 340,
    activeWindows: [],
    runningPrograms: [],
    screen: { app: "Desktop", kind: "editor", accent: "#000" },
    hasWebcam: false,
    online: false,
    ...over,
  };
}

describe("days that name nothing, and blocks that graze a day", () => {
  it("a malformed day key sends no date range rather than an invalid one", () => {
    expect(dayActivityEndpoint("23/09/2026")).toBe("/api/app/activity?MaxResultCount=1000");
  });

  it("a block that spills only seconds into the day draws no segment", () => {
    const graze = act({ startedAt: "2026-09-22T23:55:00.000Z", endedAt: `${DAY}T00:00:20.000Z` });
    expect(activityToSegment(graze, DAY)).toBeNull();
  });
});

describe("liveAppBreakdown — rows it can't use, and ties", () => {
  it("skips rows it can't place, from another day, or without timed windows; apps with equal time are listed by name", () => {
    const rows = [
      act({ id: "bad", startedAt: "garbage" }),
      act({
        id: "elsewhere",
        startedAt: "2026-09-21T09:00:00.000Z",
        endedAt: "2026-09-21T09:10:00.000Z",
        activeWindows: [{ appName: "Slack", windowTitle: "general", seconds: 600 }],
      }),
      { ...act({ id: "no-windows" }), activeWindows: undefined } as unknown as Activity,
      act({
        id: "split",
        activeWindows: [
          { appName: "Slack", windowTitle: "general", seconds: 300 },
          { appName: "Figma", windowTitle: "Board", seconds: 300 },
        ],
      }),
    ];
    expect(liveAppBreakdown(rows, DAY).map((r) => [r.app, r.minutes])).toEqual([
      ["Figma", 5],
      ["Slack", 5],
    ]);
  });
});

describe("presence — heartbeats as an agent may send them", () => {
  it("an online device reporting a status this app doesn't know reads as offline, never as tracking", () => {
    expect(presenceStateOf({ status: "warp" as PresenceDto["status"], online: true, lastSeenAt: `${DAY}T11:59:30Z` }, NOW)).toBe("offline");
  });

  it("reads a zone-less heartbeat time as UTC, an unreadable or missing one as never seen, and keeps the stronger device", () => {
    const merged = mergePresence(
      [
        { userId: "u1", deviceId: "d1", status: "tracking", online: true, lastSeenAt: `${DAY}T11:59:30` },
        { userId: "u1", deviceId: "d2", status: "idle", online: true, lastSeenAt: `${DAY}T11:59:40Z` }, // newer, but only idle
        { userId: "u2", deviceId: "d3", status: "tracking", online: true, lastSeenAt: "yesterday" },
        { userId: "u3", deviceId: "d4", status: "tracking", online: true },
      ],
      NOW,
    );
    expect(merged.get("u1")).toMatchObject({ state: "tracking", lastSeenAt: `${DAY}T11:59:30.000Z` });
    expect(merged.get("u2")).toMatchObject({ state: "offline", lastSeenAt: null, since: null });
    expect(merged.get("u3")).toMatchObject({ state: "offline", lastSeenAt: null });
  });
});

describe("blocks without a project, or without a known interval", () => {
  it("uploadPresence reads a block without a project as no project, judged on the default window", () => {
    const presence = uploadPresence(new Map([["u1", act({ projectId: "", endedAt: `${DAY}T11:50:00.000Z` })]]), NOW, () => undefined);
    expect(presence.get("u1")).toMatchObject({ state: "tracking", projectId: null, source: "uploads" });
  });

  it("liveBlockIds reads a project whose interval isn't known as a 10-minute one (plus the upload grace)", () => {
    const tracking = new Map([["u1", { state: "tracking" as const }]]);
    const recent = [act({ id: "recent", endedAt: `${DAY}T11:46:00.000Z` })]; // 14 minutes ago
    const stale = [act({ id: "stale", endedAt: `${DAY}T11:44:00.000Z` })]; // 16 minutes ago
    expect([...liveBlockIds(recent, tracking, NOW, () => undefined)]).toEqual(["recent"]);
    expect(liveBlockIds(stale, tracking, NOW, () => undefined).size).toBe(0);
  });
});
