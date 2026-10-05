/* ============================================================================
 * TIME ZONES — calendar days and clock times in a named IANA zone, using only
 * the platform's Intl (no date library, no zone data shipped).
 *
 * Reports bucket every block into a calendar day of ONE reporting zone — the
 * viewer's preference, else the workspace default, else the browser's — and
 * the backend does the same with the `timeZone` it is sent (contract §6, §7).
 * A "day key" is such a local date written "YYYY-MM-DD". Its UTC bounds come
 * from startOfZonedDay, which is exact across DST changes (a 23- or 25-hour
 * day is still one day) and in zones with odd offsets such as Asia/Kathmandu
 * (+05:45). Day arithmetic works on the keys themselves, never by adding 24h.
 *
 * An unknown zone falls back to UTC everywhere, as the server does.
 * ========================================================================== */

const DAY_MS = 86_400_000;
const HOUR_MS = 3_600_000;

/** Longest zone id the backend stores (contract §1: timeZone ≤ 64). */
const MAX_ZONE_LENGTH = 64;

/** IANA-shaped ids ("UTC", "Asia/Dhaka", "America/Argentina/Buenos_Aires", "Etc/GMT+5"); never "+05:30". */
const ZONE_SHAPE = /^[A-Za-z][A-Za-z0-9_+-]*(?:\/[A-Za-z0-9_+-]+)*$/;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

/* ----------------------------- Zones ----------------------------- */

const zoneValidity = new Map<string, boolean>();

/** Whether `value` is a time zone id the platform knows (IANA ids only; offsets like "+05:30" are not zones). */
export function isValidTimeZone(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_ZONE_LENGTH || !ZONE_SHAPE.test(value)) {
    return false;
  }
  let valid = zoneValidity.get(value);
  if (valid === undefined) {
    try {
      new Intl.DateTimeFormat("en-US", { timeZone: value });
      valid = true;
    } catch {
      valid = false;
    }
    zoneValidity.set(value, valid);
  }
  return valid;
}

/**
 * Names ICU still uses as canonical although IANA renamed the zone. Browsers
 * report and list the old ones ("Asia/Calcutta" for an Indian user), while
 * current tzdata packages ship the old names only as optional legacy links —
 * so the web always speaks the current name, which every server knows.
 */
const CURRENT_ZONE_NAMES: Record<string, string> = {
  "Africa/Asmera": "Africa/Asmara",
  "America/Buenos_Aires": "America/Argentina/Buenos_Aires",
  "America/Catamarca": "America/Argentina/Catamarca",
  "America/Coral_Harbour": "America/Atikokan",
  "America/Cordoba": "America/Argentina/Cordoba",
  "America/Godthab": "America/Nuuk",
  "America/Indianapolis": "America/Indiana/Indianapolis",
  "America/Jujuy": "America/Argentina/Jujuy",
  "America/Louisville": "America/Kentucky/Louisville",
  "America/Mendoza": "America/Argentina/Mendoza",
  "Asia/Calcutta": "Asia/Kolkata",
  "Asia/Katmandu": "Asia/Kathmandu",
  "Asia/Rangoon": "Asia/Yangon",
  "Asia/Saigon": "Asia/Ho_Chi_Minh",
  "Atlantic/Faeroe": "Atlantic/Faroe",
  "Europe/Kiev": "Europe/Kyiv",
  "Pacific/Enderbury": "Pacific/Kanton",
  "Pacific/Ponape": "Pacific/Pohnpei",
  "Pacific/Truk": "Pacific/Chuuk",
};

/** The current IANA name of a zone the platform calls by an old one (kept when the platform doesn't know the new name). */
function currentName(zone: string): string {
  const renamed = Object.prototype.hasOwnProperty.call(CURRENT_ZONE_NAMES, zone) ? CURRENT_ZONE_NAMES[zone] : undefined;
  return renamed && isValidTimeZone(renamed) ? renamed : zone;
}

