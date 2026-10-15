// The live Member Monitor draws ONLY real rows. These tests pin the mapping
// from a backend activity to a day segment (in the reporting zone), the
// per-member grouping, the app breakdown, presence from heartbeats (and the
// fallbacks an older server or the demo needs), the roster a manager sees,
// and the axis that must fit a real day.
import { describe, it, expect } from "vitest";
import {
  ACTIVE_WITHIN_MIN,
  AWAY_STATES,
  DAY_ACTIVITY_LIMIT,
  DEFAULT_DAY_WINDOW,
  activityToSegment,
  dayActivityEndpoint,
  dayBounds,
  demoPresence,
  lastSeenByUser,
  latestBlockByUser,
  liveAppBreakdown,
  liveBlockIds,
  liveRoster,
  liveStatus,
  mergePresence,
  presenceGroup,
  presenceStateOf,
  presenceUserStatus,
  presenceWindowMinutes,
  segmentsByUser,
  timelineWindow,
  unknownMember,
  uploadPresence,
} from "./monitor-live";
import { NO_SESSION_DETAILS } from "./activity-report";
import { appColor } from "./apps";
import type { DaySegment } from "./monitor-data";
import type { PresenceDto } from "./api-contract";
import type { Activity, User } from "./types";

const DAY = "2026-09-23";

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

function member(id: string, over: Partial<User> = {}): User {
  return {
    id,
    name: `Name ${id}`,
    email: `${id}@example.com`,
    role: "worker",
    designation: "Member",
    status: "active",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: "",
    ...over,
  };
}

function seg(startMin: number, endMin: number): DaySegment {
  return {
    id: `s${startMin}`,
    type: "work",
    startMin,
    endMin,
    minutes: endMin - startMin,
    app: "App",
    windowTitle: "",
    projectId: null,
    productivity: 50,
    mouseClicks: 0,
    keyboardHits: 0,
    screen: { app: "App", kind: "docs", accent: "#000" },
  };
}

describe("dayActivityEndpoint", () => {
  it("asks for one whole UTC day, a page at the backend page-size ceiling", () => {
    expect(DAY_ACTIVITY_LIMIT).toBe(1000);
    expect(dayActivityEndpoint(DAY)).toBe(
      "/api/app/activity?From=2026-09-23T00%3A00%3A00.000Z&To=2026-09-24T00%3A00%3A00.000Z&MaxResultCount=1000",
    );
  });

  it("narrows to one member when a user id is given (encoded)", () => {
    expect(dayActivityEndpoint(DAY, "a b")).toBe(
      "/api/app/activity?UserId=a+b&From=2026-09-23T00%3A00%3A00.000Z&To=2026-09-24T00%3A00%3A00.000Z&MaxResultCount=1000",
    );
  });

  it("asks for the calendar day of the reporting zone", () => {
    // A Dhaka (UTC+6) day runs from 18:00 UTC the evening before.
    expect(dayActivityEndpoint(DAY, undefined, "Asia/Dhaka")).toBe(
      "/api/app/activity?From=2026-09-22T18%3A00%3A00.000Z&To=2026-09-23T18%3A00%3A00.000Z&MaxResultCount=1000",
    );
    expect(dayBounds(DAY, "America/New_York")).toEqual({
      from: new Date("2026-09-23T04:00:00.000Z"),
      to: new Date("2026-09-24T04:00:00.000Z"),
    });
    expect(dayBounds("not-a-day")).toBeNull();
  });
});

