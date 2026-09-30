import { describe, expect, it } from "vitest";
import {
  applyProjectTotals,
  asItems,
  deriveAppCatalog,
  deriveDailyTrend,
  deriveHourly,
  deriveTopApps,
  displayName,
  mapApiActivities,
  mapApiActivity,
  mapApiCaptureMetas,
  mapApiNotifications,
  mapApiProject,
  mapTeamMember,
  memberIdsByProject,
  memberRole,
  normalizeUtcIso,
  parseCaptures,
  parseTimeline,
  parseWindowList,
  thumbnailCaptureIds,
  type ApiActivityDto,
} from "./live-dataset";
import type { Activity } from "./types";

const NOW = new Date("2026-09-23T15:00:00.000Z");

function dto(overrides: Partial<ApiActivityDto> = {}): ApiActivityDto {
  return {
    id: "a1",
    userId: "u1",
    projectId: "p1",
    startedAt: "2026-09-23T14:00:00Z",
    endedAt: "2026-09-23T14:30:00Z",
    productivity: 80,
    mouseClicks: 10,
    keyboardHits: 20,
    description: "Work",
    activeWindowsJson: JSON.stringify([{ appName: "Code", windowTitle: "main.ts" }]),
    runningProgramsJson: "[]",
    ...overrides,
  };
}

function act(overrides: Partial<Activity> = {}): Activity {
  const mapped = mapApiActivity(dto());
  if (!mapped) throw new Error("fixture must map");
  return { ...mapped, ...overrides };
}

describe("normalizeUtcIso", () => {
  it("pins a zone-less ABP timestamp to UTC instead of the viewer's local time", () => {
    expect(normalizeUtcIso("2026-09-23T08:00:00")).toBe("2026-09-23T08:00:00.000Z");
  });
  it("keeps explicit zones", () => {
    expect(normalizeUtcIso("2026-09-23T08:00:00+02:00")).toBe("2026-09-23T06:00:00.000Z");
    expect(normalizeUtcIso("2026-09-23T08:00:00Z")).toBe("2026-09-23T08:00:00.000Z");
  });
  it("rejects non-dates", () => {
    expect(normalizeUtcIso("")).toBeNull();
    expect(normalizeUtcIso("garbage")).toBeNull();
    expect(normalizeUtcIso(42)).toBeNull();
    expect(normalizeUtcIso(null)).toBeNull();
  });
});

describe("parseWindowList — guarded against whatever an agent sent", () => {
  it("parses a camelCase JSON string", () => {
    expect(parseWindowList('[{"appName":"Code","windowTitle":"a.ts","seconds":30}]')).toEqual([
      { appName: "Code", windowTitle: "a.ts", seconds: 30 },
    ]);
  });
  it("accepts PascalCase / snake_case and already-parsed arrays", () => {
    expect(parseWindowList([{ AppName: "Slack", WindowTitle: "#dev", Seconds: 5 }, { app_name: "Term", window_title: "sh" }])).toEqual([
      { appName: "Slack", windowTitle: "#dev", seconds: 5 },
      { appName: "Term", windowTitle: "sh", seconds: 0 },
    ]);
  });
  it.each([["{"], ["null"], ['{"appName":"x"}'], [""], ["   "], [42], [undefined]])(
    "returns [] for malformed input %j (never throws)",
    (raw) => {
      expect(parseWindowList(raw)).toEqual([]);
    },
  );
  it("drops entries without an app name and non-objects", () => {
    expect(parseWindowList([null, 3, { appName: "  " }, { windowTitle: "x" }, { appName: "Ok", seconds: -5 }])).toEqual([
      { appName: "Ok", windowTitle: "", seconds: 0 },
    ]);
  });
  it("keeps per-window input counts only when reported, whatever the key casing", () => {
    const list = parseWindowList(
      '[{"AppName":"code.exe","WindowTitle":"main.rs","Seconds":240,"KeyboardHits":310,"MouseClicks":12},' +
        '{"appName":"slack","keyboardHits":"7","mouse_clicks":2.6},' +
        '{"appName":"term","KeyboardHits":-4,"MouseClicks":"lots"}]',
    );
    expect(list[0]).toEqual({ appName: "code.exe", windowTitle: "main.rs", seconds: 240, keyboardHits: 310, mouseClicks: 12 });
    expect(list[1]).toEqual({ appName: "slack", windowTitle: "", seconds: 0, keyboardHits: 7, mouseClicks: 3 });
    // Negative clamps to 0; a non-number is "not reported", never 0.
    expect(list[2]).toEqual({ appName: "term", windowTitle: "", seconds: 0, keyboardHits: 0 });
    expect("mouseClicks" in list[2]).toBe(false);
  });
  it("a null camelCase key falls through to its PascalCase twin", () => {
    expect(parseWindowList([{ appName: null, AppName: "Rider" }])).toEqual([{ appName: "Rider", windowTitle: "", seconds: 0 }]);
  });
});