/**
 * A valid zone id in its canonical, current spelling, or null: letter case
 * corrected ("asia/dhaka" → "Asia/Dhaka"), links resolved ("US/Eastern" →
 * "America/New_York") and renamed zones given their current name
 * ("Asia/Calcutta" → "Asia/Kolkata").
 */
export function normalizeTimeZone(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const zone = value.trim();
  if (!isValidTimeZone(zone)) return null;
  return currentName(new Intl.DateTimeFormat("en-US", { timeZone: zone }).resolvedOptions().timeZone);
}

/** The browser's own zone (current IANA name); "UTC" when the platform can't say. */
export function browserTimeZone(): string {
  try {
    return normalizeTimeZone(new Intl.DateTimeFormat().resolvedOptions().timeZone) ?? "UTC";
  } catch {
    return "UTC";
  }
}

/** Every zone the platform knows, by current name, sorted, always including "UTC". */
export function supportedTimeZones(): string[] {
  let zones: string[] = [];
  try {
    const supportedValuesOf = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
    if (typeof supportedValuesOf === "function") zones = supportedValuesOf("timeZone");
  } catch {
    zones = [];
  }
  const set = new Set(zones.filter((zone) => isValidTimeZone(zone)).map(currentName));
  set.add("UTC");
  return [...set].sort();
}

/** Where the reporting zone came from. */
export type TimeZoneSource = "user" | "workspace" | "browser";

/**
 * The reporting zone: the viewer's preference, then the workspace default,
 * then the browser's zone (contract §7). Invalid or empty candidates are
 * skipped; with nothing usable the answer is UTC.
 */
export function effectiveTimeZone(candidates: {
  user?: string | null;
  workspace?: string | null;
  browser?: string | null;
}): { timeZone: string; source: TimeZoneSource } {
  const user = normalizeTimeZone(candidates.user);
  if (user) return { timeZone: user, source: "user" };
  const workspace = normalizeTimeZone(candidates.workspace);
  if (workspace) return { timeZone: workspace, source: "workspace" };
  return { timeZone: normalizeTimeZone(candidates.browser) ?? "UTC", source: "browser" };
}

function zoneOrUtc(timeZone: string): string {
  return isValidTimeZone(timeZone) ? timeZone : "UTC";
}

/* ----------------------------- Instants in a zone ----------------------------- */

export interface ZonedParts {
  year: number;
  /** 1–12 */
  month: number;
  /** 1–31 */
  day: number;
  /** 0–23 */
  hour: number;
  minute: number;
  second: number;
  /** 0 = Sunday … 6 = Saturday */
  weekday: number;
}

function instant(date: Date | number): number {
  const t = typeof date === "number" ? date : date.getTime();
  if (!Number.isFinite(t)) throw new RangeError("Invalid date");
  return t;
}

/** Epoch ms of a wall-clock reading taken as UTC; correct for years 0–99 too (Date.UTC would read them as 19xx). */
function utcMs(year: number, month: number, day: number, hour = 0, minute = 0, second = 0): number {
  const date = new Date(Date.UTC(2000, 0, 1, hour, minute, second));
  date.setUTCFullYear(year, month - 1, day);
  return date.getTime();
}

const partFormatters = new Map<string, Intl.DateTimeFormat>();

function partsFormatter(timeZone: string): Intl.DateTimeFormat {
  let formatter = partFormatters.get(timeZone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
    partFormatters.set(timeZone, formatter);
  }
  return formatter;
}