describe("activityToSegment", () => {
  it("maps a block to minutes of its day with the real figures, named canonically", () => {
    const s = activityToSegment(
      act({
        activeWindows: [{ appName: "Code", windowTitle: "main.rs", seconds: 600 }],
      }),
      DAY,
    );
    expect(s).toEqual({
      id: "a1",
      type: "work",
      startMin: 540,
      endMin: 550,
      minutes: 10,
      app: "Visual Studio Code",
      windowTitle: "main.rs",
      projectId: "p1",
      productivity: 80,
      mouseClicks: 12,
      keyboardHits: 340,
      screen: { app: "Visual Studio Code", kind: "docs", accent: appColor("Visual Studio Code") },
    });
    expect(s?.tabs).toBeUndefined();
  });

  it("names the app with the most focused time summed over its windows (first one on a tie)", () => {
    const windows = [
      { appName: "Slack", windowTitle: "#general", seconds: 60 },
      { appName: "Code", windowTitle: "api.rs", seconds: 400 },
      { appName: "Figma", windowTitle: "Board", seconds: 400 },
    ];
    const s = activityToSegment(act({ activeWindows: windows }), DAY);
    expect(s?.app).toBe("Visual Studio Code");
    expect(s?.windowTitle).toBe("api.rs");
    // Two Chrome windows together outweigh one longer editor window.
    const split = activityToSegment(
      act({
        activeWindows: [
          { appName: "Code.exe", windowTitle: "a.ts", seconds: 300 },
          { appName: "chrome.exe", windowTitle: "Docs", seconds: 200 },
          { appName: "Google Chrome", windowTitle: "Mail", seconds: 200 },
        ],
      }),
      DAY,
    );
    expect(split?.app).toBe("Google Chrome");
  });

  it("falls back to honest labels when the agent sent no windows", () => {
    const s = activityToSegment(act({ description: "Fixing bugs" }), DAY);
    expect(s?.app).toBe("Tracked activity");
    expect(s?.windowTitle).toBe("Fixing bugs");
    expect(activityToSegment(act({ description: "" }), DAY)?.windowTitle).toBe("Activity block");
    const untitled = activityToSegment(
      act({ activeWindows: [{ appName: "Code", windowTitle: "", seconds: 0 }], description: "Notes" }),
      DAY,
    );
    expect(untitled?.windowTitle).toBe("Notes");
  });

  it("tolerates a row whose window list is missing", () => {
    const s = activityToSegment(act({ activeWindows: undefined as unknown as Activity["activeWindows"] }), DAY);
    expect(s?.app).toBe("Tracked activity");
  });

  it("reports a missing project as none", () => {
    expect(activityToSegment(act({ projectId: "" }), DAY)?.projectId).toBeNull();
  });

  it("clamps productivity into 0–100 and treats a non-number as 0", () => {
    expect(activityToSegment(act({ productivity: 140 }), DAY)?.productivity).toBe(100);
    expect(activityToSegment(act({ productivity: -5 }), DAY)?.productivity).toBe(0);
    expect(activityToSegment(act({ productivity: 72.6 }), DAY)?.productivity).toBe(73);
    expect(activityToSegment(act({ productivity: Number.NaN }), DAY)?.productivity).toBe(0);
  });

  it("clips a block that crosses midnight to the day being viewed", () => {
    const overnight = act({ startedAt: "2026-09-22T23:50:00.000Z", endedAt: `${DAY}T00:20:00.000Z` });
    expect(activityToSegment(overnight, DAY)).toMatchObject({ startMin: 0, endMin: 20, minutes: 20 });
    expect(activityToSegment(overnight, "2026-09-22")).toMatchObject({ startMin: 1430, endMin: 1440, minutes: 10 });
  });

  it("places a block on the wall clock of the reporting zone", () => {
    // 09:00–09:10 UTC is 15:00–15:10 in Dhaka and 14:45–14:55 in Kathmandu (UTC+5:45).
    expect(activityToSegment(act(), DAY, "Asia/Dhaka")).toMatchObject({ startMin: 900, endMin: 910, minutes: 10 });
    expect(activityToSegment(act(), DAY, "Asia/Kathmandu")).toMatchObject({ startMin: 885, endMin: 895 });
    // 20:00 UTC on the 23rd is already the 24th in Dhaka.
    const evening = act({ startedAt: `${DAY}T20:00:00.000Z`, endedAt: `${DAY}T20:10:00.000Z` });
    expect(activityToSegment(evening, DAY, "Asia/Dhaka")).toBeNull();
    expect(activityToSegment(evening, "2026-09-24", "Asia/Dhaka")).toMatchObject({ startMin: 120, endMin: 130 });
  });

  it("counts a block's real length on the nights the clock changes", () => {
    // New York falls back at 2026-11-01 02:00 EDT (06:00 UTC): 01:50 EDT → 01:20 EST took 30 minutes.
    const fallBack = act({ startedAt: "2026-11-01T05:50:00.000Z", endedAt: "2026-11-01T06:20:00.000Z" });
    expect(activityToSegment(fallBack, "2026-11-01", "America/New_York")).toMatchObject({ startMin: 110, endMin: 140, minutes: 30 });
    // …and springs forward at 2026-03-08 02:00 EST (07:00 UTC): 01:50 EST → 03:10 EDT took 20 minutes.
    const springForward = act({ startedAt: "2026-03-08T06:50:00.000Z", endedAt: "2026-03-08T07:10:00.000Z" });
    expect(activityToSegment(springForward, "2026-03-08", "America/New_York")).toMatchObject({ startMin: 110, endMin: 190, minutes: 20 });
  });

  it("drops rows outside the day, zero-length rows and unusable times instead of rendering NaN", () => {
    expect(activityToSegment(act(), "2026-09-21")).toBeNull();
    expect(activityToSegment(act({ endedAt: `${DAY}T09:00:00.000Z` }), DAY)).toBeNull();
    expect(activityToSegment(act({ endedAt: `${DAY}T08:00:00.000Z` }), DAY)).toBeNull();
    expect(activityToSegment(act({ startedAt: "not a date" }), DAY)).toBeNull();
    expect(activityToSegment(act({ endedAt: "" }), DAY)).toBeNull();
    expect(activityToSegment(act(), "bad-day")).toBeNull();
  });

  it("carries what newer agents report: measured or not, the note, the end reason and its share of idle time", () => {
    const details = { ...NO_SESSION_DETAILS, measured: false, note: "Release notes", endReason: "idle" as const, idleSeconds: 240 };
    expect(activityToSegment(act(), DAY, "UTC", details)).toMatchObject({
      measured: false,
      note: "Release notes",
      endReason: "idle",
      idleMinutes: 4,
    });
    // Clipped to half the block at midnight: half of the idle time.
    const overnight = act({ startedAt: "2026-09-22T23:55:00.000Z", endedAt: `${DAY}T00:05:00.000Z` });
    expect(activityToSegment(overnight, DAY, "UTC", { ...details, idleSeconds: 120 })?.idleMinutes).toBe(1);
    expect(activityToSegment(act(), DAY, "UTC", NO_SESSION_DETAILS)?.idleMinutes).toBeUndefined();
  });
});

