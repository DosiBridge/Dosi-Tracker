// Boundary + branch behaviour of the live-data mappers.
//
// live-dataset.test.ts covers how each backend shape becomes app data; this
// file pins down the fields a well-formed payload never sends:
//   • captures taken at the same instant, or without a time,
//   • device fields longer than the contract allows, offsets sent as text or
//     as nothing readable, and blocks that name no member or project,
//   • apps with equal time, activity figures sent as text, and summary rows
//     without an id.
import { describe, expect, it } from "vitest";
import { deriveAppCatalog, mapApiActivity, mapApiCaptureMetas, mapReportSummary, type ApiActivityDto } from "./live-dataset";
import { buildActivity } from "@/test/factories";

const block = (over: Partial<Record<keyof ApiActivityDto, unknown>> = {}): ApiActivityDto =>
  ({
    id: "a1",
    userId: "u1",
    projectId: "p1",
    startedAt: "2026-09-23T09:00:00Z",
    endedAt: "2026-09-23T09:10:00Z",
    productivity: 70,
    ...over,
  }) as ApiActivityDto;

describe("captures", () => {
  it("captures taken at the same instant, or without a time, keep the server's order", () => {
    const metas = mapApiCaptureMetas([
      { id: "c2", kind: "screen", capturedAt: "2026-09-23T09:00:00Z" },
      { id: "c1", kind: "webcam", capturedAt: "2026-09-23T09:00:00Z" },
      { id: "c3", kind: "screen" },
      { id: "c0", kind: "screen", capturedAt: "2026-09-23T08:00:00Z" },
      { id: "c4", kind: "screen" },
    ]);
    expect(metas.map((m) => m.id)).toEqual(["c0", "c2", "c1", "c3", "c4"]);
  });
});

describe("mapApiActivity — the agent context as it arrives", () => {
  it("cuts over-long device fields to the contract's lengths and reads an offset sent as text", () => {
    const a = mapApiActivity(block({ osVersion: "x".repeat(80), deviceName: "N".repeat(200), agentVersion: `1.2.3-${"b".repeat(40)}`, utcOffsetMinutes: "330" }));
    expect(a?.osVersion).toHaveLength(64);
    expect(a?.deviceName).toHaveLength(128);
    expect(a?.agentVersion).toHaveLength(32);
    expect(a?.utcOffsetMinutes).toBe(330);
  });

  it("drops an offset that isn't a number, and still maps a block that names no member or project", () => {
    const a = mapApiActivity(block({ userId: undefined, projectId: null, utcOffsetMinutes: "east" }));
    expect(a).toMatchObject({ id: "a1", userId: "", projectId: "" });
    expect(a).not.toHaveProperty("utcOffsetMinutes");
  });
});

describe("app catalog and summaries", () => {
  it("deriveAppCatalog lists apps with equal time by name", () => {
    const rows = [
      buildActivity({
        activeWindows: [
          { appName: "Slack", windowTitle: "general", seconds: 300 },
          { appName: "Figma", windowTitle: "Board", seconds: 300 },
        ],
      }),
    ];
    expect(deriveAppCatalog(rows).map((a) => [a.app, a.minutes])).toEqual([
      ["Figma", 5],
      ["Slack", 5],
    ]);
  });

  it("mapReportSummary reads activity sent as text, an unreadable one as not measured, and skips rows without an id", () => {
    const summary = mapReportSummary({
      totalTrackedMinutes: 90,
      averageProductivity: "63.6",
      perUser: [{ userId: "u1", trackedMinutes: 90, averageProductivity: "high" }],
      perProject: [{ trackedMinutes: 30 }, { projectId: "p1", trackedMinutes: 60 }],
    });
    expect(summary).toMatchObject({ totalMinutes: 90, activity: 64 });
    expect(summary?.users.get("u1")).toEqual({ minutes: 90, activity: null });
    expect([...(summary?.projects ?? [])]).toEqual([["p1", 60]]);
  });
});