/** The wall-clock reading of an instant in a zone. Throws a RangeError for an invalid date. */
export function zonedParts(date: Date | number, timeZone: string): ZonedParts {
  const t = instant(date);
  const fields: Record<string, number> = {};
  for (const part of partsFormatter(zoneOrUtc(timeZone)).formatToParts(t)) {
    if (part.type !== "literal") fields[part.type] = Number(part.value);
  }
  const year = fields.year;
  const month = fields.month;
  const day = fields.day;
  return {
    year,
    month,
    day,
    // Some engines spell midnight "24" even with a 0–23 hour cycle.
    hour: fields.hour % 24,
    minute: fields.minute,
    second: fields.second,
    weekday: new Date(utcMs(year, month, day)).getUTCDay(),
  };
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

function keyOf(year: number, month: number, day: number): string {
  return `${String(year).padStart(4, "0")}-${pad2(month)}-${pad2(day)}`;
}

/** The calendar day ("YYYY-MM-DD") an instant falls on in a zone. */
export function zonedDayKey(date: Date | number, timeZone: string): string {
  const p = zonedParts(date, timeZone);
  return keyOf(p.year, p.month, p.day);
}

/** The hour (0–23) an instant falls in, in a zone. */
export function zonedHour(date: Date | number, timeZone: string): number {
  return zonedParts(date, timeZone).hour;
}

/** The zone's offset from UTC at an instant, in exact milliseconds. */
function offsetMs(t: number, timeZone: string): number {
  const p = zonedParts(t, timeZone);
  return utcMs(p.year, p.month, p.day, p.hour, p.minute, p.second) - Math.floor(t / 1000) * 1000;
}

/** The zone's UTC offset at an instant, in minutes (+345 for Kathmandu, −240 for New York in summer). */
export function utcOffsetMinutes(date: Date | number, timeZone: string): number {
  return Math.round(offsetMs(instant(date), zoneOrUtc(timeZone)) / 60_000);
}

const displayFormatters = new Map<string, Intl.DateTimeFormat>();

/**
 * Format an instant as read in a zone. Defaults to British English day-month
 * order, the order the reports use ("15 Jul 2026"); `options.timeZone` is
 * always replaced by `timeZone`.
 */
export function formatInZone(
  date: Date | number,
  timeZone: string,
  options: Intl.DateTimeFormatOptions = {},
  locale = "en-GB",
): string {
  const zone = zoneOrUtc(timeZone);
  const key = `${locale}|${zone}|${JSON.stringify(options)}`;
  let formatter = displayFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, { ...options, timeZone: zone });
    displayFormatters.set(key, formatter);
  }
  return formatter.format(instant(date));
}

/* ----------------------------- Day keys ----------------------------- */

function parseDayKey(value: string): { year: number; month: number; day: number } | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const back = new Date(utcMs(year, month, day));
  if (back.getUTCFullYear() !== year || back.getUTCMonth() !== month - 1 || back.getUTCDate() !== day) return null;
  return { year, month, day };
}

function requireDayKey(value: string): { year: number; month: number; day: number } {
  const parsed = parseDayKey(value);
  if (!parsed) throw new RangeError(`Not a calendar day: "${value}"`);
  return parsed;
}

/** UTC midnight of the date a key names: a plain calendar coordinate for day arithmetic. */
function keyEpoch(value: string): number {
  const { year, month, day } = requireDayKey(value);
  return utcMs(year, month, day);
}

function keyFromEpoch(ms: number): string {
  const d = new Date(ms);
  return keyOf(d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate());
}

/** Whether `value` is a real calendar date written "YYYY-MM-DD". */
export function isDayKey(value: unknown): value is string {
  return typeof value === "string" && parseDayKey(value) !== null;
}

/** The day key `days` calendar days after (or, negative, before) `dayKey`. Zone-free: a day is a day. */
export function addZonedDays(dayKey: string, days: number): string {
  return keyFromEpoch(keyEpoch(dayKey) + Math.trunc(days) * DAY_MS);
}

/** How many calendar days `fromKey` … `toKey` covers, both included (0 when `toKey` is earlier). */
export function countDays(fromKey: string, toKey: string): number {
  return Math.max(0, Math.round((keyEpoch(toKey) - keyEpoch(fromKey)) / DAY_MS) + 1);
}

/**
 * The instant a calendar day begins in a zone: local midnight, as a UTC Date.
 * Exact across DST: when a zone springs forward AT midnight (Santiago, Beirut,
 * Havana) the day begins at the transition, its first existing minute; when
 * midnight happens twice, at the first one. A day a zone skipped entirely
 * resolves to the start of the next day.
 */