describe("segmentsByUser", () => {
  it("groups each member's blocks for the day, in time order, skipping unusable rows", () => {
    const rows = [
      act({ id: "late", startedAt: `${DAY}T15:00:00.000Z`, endedAt: `${DAY}T15:10:00.000Z` }),
      act({ id: "early", startedAt: `${DAY}T08:00:00.000Z`, endedAt: `${DAY}T08:10:00.000Z` }),
      act({ id: "other", userId: "u2" }),
      act({ id: "broken", userId: "u3", endedAt: "garbage" }),
    ];
    const byUser = segmentsByUser(rows, DAY);
    expect(byUser.get("u1")?.map((s) => s.id)).toEqual(["early", "late"]);
    expect(byUser.get("u2")?.map((s) => s.id)).toEqual(["other"]);
    expect(byUser.has("u3")).toBe(false);
    expect(segmentsByUser([], DAY).size).toBe(0);
  });

  it("looks up each row's details", () => {
    const byUser = segmentsByUser([act()], DAY, "UTC", () => ({ ...NO_SESSION_DETAILS, note: "Docs" }));
    expect(byUser.get("u1")?.[0].note).toBe("Docs");
  });
});

describe("liveAppBreakdown — apps from the focused seconds each window reported", () => {
  it("credits each app with its own time, not the whole block to its top app", () => {
    const rows = [
      act({
        id: "b1",
        activeWindows: [
          { appName: "Code.exe", windowTitle: "a.ts", seconds: 360 },
          { appName: "slack.exe", windowTitle: "#dev", seconds: 240 },
        ],
      }),
      act({
        id: "b2",
        startedAt: `${DAY}T09:10:00.000Z`,
        endedAt: `${DAY}T09:20:00.000Z`,
        activeWindows: [
          { appName: "Code", windowTitle: "b.ts", seconds: 360 },
          { appName: "Slack", windowTitle: "#dev", seconds: 240 },
        ],
      }),
    ];
    expect(liveAppBreakdown(rows, DAY).map((r) => [r.app, r.minutes])).toEqual([
      ["Visual Studio Code", 12],
      ["Slack", 8],
    ]);
    expect(liveAppBreakdown(rows, DAY)[0]).toMatchObject({ accent: appColor("Visual Studio Code"), category: "productive" });
  });

  it("never credits more than the block lasted, and only the day's share of a block across midnight", () => {
    const over = act({ activeWindows: [{ appName: "Figma", windowTitle: "", seconds: 900 }, { appName: "Zoom", windowTitle: "", seconds: 300 }] });
    expect(liveAppBreakdown([over], DAY).map((r) => [r.app, r.minutes])).toEqual([
      ["Figma", 8],
      ["Zoom", 3],
    ]);
    const overnight = act({
      startedAt: "2026-09-22T23:50:00.000Z",
      endedAt: `${DAY}T00:10:00.000Z`,
      activeWindows: [{ appName: "Rider", windowTitle: "", seconds: 1200 }],
    });
    expect(liveAppBreakdown([overnight], DAY)).toMatchObject([{ app: "Rider", minutes: 10 }]);
  });

  it("counts a legacy row's one credited window as the whole block, and skips blocks with nothing reported", () => {
    const legacy = act({ activeWindows: [{ appName: "Rider", windowTitle: "Worker.cs", seconds: 600, credited: true }] });
    const silent = act({ id: "silent", activeWindows: [{ appName: "Rider", windowTitle: "", seconds: 0 }] });
    expect(liveAppBreakdown([legacy, silent], DAY)).toMatchObject([{ app: "Rider", minutes: 10 }]);
    expect(liveAppBreakdown([legacy], "nope")).toEqual([]);
  });
});