describe("parseTimeline — per-minute buckets", () => {
  it("parses PascalCase buckets, sorted by minute, dropping a null AppName", () => {
    expect(
      parseTimeline('[{"Minute":1,"KeyboardHits":4,"MouseClicks":1,"Active":true,"AppName":"code.exe"},{"Minute":0,"KeyboardHits":12,"MouseClicks":3,"Active":true,"AppName":null}]'),
    ).toEqual([
      { minute: 0, keyboardHits: 12, mouseClicks: 3, active: true },
      { minute: 1, keyboardHits: 4, mouseClicks: 1, active: true, appName: "code.exe" },
    ]);
  });
  it("accepts camelCase arrays, infers a missing Active flag from the counts, keeps the last duplicate", () => {
    expect(
      parseTimeline([
        { minute: 0, keyboardHits: 0, mouseClicks: 0 },
        { minute: 1, keyboardHits: 3 },
        { minute: 2, active: false, keyboardHits: 1 },
        { minute: 2, active: true, keyboardHits: 9 },
      ]),
    ).toEqual([
      { minute: 0, keyboardHits: 0, mouseClicks: 0, active: false },
      { minute: 1, keyboardHits: 3, mouseClicks: 0, active: true },
      { minute: 2, keyboardHits: 9, mouseClicks: 0, active: true },
    ]);
  });
  it("drops buckets without a valid minute index", () => {
    expect(
      parseTimeline([{ KeyboardHits: 1 }, { Minute: -1 }, { Minute: 1.5 }, { Minute: "x" }, { Minute: 24 * 60 }, null, [1], { Minute: "3", Active: true }]),
    ).toEqual([{ minute: 3, keyboardHits: 0, mouseClicks: 0, active: true }]);
  });
  it.each([[undefined], [null], [""], ["[]"], ["{broken"], ['{"Minute":0}'], [42]])("returns [] for %j (never throws)", (raw) => {
    expect(parseTimeline(raw)).toEqual([]);
  });
});

describe("capture refs", () => {
  it("parseCaptures keeps known kinds with an id, once each", () => {
    expect(
      parseCaptures([
        { id: "s1", kind: "screen" },
        { Id: "t1", Kind: "THUMB" },
        { id: "w1", kind: "webcam" },
        { id: "s1", kind: "screen" },
        { id: "x1", kind: "audio" },
        { kind: "screen" },
        { id: 7, kind: "screen" },
        null,
      ]),
    ).toEqual([
      { id: "s1", kind: "screen" },
      { id: "t1", kind: "thumb" },
      { id: "w1", kind: "webcam" },
      { id: "7", kind: "screen" },
    ]);
  });
  it.each([[undefined], [null], ["nope"], [{}], [[]]])("parseCaptures(%j) → [] (older backends omit the property)", (raw) => {
    expect(parseCaptures(raw)).toEqual([]);
  });
  it("thumbnailCaptureIds prefers the thumb, then the full screen capture, never the webcam", () => {
    expect(thumbnailCaptureIds({ captures: [{ id: "s", kind: "screen" }, { id: "w", kind: "webcam" }, { id: "t", kind: "thumb" }] })).toEqual(["t", "s"]);
    expect(thumbnailCaptureIds({ captures: [{ id: "s", kind: "screen" }] })).toEqual(["s"]);
    expect(thumbnailCaptureIds({ captures: [{ id: "w", kind: "webcam" }] })).toEqual([]);
    expect(thumbnailCaptureIds({})).toEqual([]);
  });
  it("mapApiCaptureMetas: oldest first, thumbs dropped, unknown kinds read as screen, bad input → []", () => {
    const metas = mapApiCaptureMetas({
      items: [
        { id: "b", kind: "screen", capturedAt: "2026-09-23T14:03:00", blurred: true, sizeBytes: 2048, contentType: "image/png" },
        { id: "t", kind: "thumb", capturedAt: "2026-09-23T14:01:00Z" },
        { id: "a", kind: "webcam", capturedAt: "2026-09-23T14:02:00Z" },
        { id: "c", kind: "mystery" },
        { kind: "screen" },
        null,
      ],
    });
    expect(metas).toEqual([
      { id: "a", kind: "webcam", capturedAt: "2026-09-23T14:02:00.000Z", blurred: false, sizeBytes: null, contentType: null },
      { id: "b", kind: "screen", capturedAt: "2026-09-23T14:03:00.000Z", blurred: true, sizeBytes: 2048, contentType: "image/png" },
      { id: "c", kind: "screen", capturedAt: null, blurred: false, sizeBytes: null, contentType: null },
    ]);
    expect(mapApiCaptureMetas("nope")).toEqual([]);
    expect(mapApiCaptureMetas(null)).toEqual([]);
  });
});