export function startOfZonedDay(dayKey: string, timeZone: string): Date {
  const zone = zoneOrUtc(timeZone);
  const wall = keyEpoch(dayKey);
  // Midnight read as UTC, shifted by each offset in force around it: whichever
  // candidates read exactly 00:00:00 on that day are the day's midnights.
  const offsets = new Set([wall - DAY_MS, wall, wall + DAY_MS].map((t) => offsetMs(t, zone)));
  let best: number | null = null;
  for (const offset of offsets) {
    const t = wall - offset;
    const p = zonedParts(t, zone);
    if (keyOf(p.year, p.month, p.day) === dayKey && p.hour === 0 && p.minute === 0 && p.second === 0) {
      if (best === null || t < best) best = t;
    }
  }
  if (best !== null) return new Date(best);
  // No local midnight: find the first second whose local date is the day (local dates never run backwards).
  let lo = wall - 15 * HOUR_MS; // before the day starts even at UTC+14
  let hi = wall + 13 * HOUR_MS; // after it starts even at UTC−12
  while (hi - lo > 1000) {
    const mid = lo + Math.floor((hi - lo) / 2000) * 1000;
    if (zonedDayKey(mid, zone) >= dayKey) hi = mid;
    else lo = mid;
  }
  return new Date(hi);
}

export interface ZonedDayRange {
  /** Every day of the range, oldest first. */
  days: string[];
  /** Query bounds: the first day's start … the start of the day after the last. */
  from: Date;
  to: Date;
}

/** Whole calendar days `fromKey` … `toKey` (both included) in a zone, with their exact UTC bounds. */
export function zonedDayRange(fromKey: string, toKey: string, timeZone: string): ZonedDayRange {
  const count = countDays(fromKey, toKey);
  const days: string[] = [];
  for (let i = 0; i < count; i++) days.push(addZonedDays(fromKey, i));
  const end = count > 0 ? addZonedDays(toKey, 1) : fromKey;
  return { days, from: startOfZonedDay(fromKey, timeZone), to: startOfZonedDay(end, timeZone) };
}

/** The periods the reports step through; "week" is Monday–Sunday (the Work Report's presets). */
export type ZonedPeriodPreset = "week" | "7d" | "30d" | "month";

export interface ZonedPeriod extends ZonedDayRange {
  /** First and last day of the period, both included. */
  fromKey: string;
  toKey: string;
}

/**
 * A period's whole calendar days in a zone, `offset` periods back from the
 * one containing `now` (0 = current; positive offsets count as 0 — there is
 * no future to report). The current week and month run up to today; earlier
 * ones are complete. The same rules as the Work Report's periodBounds, with
 * "today" read in the reporting zone instead of the browser's.
 */
export function zonedPeriod(preset: ZonedPeriodPreset, now: Date | number, timeZone: string, offset = 0): ZonedPeriod {
  const back = Math.min(0, Math.trunc(Number.isFinite(offset) ? offset : 0));
  const today = zonedDayKey(now, timeZone);
  const span = (fromKey: string, toKey: string): ZonedPeriod => ({ fromKey, toKey, ...zonedDayRange(fromKey, toKey, timeZone) });
  switch (preset) {
    case "week": {
      const weekday = new Date(keyEpoch(today)).getUTCDay();
      const monday = addZonedDays(today, -((weekday + 6) % 7) + back * 7);
      return span(monday, back === 0 ? today : addZonedDays(monday, 6));
    }
    case "30d": {
      const end = addZonedDays(today, back * 30);
      return span(addZonedDays(end, -29), end);
    }
    case "month": {
      const { year, month } = requireDayKey(today);
      const first = new Date(Date.UTC(2000, 0, 1));
      first.setUTCFullYear(year, month - 1 + back, 1);
      const firstKey = keyFromEpoch(first.getTime());
      const nextFirst = new Date(first.getTime());
      nextFirst.setUTCMonth(nextFirst.getUTCMonth() + 1, 1);
      return span(firstKey, back === 0 ? today : keyFromEpoch(nextFirst.getTime() - DAY_MS));
    }
    default: {
      const end = addZonedDays(today, back * 7);
      return span(addZonedDays(end, -6), end);
    }
  }
}