describe("lastSeenByUser / latestBlockByUser", () => {
  it("keeps each member's latest block end and ignores unusable ends", () => {
    const rows = [
      act({ userId: "u1", endedAt: "garbage" }),
      act({ userId: "u1", endedAt: `${DAY}T12:00:00.000Z` }),
      act({ userId: "u1", endedAt: `${DAY}T09:00:00.000Z` }),
      act({ userId: "u2", endedAt: `${DAY}T10:00:00.000Z` }),
    ];
    const seen = lastSeenByUser(rows);
    expect(seen.get("u1")).toBe(Date.parse(`${DAY}T12:00:00.000Z`));
    expect(seen.get("u2")).toBe(Date.parse(`${DAY}T10:00:00.000Z`));
    expect(lastSeenByUser([act({ endedAt: "garbage" })]).size).toBe(0);
  });

  it("keeps each member's latest block itself", () => {
    const latest = latestBlockByUser([
      act({ id: "old", endedAt: `${DAY}T08:00:00.000Z` }),
      act({ id: "new", endedAt: `${DAY}T12:00:00.000Z` }),
      act({ id: "bad", endedAt: "garbage" }),
    ]);
    expect(latest.get("u1")?.id).toBe("new");
  });
});

describe("liveStatus and the presence window", () => {
  const now = Date.parse(`${DAY}T12:00:00.000Z`);

  it("is active only while the latest block ended within the window", () => {
    expect(ACTIVE_WITHIN_MIN).toBe(15);
    expect(liveStatus(now - 15 * 60_000, now)).toBe("active");
    expect(liveStatus(now - 15 * 60_000 - 1, now)).toBe("offline");
    expect(liveStatus(now, now)).toBe("active");
    expect(liveStatus(now - 50 * 60_000, now, 60)).toBe("active");
  });

  it("is offline for a member who has never reported", () => {
    expect(liveStatus(undefined, now)).toBe("offline");
  });

  it("widens to two project intervals for long-interval projects", () => {
    expect(presenceWindowMinutes(10)).toBe(20);
    expect(presenceWindowMinutes(5)).toBe(15);
    expect(presenceWindowMinutes(30)).toBe(60);
    expect(presenceWindowMinutes(undefined)).toBe(15);
  });
});

describe("unknownMember", () => {
  it("labels a row owner the roster did not include without inventing a person", () => {
    expect(unknownMember("0123456789abcdef")).toEqual({
      id: "0123456789abcdef",
      name: "Member 01234567",
      email: "",
      role: "worker",
      designation: "Not on the loaded roster",
      status: "offline",
      timezone: "UTC",
      trackedToday: 0,
      productivity: 0,
      joinedAt: "",
    });
  });
});

describe("liveRoster", () => {
  const now = Date.parse(`${DAY}T12:00:00.000Z`);

  it("lists the real roster (no clients, no host) plus unknown row owners, with presence from rows", () => {
    const members = [
      member("owner", { role: "owner", status: "offline" }),
      member("w1"),
      member("client", { role: "client" }),
      member("host", { role: "host" }),
    ];
    const seen = new Map([["owner", now - 60_000]]);
    const roster = liveRoster(members, ["w1", "ghost", "ghost"], seen, now);
    expect(roster.map((u) => u.id)).toEqual(["owner", "w1", "ghost"]);
    expect(roster.map((u) => u.status)).toEqual(["active", "offline", "offline"]);
    expect(roster[2].name).toBe("Member ghost");
    // The input roster is not mutated.
    expect(members[0].status).toBe("offline");
  });

  it("takes each status from presence when it is given", () => {
    const roster = liveRoster([member("a"), member("b")], [], new Map(), now, (id) => (id === "a" ? "idle" : "active"));
    expect(roster.map((u) => u.status)).toEqual(["idle", "active"]);
  });

  it("is empty when there is no one to monitor", () => {
    expect(liveRoster([], [], new Map(), now)).toEqual([]);
  });
});

