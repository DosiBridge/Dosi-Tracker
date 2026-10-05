// The demo dataset shows what the v2 agents report (contract §5): the device a
// block came from, its zone, idle time, why it ended and the member's task note.
// These are DERIVED from each generated session, so they must agree with the
// session itself (idle time with its Activity %, the zone's offset with its
// start) and they never disturb the seeded values the rest of the demo relies on.
import { beforeEach, describe, expect, it } from "vitest";
import { DAY_MS, DEMO_AGENT_VERSION, NOW, TODAY, activities as primaryActivities, demoAgentFields, demoDevicesFor, users } from "./mock-data";
import { datasetFor } from "./tenant-data";
import { durationMinutes, idleMinutesOf, isActivityMeasured } from "./metrics";
import { END_REASON_LABELS } from "./api-contract";
import { utcOffsetMinutes } from "./time-zone";
import { resetPrototypeState } from "@/test/harness";
import type { Activity } from "./types";

beforeEach(() => resetPrototypeState());

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const tenants = ["w1", "w2", "w3"] as const;

function rowsOf(workspaceId: string): Activity[] {
  return datasetFor(workspaceId).activities;
}

describe("demo blocks carry the v2 agent fields", () => {
  it.each(tenants)("every %s block names its device, zone, idle time, end reason and task note", (ws) => {
    const rows = rowsOf(ws);
    expect(rows.length).toBeGreaterThan(0);
    for (const a of rows) {
      expect(a.deviceId, a.id).toMatch(UUID);
      expect(a.deviceName, a.id).toBeTruthy();
      expect(["windows", "macos"], a.id).toContain(a.platform);
      expect(a.osVersion, a.id).toMatch(a.platform === "windows" ? /^Windows 1\d / : /^macOS 1\d/);
      expect(a.agentVersion, a.id).toMatch(/^\d+\.\d+\.\d+$/);
      expect(Object.keys(END_REASON_LABELS), a.id).toContain(a.endReason);
      expect(a.note, a.id).toBe(a.description);
      expect(isActivityMeasured(a), a.id).toBe(true);
    }
  });

  it.each(tenants)("%s idle time is the minutes without input that each block's Activity % implies", (ws) => {
    for (const a of rowsOf(ws)) {
      const minutes = Math.round(durationMinutes(a));
      const idle = idleMinutesOf(a);
      expect(idle, a.id).not.toBeNull();
      expect(Number.isInteger(idle), a.id).toBe(true);
      expect((idle ?? 0) + Math.round((minutes * a.productivity) / 100), a.id).toBe(minutes);
    }
  });

  it("each block's offset is its device zone's offset when it started", () => {
    for (const a of primaryActivities) {
      expect(a.utcOffsetMinutes).toBe(utcOffsetMinutes(new Date(a.startedAt), a.timeZone ?? "UTC"));
    }
    const dhaka = primaryActivities.find((a) => a.timeZone === "Asia/Dhaka");
    expect(dhaka?.utcOffsetMinutes).toBe(360);
  });

  it("task notes read like real ones ('Feature – task'), e.g. a PR review on the checkout flow", () => {
    const notes = new Set(tenants.flatMap((ws) => rowsOf(ws).map((a) => a.note)));
    expect(notes.has("Checkout flow – PR review")).toBe(true);
    for (const note of notes) expect(note).toMatch(/^\S.* – \S.*$/);
  });
});

describe("demo devices", () => {
  it("the primary team runs a mix of Windows and macOS, with one agent a release behind", () => {
    const platforms = new Set(primaryActivities.map((a) => a.platform));
    expect(platforms).toEqual(new Set(["windows", "macos"]));
    const versions = new Set(primaryActivities.map((a) => a.agentVersion));
    expect(versions.has(DEMO_AGENT_VERSION)).toBe(true);
    expect([...versions].some((v) => v !== DEMO_AGENT_VERSION)).toBe(true);
  });

  it("a member with two computers tracks on both, switching by day", () => {
    const tanvir = primaryActivities.filter((a) => a.userId === "u2");
    expect(new Set(tanvir.map((a) => a.deviceId)).size).toBe(2);
    expect(new Set(tanvir.map((a) => a.platform))).toEqual(new Set(["windows", "macos"]));
  });

  it("device identity is stable: the same member always gets the same machines and ids", () => {
    const user = { id: "w9-u1", name: "Lena Park" };
    expect(demoDevicesFor(user)).toEqual(demoDevicesFor({ ...user }));
    const ids = users.flatMap((u) => demoDevicesFor(u).map((d) => d.deviceId));
    expect(new Set(ids).size).toBe(ids.length);
    expect(demoDevicesFor(user)[0].deviceName).toMatch(/^(Lena's MacBook Pro|LENA-PC)$/);
  });

  it("demoAgentFields derives everything from the session and its member", () => {
    const user = users[0];
    const session = primaryActivities.find((a) => a.userId === user.id) as Activity;
    const fields = demoAgentFields(user, session, 0);
    expect(fields).toMatchObject({ timeZone: user.timezone, note: session.description, deviceId: demoDevicesFor(user)[0].deviceId });
  });
});

describe("demo end reasons", () => {
  /** A member's blocks of one day, oldest first. */
  function days(rows: Activity[]): Activity[][] {
    const byDay = new Map<string, Activity[]>();
    for (const a of rows) {
      const key = `${a.userId}|${a.endedAt.slice(0, 10)}`;
      byDay.set(key, [...(byDay.get(key) ?? []), a]);
    }
    return [...byDay.values()].map((list) => list.sort((x, y) => Date.parse(x.startedAt) - Date.parse(y.startedAt)));
  }

  it.each(tenants)("%s: a past day ends the day, today's last block ends at its interval, a project switch says so", (ws) => {
    const today = new Date(NOW.getTime()).toISOString().slice(0, 10);
    expect(today).toBe(TODAY);
    for (const day of days(rowsOf(ws))) {
      const last = day[day.length - 1];
      if (last.endedAt.startsWith(TODAY)) expect(last.endReason).toBe("interval");
      else expect(["quit", "shutdown", "signout", "sleep"]).toContain(last.endReason);
      day.slice(0, -1).forEach((a, i) => {
        const next = day[i + 1];
        if (next.projectId !== a.projectId) expect(a.endReason).toBe("project");
        else expect(["break", "pause", "idle", "lock", "task"]).toContain(a.endReason);
      });
    }
  });

  it("the demo spans several days of history (so past-day end reasons appear)", () => {
    const oldest = Math.min(...primaryActivities.map((a) => Date.parse(a.startedAt)));
    expect(NOW.getTime() - oldest).toBeGreaterThan(2 * DAY_MS);
  });
});