export type DayRangeCheck =
  | { ok: true; days: number }
  | { ok: false; field: "from" | "to"; message: string };

/**
 * Check a from/to pair of day keys a person typed: both real dates, in order,
 * not ending after `today` (when given) and at most `maxDays` long (when given).
 */
export function validateDayRange(
  range: { from: string; to: string },
  rules: { today?: string; maxDays?: number } = {},
): DayRangeCheck {
  if (!isDayKey(range.from)) return { ok: false, field: "from", message: "Enter a start date." };
  if (!isDayKey(range.to)) return { ok: false, field: "to", message: "Enter an end date." };
  if (range.to < range.from) return { ok: false, field: "to", message: "The end date can't be before the start date." };
  if (rules.today && isDayKey(rules.today) && range.to > rules.today) {
    return { ok: false, field: "to", message: "The range can't end after today." };
  }
  const days = countDays(range.from, range.to);
  if (rules.maxDays !== undefined && days > rules.maxDays) {
    return { ok: false, field: "from", message: `Choose ${rules.maxDays} days or fewer — this range has ${days}.` };
  }
  return { ok: true, days };
}

/* ----------------------------- Labels ----------------------------- */

/** "15 Jul", "15 Jul 2026" or "Wed, 15 Jul 2026" for a day key. Deterministic (no locale data). */
export function formatDayKey(dayKey: string, options: { year?: boolean; weekday?: boolean } = {}): string {
  const { year, month, day } = requireDayKey(dayKey);
  const weekday = options.weekday ? `${WEEKDAYS[new Date(keyEpoch(dayKey)).getUTCDay()]}, ` : "";
  return `${weekday}${day} ${MONTHS[month - 1]}${options.year ? ` ${year}` : ""}`;
}

/**
 * A range of days: "1 Jul – 15 Jul 2026"; the year is written once unless the
 * range crosses New Year ("28 Dec 2025 – 3 Jan 2026"); one day reads "15 Jul 2026".
 */
export function formatDayRange(fromKey: string, toKey: string): string {
  if (fromKey === toKey) return formatDayKey(fromKey, { year: true });
  const sameYear = fromKey.slice(0, 4) === toKey.slice(0, 4);
  return `${formatDayKey(fromKey, { year: !sameYear })} – ${formatDayKey(toKey, { year: true })}`;
}

/** "UTC", "UTC+6", "UTC+5:45", "UTC-3:30". */
export function formatUtcOffset(minutes: number): string {
  const total = Number.isFinite(minutes) ? Math.round(minutes) : 0;
  if (total === 0) return "UTC";
  const sign = total > 0 ? "+" : "-";
  const abs = Math.abs(total);
  const rest = abs % 60;
  return `UTC${sign}${Math.floor(abs / 60)}${rest ? `:${pad2(rest)}` : ""}`;
}

/** The place a zone is named after: "Asia/Dhaka" → "Dhaka", "America/Argentina/Buenos_Aires" → "Buenos Aires". */
export function timeZoneCity(timeZone: string): string {
  const last = timeZone.split("/").pop() || "UTC";
  return last.replace(/_/g, " ");
}

/** A short label for a zone at an instant: "Dhaka (UTC+6)", "Kathmandu (UTC+5:45)", or "UTC". */
export function timeZoneLabel(timeZone: string, at: Date | number = Date.now()): string {
  const zone = zoneOrUtc(timeZone);
  if (zone === "UTC" || zone === "Etc/UTC") return "UTC";
  return `${timeZoneCity(zone)} (${formatUtcOffset(utcOffsetMinutes(at, zone))})`;
}