describe("timelineWindow", () => {
  it("falls back to the working day when there are no segments", () => {
    expect(DEFAULT_DAY_WINDOW).toEqual({ start: 480, end: 1200 });
    expect(timelineWindow([])).toEqual({ start: 480, end: 1200 });
  });

  it("focuses on actual activity range with one-hour padding, snapped to whole hours", () => {
    // 08:00–20:00 work: padded to 07:00–21:00
    expect(timelineWindow([seg(480, 1200), seg(600, 700)])).toEqual({ start: 420, end: 1260 });
    // Early-morning work at 03:05–04:10: padded to 02:00–06:00
    expect(timelineWindow([seg(185, 250)])).toEqual({ start: 120, end: 360 });
    // Late-night work at 20:50–21:40: padded to 19:00–22:00
    expect(timelineWindow([seg(1250, 1300)])).toEqual({ start: 1140, end: 1380 });
    // Spanning early and late: 06:00–20:10, padded to 05:00–21:00
    expect(timelineWindow([seg(360, 400), seg(1201, 1210)])).toEqual({ start: 300, end: 1320 });
  });

  it("never runs past midnight on either side", () => {
    // Near-midnight activity: start clamped to 0, end clamped to 1440
    expect(timelineWindow([seg(0, 30), seg(1430, 1440)])).toEqual({ start: 0, end: 1440 });
    expect(timelineWindow([seg(-30, 10), seg(1400, 1500)])).toEqual({ start: 0, end: 1440 });
  });
});

/* ----------------------------- Presence ----------------------------- */

const NOW = new Date(`${DAY}T12:00:00.000Z`);

function device(over: Partial<PresenceDto> = {}): PresenceDto {
  return {
    userId: "u1",
    userName: "ana",
    deviceId: "d1",
    status: "tracking",
    lastSeenAt: `${DAY}T11:59:30Z`,
    online: true,
    ...over,
  };
}

describe("presence states (contract §3)", () => {
  it("maps each agent status, and says offline once the heartbeat is too old or the agent stopped", () => {
    expect(presenceStateOf(device(), NOW)).toBe("tracking");
    for (const status of ["paused", "break", "idle", "locked", "choosing"] as const) {
      expect(presenceStateOf(device({ status }), NOW)).toBe(status);
    }
    expect(presenceStateOf(device({ status: "signedout" }), NOW)).toBe("offline");
    expect(presenceStateOf(device({ status: "stopped" }), NOW)).toBe("offline");
    expect(presenceStateOf(device({ online: false }), NOW)).toBe("offline");
    // Aged locally between refreshes: 151 s after the last heartbeat it's offline.
    expect(presenceStateOf(device({ lastSeenAt: `${DAY}T11:57:29Z` }), NOW)).toBe("offline");
    // A zone-less UTC time is read as UTC.
    expect(presenceStateOf(device({ lastSeenAt: `${DAY}T11:59:30` }), NOW)).toBe("tracking");
  });

  it("groups the states for the filter and the avatar dot", () => {
    expect(AWAY_STATES).toEqual(["idle", "paused", "break", "locked", "choosing"]);
    expect(presenceGroup("tracking")).toBe("tracking");
    expect(presenceGroup("locked")).toBe("away");
    expect(presenceGroup("offline")).toBe("offline");
    expect(presenceUserStatus("tracking")).toBe("active");
    expect(presenceUserStatus("break")).toBe("idle");
    expect(presenceUserStatus("offline")).toBe("offline");
  });
});