describe("mapApiActivity", () => {
  it("maps a real row and credits the focused app with the block's duration", () => {
    const a = mapApiActivity(dto());
    expect(a).toMatchObject({ id: "a1", userId: "u1", projectId: "p1", productivity: 80, description: "Work", online: false, hasWebcam: false });
    // Flagged, so the detail view can say the time was credited rather than measured.
    expect(a?.activeWindows[0]).toEqual({ appName: "Code", windowTitle: "main.ts", seconds: 1800, credited: true });
    expect(a?.screen.app).toBe("Code");
  });

  it("maps a legacy row (no timeline, no captures property) to empty timeline/captures", () => {
    const a = mapApiActivity(dto());
    expect(a?.timeline).toEqual([]);
    expect(a?.captures).toEqual([]);
    expect(a?.activeWindows[0].keyboardHits).toBeUndefined();
    expect(a?.activeWindows[0].mouseClicks).toBeUndefined();
  });

  it("maps the new per-window usage, timeline and capture refs (PascalCase JSON from .NET)", () => {
    const a = mapApiActivity(
      dto({
        startedAt: "2026-09-23T14:00:00Z",
        endedAt: "2026-09-23T14:04:58Z",
        activeWindowsJson: JSON.stringify([
          { AppName: "chrome.exe", WindowTitle: "Docs", Seconds: 100, KeyboardHits: 5, MouseClicks: 9 },
          { AppName: "code.exe", WindowTitle: "main.rs", Seconds: 240, KeyboardHits: 310, MouseClicks: 12 },
          { AppName: "chrome.exe", WindowTitle: "Mail", Seconds: 90, KeyboardHits: 1, MouseClicks: 3 },
        ]),
        timelineJson: JSON.stringify([
          { Minute: 1, KeyboardHits: 40, MouseClicks: 2, Active: true, AppName: "code.exe" },
          { Minute: 0, KeyboardHits: 12, MouseClicks: 3, Active: true, AppName: null },
          { Minute: 2, KeyboardHits: 0, MouseClicks: 0, Active: false },
        ]),
        captures: [
          { id: "s1", kind: "screen" },
          { id: "t1", kind: "thumb" },
          { id: "w1", kind: "webcam" },
        ],
      }),
    );
    expect(a?.activeWindows[1]).toEqual({ appName: "code.exe", windowTitle: "main.rs", seconds: 240, keyboardHits: 310, mouseClicks: 12 });
    // Measured seconds are never overwritten by the legacy credit.
    expect(a?.activeWindows.some((w) => w.credited)).toBe(false);
    // Main app = most focused time per app: code 240s vs chrome 100 + 90 = 190s.
    expect(a?.screen.app).toBe("code.exe");
    expect(a?.timeline).toEqual([
      { minute: 0, keyboardHits: 12, mouseClicks: 3, active: true },
      { minute: 1, keyboardHits: 40, mouseClicks: 2, active: true, appName: "code.exe" },
      { minute: 2, keyboardHits: 0, mouseClicks: 0, active: false },
    ]);
    expect(a?.captures).toEqual([
      { id: "s1", kind: "screen" },
      { id: "t1", kind: "thumb" },
      { id: "w1", kind: "webcam" },
    ]);
    expect(a?.hasWebcam).toBe(true);
  });

  it("names the block after the app with the most focused time summed over its windows", () => {
    const a = mapApiActivity(
      dto({
        activeWindowsJson: JSON.stringify([
          { appName: "code.exe", windowTitle: "main.rs", seconds: 200 },
          { appName: "chrome.exe", windowTitle: "Docs", seconds: 150 },
          { appName: "Chrome.exe", windowTitle: "Mail", seconds: 150 },
        ]),
      }),
    );
    expect(a?.screen.app).toBe("chrome.exe");
  });

  it("survives a corrupt timeline and captures payload", () => {
    const a = mapApiActivity(dto({ timelineJson: "{not json", captures: "nope" }));
    expect(a?.timeline).toEqual([]);
    expect(a?.captures).toEqual([]);
    expect(a?.hasWebcam).toBe(false);
  });

  it("keeps agent-reported seconds when present", () => {
    const a = mapApiActivity(dto({ activeWindowsJson: '[{"appName":"Code","seconds":120}]' }));
    expect(a?.activeWindows[0].seconds).toBe(120);
  });

  it("clamps numbers, defaults missing fields and survives corrupt window JSON", () => {
    const a = mapApiActivity(
      dto({ productivity: 250, mouseClicks: -3, keyboardHits: null, description: "  ", activeWindowsJson: "{broken", runningProgramsJson: undefined }),
    );
    expect(a).toMatchObject({ productivity: 100, mouseClicks: 0, keyboardHits: 0, description: "Tracked session", activeWindows: [], runningPrograms: [] });
    expect(a?.screen.app).toBe("Desktop");
  });

  it.each([
    [null],
    [{ ...dto(), id: "" }],
    [{ ...dto(), startedAt: "nope" }],
    [{ ...dto(), endedAt: undefined as unknown as string }],
  ])("drops unusable rows (%#)", (row) => {
    expect(mapApiActivity(row as ApiActivityDto)).toBeNull();
  });

  it("mapApiActivities filters bad rows, sorts newest first and rejects non-arrays", () => {
    const list = mapApiActivities([
      dto({ id: "old", endedAt: "2026-09-22T10:00:00Z", startedAt: "2026-09-22T09:00:00Z" }),
      { bogus: true },
      dto({ id: "new" }),
    ]);
    expect(list.map((a) => a.id)).toEqual(["new", "old"]);
    expect(mapApiActivities({ items: [] })).toEqual([]);
  });
});

