import type { AppUsageRowDto, DailyReportRowDto, HourlyReportRowDto } from "@/lib/api-contract";
import { appColor, friendlyAppName } from "@/lib/apps";
import type { TrendPoint } from "./charts";

/* ============================================================================
 * DASHBOARD DATA — the reporting API's rows shaped for the dashboard charts.
 * Every input is a server aggregate for whole calendar days of the reporting
 * zone (contract §6); nothing here counts rows in the browser. Pure.
 * ========================================================================== */

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

/** The array a list endpoint returned, or [] for anything else (an older server's shape, an error body). */
export function arrayOf<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

/** "Mon" for the day key "2026-07-13". */
export function weekdayOf(dayKey: string): string {
  return WEEKDAYS[new Date(`${dayKey}T00:00:00Z`).getUTCDay()];
}

/**
 * One trend point per day of `days` (day keys, oldest first), from the daily
 * series. A day the server left out tracked nothing. Active minutes are the
 * tracked minutes × the day's activity %; a day with nothing measured has none.
 */
export function trendFromSeries(days: readonly string[], series: readonly DailyReportRowDto[]): TrendPoint[] {
  const byDay = new Map(series.filter((r) => r && typeof r.date === "string").map((r) => [r.date.slice(0, 10), r]));
  return days.map((key) => {
    const row = byDay.get(key);
    const tracked = Math.round(Number(row?.trackedMinutes) || 0);
    const activity = typeof row?.averageProductivity === "number" ? row.averageProductivity : null;
    return { day: weekdayOf(key), tracked, productive: activity === null ? 0 : Math.round((tracked * activity) / 100) };
  });
}

/**
 * App rows under their canonical names, biggest first: an older server groups
 * "chrome.exe" and "Google Chrome" apart, and both read "Google Chrome" here.
 * Colours follow the app (lib/apps), never its rank.
 */
export function appsFromUsage(rows: readonly AppUsageRowDto[], limit = 6): { app: string; minutes: number; color: string }[] {
  const minutes = new Map<string, number>();
  for (const r of rows) {
    if (!r) continue;
    const name = (typeof r.displayName === "string" && r.displayName.trim()) || friendlyAppName(r.appName);
    minutes.set(name, (minutes.get(name) ?? 0) + (Number(r.trackedMinutes) || 0));
  }
  return [...minutes.entries()]
    .map(([app, m]) => ({ app, minutes: Math.round(m), color: appColor(app) }))
    .filter((a) => a.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.app.localeCompare(b.app))
    .slice(0, limit);
}

/** Minutes per hour of the day, 8:00–19:00 widened to any hour with work. */
export function hoursFromReport(rows: readonly HourlyReportRowDto[]): { hour: string; minutes: number }[] {
  const buckets = Array<number>(24).fill(0);
  for (const r of rows) {
    if (r && Number.isInteger(r.hour) && r.hour >= 0 && r.hour < 24) buckets[r.hour] += Number(r.trackedMinutes) || 0;
  }
  const used = buckets.map((m, h) => (m > 0 ? h : -1)).filter((h) => h >= 0);
  const first = Math.min(8, ...used);
  const last = Math.max(19, ...used);
  const out: { hour: string; minutes: number }[] = [];
  for (let h = first; h <= last; h++) out.push({ hour: `${h}:00`, minutes: Math.round(buckets[h]) });
  return out;
}
