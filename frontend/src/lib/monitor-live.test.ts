// The live Member Monitor draws ONLY real rows. These tests pin the mapping
// from a backend activity to a day segment, the per-member grouping, presence,
// the roster a manager sees, and the axis that must fit a real (UTC) day.
import { describe, it, expect } from "vitest";
import {
  ACTIVE_WITHIN_MIN,
  DAY_ACTIVITY_LIMIT,
  DEFAULT_DAY_WINDOW,
  activityToSegment,
  dayActivityEndpoint,
  lastSeenByUser,
  liveRoster,
  liveStatus,
  segmentsByUser,
  timelineWindow,
  unknownMember,
} from "./monitor-live";
import type { DaySegment } from "./monitor-data";
import type { Activity, User } from "./types";
import { colorFromString } from "./utils";

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
  it("asks for one whole UTC day, capped at the backend page-size ceiling", () => {
    expect(DAY_ACTIVITY_LIMIT).toBe(1000);
    expect(dayActivityEndpoint(DAY)).toBe(
      "/api/app/activity?From=2026-09-23T00%3A00%3A00.000Z&To=2026-09-24T00%3A00%3A00.000Z&MaxResultCount=1000",
    );
  });

  it("narrows to one member when a user id is given (encoded)", () => {
    expect(dayActivityEndpoint(DAY, "a b")).toBe(
      "/api/app/activity?From=2026-09-23T00%3A00%3A00.000Z&To=2026-09-24T00%3A00%3A00.000Z&UserId=a%20b&MaxResultCount=1000",
    );
  });
});

describe("activityToSegment", () => {
  it("maps a block to minutes of its UTC day with the real figures", () => {
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
      app: "Code",
      windowTitle: "main.rs",
      projectId: "p1",
      productivity: 80,
      mouseClicks: 12,
      keyboardHits: 340,
      screen: { app: "Code", kind: "docs", accent: colorFromString("Code", 55, 45) },
    });
    expect(s?.tabs).toBeUndefined();
  });

  it("credits the window with the most seconds (first one on a tie)", () => {
    const windows = [
      { appName: "Slack", windowTitle: "#general", seconds: 60 },
      { appName: "Code", windowTitle: "api.rs", seconds: 400 },
      { appName: "Figma", windowTitle: "Board", seconds: 400 },
    ];
    const s = activityToSegment(act({ activeWindows: windows }), DAY);
    expect(s?.app).toBe("Code");
    expect(s?.windowTitle).toBe("api.rs");
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

  it("drops rows outside the day, zero-length rows and unusable times instead of rendering NaN", () => {
    expect(activityToSegment(act(), "2026-09-21")).toBeNull();
    expect(activityToSegment(act({ endedAt: `${DAY}T09:00:00.000Z` }), DAY)).toBeNull();
    expect(activityToSegment(act({ endedAt: `${DAY}T08:00:00.000Z` }), DAY)).toBeNull();
    expect(activityToSegment(act({ startedAt: "not a date" }), DAY)).toBeNull();
    expect(activityToSegment(act({ endedAt: "" }), DAY)).toBeNull();
    expect(activityToSegment(act(), "bad-day")).toBeNull();
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
});

describe("lastSeenByUser", () => {
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
});

describe("liveStatus", () => {
  const now = Date.parse(`${DAY}T12:00:00.000Z`);

  it("is active only while the latest block ended within the window", () => {
    expect(ACTIVE_WITHIN_MIN).toBe(15);
    expect(liveStatus(now - 15 * 60_000, now)).toBe("active");
    expect(liveStatus(now - 15 * 60_000 - 1, now)).toBe("offline");
    expect(liveStatus(now, now)).toBe("active");
  });

  it("is offline for a member who has never reported", () => {
    expect(liveStatus(undefined, now)).toBe("offline");
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

  it("is empty when there is no one to monitor", () => {
    expect(liveRoster([], [], new Map(), now)).toEqual([]);
  });
});

describe("timelineWindow", () => {
  it("is the working day when every segment fits in it (and when there are none)", () => {
    expect(DEFAULT_DAY_WINDOW).toEqual({ start: 480, end: 1200 });
    expect(timelineWindow([])).toEqual({ start: 480, end: 1200 });
    expect(timelineWindow([seg(480, 1200), seg(600, 700)])).toEqual({ start: 480, end: 1200 });
  });

  it("widens to whole two-hour marks so work outside it stays on the axis", () => {
    expect(timelineWindow([seg(185, 250)])).toEqual({ start: 120, end: 1200 });
    expect(timelineWindow([seg(1250, 1300)])).toEqual({ start: 480, end: 1320 });
    expect(timelineWindow([seg(360, 400), seg(1201, 1210)])).toEqual({ start: 360, end: 1320 });
  });

  it("never runs past midnight on either side", () => {
    expect(timelineWindow([seg(0, 30), seg(1430, 1440)])).toEqual({ start: 0, end: 1440 });
    expect(timelineWindow([seg(-30, 10), seg(1400, 1500)])).toEqual({ start: 0, end: 1440 });
  });
});
