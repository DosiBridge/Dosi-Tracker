// Unmeasured keyboard / mouse counts through Work Analytics: a block whose
// input wasn't counted (consent off, a Wayland session) is null all the way to
// the charts and exports — never a 0 that reads as "no input" (API-05).
import { describe, expect, it } from "vitest";
import {
  aggregateBlocks,
  blocksFromApi,
  dayStats,
  hourProfile,
  hourTotals,
  memberStats,
  memberClock,
  summarize,
  zoneCalendar,
} from "./analytics";

const UTC = zoneCalendar("UTC");
const DAY = "2026-09-28";

function dto(id: string, startIso: string, minutes: number, extra: Record<string, unknown> = {}) {
  const start = Date.parse(startIso);
  return {
    id,
    userId: "u1",
    projectId: "p1",
    startedAt: startIso,
    endedAt: new Date(start + minutes * 60_000).toISOString(),
    productivity: 60,
    ...extra,
  };
}

const unmeasured = dto("a", `${DAY}T09:00:00Z`, 30, { keyboardHits: null, mouseClicks: null });
const measured = dto("b", `${DAY}T10:00:00Z`, 30, { keyboardHits: 120, mouseClicks: 0 });

describe("Work Analytics with unmeasured input", () => {
  it("keeps a day, an hour and a member with no measured input not measured", () => {
    const blocks = blocksFromApi([unmeasured]);
    const agg = aggregateBlocks(blocks, UTC);
    expect(agg.totals).toMatchObject({ keys: null, clicks: null });
    expect(dayStats(agg, [DAY])[0]).toMatchObject({ keys: null, clicks: null });
    expect(hourProfile(blocks, UTC)[9]).toMatchObject({ minutes: 30, keys: null, clicks: null });
    const [member] = memberStats(agg, blocks, [DAY], memberClock(blocks, UTC));
    expect(member).toMatchObject({ keys: null, clicks: null, inputPerHour: null });
    expect(summarize(agg, [DAY], blocks, []).inputPerHour).toBeNull();
  });

  it("adds only what was measured, and the rate leaves the unmeasured minutes out", () => {
    const blocks = blocksFromApi([unmeasured, measured]);
    const agg = aggregateBlocks(blocks, UTC);
    expect(agg.totals).toMatchObject({ keys: 120, clicks: 0 });
    expect(hourProfile(blocks, UTC)[10]).toMatchObject({ keys: 120, clicks: 0 });
    expect(hourProfile(blocks, UTC)[9]).toMatchObject({ keys: null, clicks: null });
    const [member] = memberStats(agg, blocks, [DAY], memberClock(blocks, UTC));
    // 120 inputs over the 30 measured minutes: 240 an hour (not 120 over all 60 minutes).
    expect(member).toMatchObject({ keys: 120, clicks: 0, inputPerHour: 240 });
    // The period's rate is over its tracked minutes (the server's totals don't say which were measured).
    expect(summarize(agg, [DAY], blocks, []).inputPerHour).toBe(120);
  });

  it("reads the hour cells' input as not carried, not as zero", () => {
    expect(hourTotals([{ weekday: 0, hour: 9, minutes: 30 }])[9]).toMatchObject({ minutes: 30, keys: null, clicks: null });
  });
});