describe("mergePresence — one presence per member, over their devices", () => {
  it("prefers the device that is tracking, and keeps what it reports", () => {
    const merged = mergePresence(
      [
        device({ deviceId: "laptop", status: "idle", lastSeenAt: `${DAY}T11:59:50Z` }),
        device({
          deviceId: "desktop",
          deviceName: "DESKTOP-7Q2",
          platform: "windows",
          agentVersion: "0.2.0",
          currentApp: "Code.exe",
          projectId: "p1",
          projectTitle: "Apollo",
          statusSince: `${DAY}T10:05:00Z`,
        }),
      ],
      NOW,
    );
    expect(merged.get("u1")).toEqual({
      userId: "u1",
      state: "tracking",
      since: `${DAY}T10:05:00.000Z`,
      lastSeenAt: `${DAY}T11:59:30.000Z`,
      currentApp: "Visual Studio Code",
      projectId: "p1",
      projectTitle: "Apollo",
      deviceName: "DESKTOP-7Q2",
      platform: "windows",
      agentVersion: "0.2.0",
      source: "heartbeat",
    });
  });

  it("shows an online-but-away device before an offline one, and the latest heartbeat when all are offline", () => {
    const away = mergePresence([device({ online: false, lastSeenAt: `${DAY}T08:00:00Z` }), device({ deviceId: "d2", status: "break" })], NOW);
    expect(away.get("u1")?.state).toBe("break");
    const off = mergePresence(
      [
        device({ online: false, lastSeenAt: `${DAY}T08:00:00Z`, currentApp: "Slack" }),
        device({ deviceId: "d2", status: "stopped", lastSeenAt: `${DAY}T10:00:00Z` }),
      ],
      NOW,
    );
    expect(off.get("u1")).toMatchObject({ state: "offline", lastSeenAt: `${DAY}T10:00:00.000Z`, currentApp: null, since: null });
  });

  it("skips malformed entries and survives a payload that isn't a list", () => {
    const merged = mergePresence([null, "x", { status: "tracking" }, device({ userId: "u2" })], NOW);
    expect([...merged.keys()]).toEqual(["u2"]);
    expect(mergePresence({ items: [] }, NOW).size).toBe(0);
  });
});

describe("uploadPresence — an older server without heartbeats", () => {
  it("reads a member as tracking while their latest block is within two project intervals", () => {
    const latest = latestBlockByUser([
      act({ id: "recent", userId: "u1", projectId: "p30", endedAt: `${DAY}T11:20:00.000Z` }),
      act({ id: "stale", userId: "u2", projectId: "p10", endedAt: `${DAY}T11:20:00.000Z` }),
    ]);
    const intervals: Record<string, number> = { p30: 30, p10: 10 };
    const presence = uploadPresence(latest, NOW, (pid) => intervals[pid]);
    // 40 minutes ago: inside 2 × 30 min, outside max(15, 2 × 10) min.
    expect(presence.get("u1")).toMatchObject({ state: "tracking", source: "uploads", lastSeenAt: `${DAY}T11:20:00.000Z` });
    expect(presence.get("u2")?.state).toBe("offline");
  });
});

describe("demoPresence — the demo's seeded roster", () => {
  it("follows each member's seeded status, with the app of their latest block while tracking", () => {
    const latest = latestBlockByUser([act({ userId: "a", screen: { app: "Figma", kind: "design", accent: "#000" } })]);
    const presence = demoPresence([member("a"), member("b", { status: "idle" }), member("c", { status: "offline" })], latest);
    expect(presence.get("a")).toMatchObject({ state: "tracking", currentApp: "Figma", source: "demo", lastSeenAt: `${DAY}T09:10:00.000Z` });
    expect(presence.get("b")).toMatchObject({ state: "idle", currentApp: null });
    expect(presence.get("c")?.state).toBe("offline");
  });
});

describe("liveBlockIds — the sessions running right now", () => {
  const interval = () => 10;

  it("is each tracking member's latest block when it ended within one interval plus a grace", () => {
    const rows = [
      act({ id: "a-old", userId: "a", endedAt: `${DAY}T11:30:00.000Z` }),
      act({ id: "a-new", userId: "a", endedAt: `${DAY}T11:50:00.000Z` }),
      act({ id: "b-new", userId: "b", endedAt: `${DAY}T11:55:00.000Z` }),
    ];
    const presence = new Map([
      ["a", { state: "tracking" as const }],
      ["b", { state: "idle" as const }],
    ]);
    expect([...liveBlockIds(rows, presence, NOW, interval)]).toEqual(["a-new"]);
  });

  it("never marks an old block live because the member started again", () => {
    const rows = [act({ id: "yesterday", userId: "a", endedAt: "2026-09-22T18:00:00.000Z" })];
    expect(liveBlockIds(rows, new Map([["a", { state: "tracking" as const }]]), NOW, interval).size).toBe(0);
    // A 30-minute project's block can still be the running one 30 minutes on.
    const long = [act({ id: "long", userId: "a", endedAt: `${DAY}T11:28:00.000Z` })];
    expect(liveBlockIds(long, new Map([["a", { state: "tracking" as const }]]), NOW, () => 30).has("long")).toBe(true);
  });
});
