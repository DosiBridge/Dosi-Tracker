import { describe, expect, it } from "vitest";
import type { AppUsageRowDto, DailyReportRowDto, HourlyReportRowDto } from "@/lib/api-contract";
import { appColor } from "@/lib/apps";
import { appsFromUsage, arrayOf, hoursFromReport, trendFromSeries, weekdayOf } from "./dashboard-data";
import { clientProjectStatus, clientProjectStatusOnDay } from "./client-status";

function day(date: string, trackedMinutes: number, averageProductivity: number | null): DailyReportRowDto {
  return { date: `${date}T00:00:00`, activityCount: 1, trackedMinutes, averageProductivity, mouseClicks: 0, keyboardHits: 0 };
}

describe("trendFromSeries", () => {
  it("gives one point per day, in order, filling days the server left out with zero", () => {
    const days = ["2026-10-01", "2026-10-02", "2026-10-03"];
    const points = trendFromSeries(days, [day("2026-10-03", 120, 50), day("2026-10-01", 60, 80)]);
    expect(points).toEqual([
      { day: "Thu", tracked: 60, productive: 48 },
      { day: "Fri", tracked: 0, productive: 0 },
      { day: "Sat", tracked: 120, productive: 60 },
    ]);
  });

  it("counts no active time on a day whose input wasn't measured", () => {
    expect(trendFromSeries(["2026-10-01"], [day("2026-10-01", 90, null)])).toEqual([{ day: "Thu", tracked: 90, productive: 0 }]);
  });

  it("names weekdays from the day key, not the browser's zone", () => {
    expect(weekdayOf("2026-10-05")).toBe("Mon");
  });
});

describe("appsFromUsage", () => {
  it("groups rows under the canonical app name and colours by the app, never its rank", () => {
    const rows: AppUsageRowDto[] = [
      { appName: "chrome.exe", trackedMinutes: 30, activityCount: 1, userCount: 1 },
      { appName: "Google Chrome", trackedMinutes: 15, activityCount: 1, userCount: 1 },
      { appName: "Code.exe", displayName: "Visual Studio Code", trackedMinutes: 60, activityCount: 2, userCount: 1 },
      { appName: "idle", trackedMinutes: 0, activityCount: 0, userCount: 0 },
    ];
    expect(appsFromUsage(rows)).toEqual([
      { app: "Visual Studio Code", minutes: 60, color: appColor("Visual Studio Code") },
      { app: "Google Chrome", minutes: 45, color: appColor("Google Chrome") },
    ]);
  });

  it("keeps the top N", () => {
    const rows = Array.from({ length: 9 }, (_, i): AppUsageRowDto => ({ appName: `App${i}`, trackedMinutes: 10 + i, activityCount: 1, userCount: 1 }));
    expect(appsFromUsage(rows, 3).map((a) => a.minutes)).toEqual([18, 17, 16]);
  });
});

describe("hoursFromReport", () => {
  it("sums each hour across weekdays and widens 8:00–19:00 to any hour with work", () => {
    const rows: HourlyReportRowDto[] = [
      { dayOfWeek: 1, hour: 6, trackedMinutes: 20, activeMinutes: 10 },
      { dayOfWeek: 1, hour: 9, trackedMinutes: 30, activeMinutes: 20 },
      { dayOfWeek: 2, hour: 9, trackedMinutes: 15, activeMinutes: 5 },
      { dayOfWeek: 1, hour: 24, trackedMinutes: 99, activeMinutes: 0 },
    ];
    const hours = hoursFromReport(rows);
    expect(hours[0]).toEqual({ hour: "6:00", minutes: 20 });
    expect(hours.at(-1)?.hour).toBe("19:00");
    expect(hours.find((h) => h.hour === "9:00")?.minutes).toBe(45);
    expect(hours.reduce((s, h) => s + h.minutes, 0)).toBe(65);
  });
});

describe("arrayOf", () => {
  it("returns arrays as they are and [] for anything else", () => {
    expect(arrayOf([1, 2])).toEqual([1, 2]);
    expect(arrayOf({ items: [1] })).toEqual([]);
    expect(arrayOf(null)).toEqual([]);
  });
});

describe("clientProjectStatus", () => {
  const now = new Date("2026-10-05T12:00:00Z");
  const at = (hoursAgo: number) => new Date(now.getTime() - hoursAgo * 3_600_000).toISOString();

  it("derives the badge from when work was last tracked", () => {
    expect(clientProjectStatus(at(3), now)).toMatchObject({ label: "Active", tone: "success" });
    expect(clientProjectStatus(at(24 * 5), now)).toMatchObject({ label: "Active this week", tone: "info" });
    expect(clientProjectStatus(at(24 * 12), now)).toMatchObject({ label: "Quiet for 12 days", tone: "warning" });
    expect(clientProjectStatus(at(24 * 40), now)).toMatchObject({ label: "No recent work", tone: "muted" });
    expect(clientProjectStatus(null, now)).toMatchObject({ label: "No recent work", detail: "Nothing tracked in the last 30 days" });
  });

  it("says what the badge is based on", () => {
    expect(clientProjectStatus(at(3), now).detail).toBe("Last work tracked 3h ago");
  });
});

describe("clientProjectStatusOnDay", () => {
  it("reports a daily series at day precision, aged in calendar days", () => {
    expect(clientProjectStatusOnDay("2026-10-05", "2026-10-05")).toMatchObject({ label: "Active", detail: "Last work tracked today" });
    expect(clientProjectStatusOnDay("2026-10-04", "2026-10-05").detail).toBe("Last work tracked yesterday");
    // Work late on day N-2 is still "Active", not tipped a tier down by a start-of-day instant.
    expect(clientProjectStatusOnDay("2026-10-03", "2026-10-05")).toMatchObject({ label: "Active", tone: "success" });
    expect(clientProjectStatusOnDay("2026-09-30", "2026-10-05")).toMatchObject({ label: "Active this week", detail: "Last work tracked on Wed, 30 Sep" });
    expect(clientProjectStatusOnDay("2026-09-23", "2026-10-05")).toMatchObject({ label: "Quiet for 12 days", tone: "warning" });
    expect(clientProjectStatusOnDay(null, "2026-10-05")).toMatchObject({ label: "No recent work" });
  });
});