describe("projects and roster", () => {
  it("mapApiProject fills safe defaults and always carries a memberIds array", () => {
    const p = mapApiProject({ id: "p9", isArchived: true, allowWebcam: true, allowScreenshot: false, creationTime: "2026-09-01T00:00:00" });
    expect(p).toMatchObject({ id: "p9", title: "Untitled project", archived: true, intervalMinutes: 10, createdAt: "2026-09-01", memberIds: [] });
    expect(p.permissions).toEqual({ screenshot: false, webcam: true, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true });
    expect(p.color).toMatch(/^hsl\(/);
  });

  it("copies member ids (and tolerates a non-array)", () => {
    const ids = ["u1"];
    const p = mapApiProject({ id: "p1", title: "X", color: "#123456", intervalMinutes: 15 }, ids);
    ids.push("u2");
    expect(p.memberIds).toEqual(["u1"]);
    expect(p).toMatchObject({ title: "X", color: "#123456", intervalMinutes: 15 });
    expect(mapApiProject({ id: "p2" }, null as unknown as string[]).memberIds).toEqual([]);
  });

  it("memberIdsByProject inverts the roster without duplicates", () => {
    const map = memberIdsByProject([
      { userId: "u1", projectIds: ["p1", "p2"] },
      { userId: "u2", projectIds: ["p1", "p1"] },
      { userId: "u3", projectIds: null },
    ]);
    expect(map.get("p1")).toEqual(["u1", "u2"]);
    expect(map.get("p2")).toEqual(["u1"]);
  });

  it("displayName prefers the full name, then userName, email, fallback", () => {
    expect(displayName({ name: "Mira", surname: "Okafor" })).toBe("Mira Okafor");
    expect(displayName({ name: "Mira", surName: "O." })).toBe("Mira O.");
    expect(displayName({ userName: "mira" })).toBe("mira");
    expect(displayName({ email: "m@x.io" })).toBe("m@x.io");
    expect(displayName({}, "You")).toBe("You");
  });

  it("memberRole: owner flag wins, then manager, else worker", () => {
    expect(memberRole({ isOwner: true, isManager: true })).toBe("owner");
    expect(memberRole({ isManager: true })).toBe("admin");
    expect(memberRole({})).toBe("worker");
  });

  it("mapTeamMember derives today's time and live status from the member's own rows", () => {
    const rows = [
      act({ userId: "u1", startedAt: "2026-09-23T14:00:00.000Z", endedAt: "2026-09-23T14:50:00.000Z", productivity: 60 }),
      act({ id: "y", userId: "u1", startedAt: "2026-09-22T10:00:00.000Z", endedAt: "2026-09-22T11:00:00.000Z" }),
      act({ id: "z", userId: "u2", startedAt: "2026-09-23T10:00:00.000Z", endedAt: "2026-09-23T11:00:00.000Z" }),
    ];
    const u1 = mapTeamMember({ userId: "u1", name: "Ann", email: "a@x.io", isManager: true }, rows, NOW);
    expect(u1).toMatchObject({ id: "u1", name: "Ann", role: "admin", designation: "Administrator", trackedToday: 50, productivity: 60, status: "active" });

    const u2 = mapTeamMember({ userId: "u2", userName: "ben" }, rows, NOW);
    expect(u2).toMatchObject({ name: "ben", role: "worker", trackedToday: 60, status: "offline" });

    const gone = mapTeamMember({ userId: "u1", isActive: false }, rows, NOW);
    expect(gone.status).toBe("offline");
    expect(mapTeamMember({ userId: "nobody" }, rows, NOW)).toMatchObject({ trackedToday: 0, productivity: 0, status: "offline", email: "" });
  });
});

describe("derived series (real rows only — empty stays empty)", () => {
  it("deriveDailyTrend buckets the last N UTC days, oldest first", () => {
    const trend = deriveDailyTrend(
      [
        act({ startedAt: "2026-09-23T10:00:00.000Z", endedAt: "2026-09-23T11:00:00.000Z", productivity: 50 }),
        act({ startedAt: "2026-09-21T10:00:00.000Z", endedAt: "2026-09-21T10:30:00.000Z", productivity: 100 }),
        act({ startedAt: "2026-09-01T10:00:00.000Z", endedAt: "2026-09-01T10:30:00.000Z" }), // outside
      ],
      NOW,
      3,
    );
    expect(trend).toEqual([
      { day: "Mon", tracked: 30, productive: 30 },
      { day: "Tue", tracked: 0, productive: 0 },
      { day: "Wed", tracked: 60, productive: 30 },
    ]);
    expect(deriveDailyTrend([], NOW)).toHaveLength(7);
  });

  it("deriveHourly splits a session across local hours and keeps the 8–19 axis", () => {
    const localDay = new Date(2026, 8, 23, 9, 45); // local 09:45
    const end = new Date(2026, 8, 23, 10, 15);
    const hours = deriveHourly([act({ startedAt: localDay.toISOString(), endedAt: end.toISOString() })], new Date(2026, 8, 23, 18));
    expect(hours[0].hour).toBe("8:00");
    expect(hours[hours.length - 1].hour).toBe("19:00");
    expect(hours.find((h) => h.hour === "9:00")?.minutes).toBe(15);
    expect(hours.find((h) => h.hour === "10:00")?.minutes).toBe(15);
    expect(hours.reduce((s, h) => s + h.minutes, 0)).toBe(30);
  });

  it("deriveHourly widens the axis for early/late work and ignores other days and bad rows", () => {
    const early = new Date(2026, 8, 23, 6, 0);
    const late = new Date(2026, 8, 23, 21, 0);
    const hours = deriveHourly(
      [
        act({ startedAt: early.toISOString(), endedAt: new Date(early.getTime() + 20 * 60_000).toISOString() }),
        act({ startedAt: late.toISOString(), endedAt: new Date(late.getTime() + 10 * 60_000).toISOString() }),
        act({ startedAt: "2020-01-01T00:00:00Z", endedAt: "2020-01-01T01:00:00Z" }),
        act({ startedAt: "garbage", endedAt: "garbage" }),
      ],
      new Date(2026, 8, 23, 22),
    );
    expect(hours[0]).toEqual({ hour: "6:00", minutes: 20 });
    expect(hours[hours.length - 1]).toEqual({ hour: "21:00", minutes: 10 });
    expect(deriveHourly([], NOW).every((h) => h.minutes === 0)).toBe(true);
  });

  it("deriveAppCatalog credits focused time per app with member reach, uncategorized", () => {
    const rows = [
      act({ userId: "u1", activeWindows: [{ appName: "Code", windowTitle: "", seconds: 3600 }] }),
      act({ userId: "u2", activeWindows: [{ appName: "Code", windowTitle: "", seconds: 1800 }, { appName: "Slack", windowTitle: "", seconds: 600 }] }),
      act({ userId: "u2", activeWindows: [{ appName: "Idle", windowTitle: "", seconds: 0 }] }),
    ];
    const catalog = deriveAppCatalog(rows);
    expect(catalog.map((a) => [a.app, a.minutes, a.activeUsers, a.category])).toEqual([
      ["Code", 90, 2, "neutral"],
      ["Slack", 10, 1, "neutral"],
    ]);
    expect(deriveTopApps(catalog)).toEqual([
      { app: "Code", minutes: 90, color: catalog[0].color },
      { app: "Slack", minutes: 10, color: catalog[1].color },
    ]);
    expect(deriveAppCatalog([])).toEqual([]);
  });

  it("applyProjectTotals fills week / month / loaded totals from the rows", () => {
    const project = mapApiProject({ id: "p1", title: "P" });
    applyProjectTotals(
      [project],
      [
        act({ projectId: "p1", startedAt: "2026-09-23T10:00:00.000Z", endedAt: "2026-09-23T11:00:00.000Z" }),
        act({ projectId: "p1", startedAt: "2026-09-02T10:00:00.000Z", endedAt: "2026-09-02T10:30:00.000Z" }),
        act({ projectId: "p1", startedAt: "2026-08-20T10:00:00.000Z", endedAt: "2026-08-20T10:15:00.000Z" }),
        act({ projectId: "other", startedAt: "2026-09-23T10:00:00.000Z", endedAt: "2026-09-23T12:00:00.000Z" }),
      ],
      NOW,
    );
    expect(project).toMatchObject({ loggedThisWeek: 60, loggedThisMonth: 90, loggedTotal: 105 });
  });
});

describe("notifications and list envelopes", () => {
  it("maps real notifications newest first, inferring a type from the message", () => {
    const list = mapApiNotifications([
      { id: "1", message: "You were invited to Apollo", isRead: false, creationTime: "2026-09-20T10:00:00" },
      { id: "2", message: "Weekly report is ready", isRead: true, creationTime: "2026-09-22T10:00:00Z" },
      { id: "3", message: "Seat limit reached", creationTime: "2026-09-21T10:00:00Z" },
      { id: "4", message: "", creationTime: null },
      { id: 5 },
      null,
    ]);
    expect(list.map((n) => [n.id, n.type, n.read])).toEqual([
      ["2", "report", true],
      ["3", "alert", false],
      ["1", "member", false],
      ["4", "system", false],
    ]);
    expect(list[3].title).toBe("Notification");
    expect(list[2].at).toBe("2026-09-20T10:00:00.000Z");
    expect(mapApiNotifications("nope")).toEqual([]);
  });

  it("asItems accepts an unwrapped array or an { items } envelope", () => {
    expect(asItems([1, 2])).toEqual([1, 2]);
    expect(asItems({ items: [3] })).toEqual([3]);
    expect(asItems({ items: "x" })).toEqual([]);
    expect(asItems(null)).toEqual([]);
  });
});
