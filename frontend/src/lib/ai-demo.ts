import {
  type AiAnswerDto,
  type AiDataQualityItemDto,
  type AiDataQualityKind,
  type AiDataQualityReviewDto,
  type AiDataQualityReviewInput,
  type AiFactRefDto,
  type AiFeatureInputs,
  type AiFeatureResults,
  type AiInsightFeatureId,
  type AiInsightItemDto,
  type AiMemberDaySummaryDto,
  type AiMemberDaySummaryInput,
  type AiNoteSuggestionDto,
  type AiNoteSuggestionsDto,
  type AiNoteSuggestionsInput,
  type AiPage,
  type AiPeriodSummaryDto,
  type AiPeriodSummaryInput,
  type AiReportExplanationDto,
  type AiReportExplanationInput,
  type AiResultMetaDto,
  type AiTaskDraftDto,
  type AiTaskDraftsDto,
  type AiTaskDraftsInput,
} from "./ai-contract";
import { AI_PROMPT_CHIPS, DEMO_NOTE, stableKey } from "./ai-assist";
import { friendlyAppName } from "./apps";
import { DAY_MS, NOW } from "./demo-clock";
import { activityPercent, dayKeyIn, durationMinutes, isActivityMeasured, minutesByProject, totalIdleMinutes, totalMinutes } from "./metrics";
import { isTrackedMember } from "./roles";
import { activeWorkspaceId, datasetFor, type Dataset } from "./tenant-data";
import { isWeekendDay } from "./report-math";
import { addZonedDays, formatDayKey, formatInZone, zonedDayKey } from "./time-zone";
import type { Activity, User } from "./types";

/* ============================================================================
 * AI ASSIST IN DEMO MODE — deterministic sample output computed from the demo
 * dataset (lib/tenant-data's datasets, the canonical lib/metrics figures), so
 * a demo shows what each card does with no network and no model.
 *
 * Every result carries status "demo" (or "no-data" when nothing was tracked)
 * and the demo note instead of the disclaimer; its figures are the dataset's
 * own, so they agree with the rest of the demo pages. A live session never
 * shows this output: the hooks choose it only when the session is not live,
 * and load this module on demand (a dynamic import), so a live deployment
 * never downloads it (.dependency-cruiser.cjs enforces that). The demo's AI
 * status lives apart, in lib/ai-demo-status.
 *
 * Wording follows the AI assist policy: describe, never judge; "activity %"
 * is input share, not output; nobody is ranked.
 * ========================================================================== */

/** Who the sample output is for: the viewer (self scopes) and the zone days are read in. */
export interface DemoAiContext {
  userId?: string;
  timeZone?: string;
}

/* ----------------------------- Formatting ----------------------------- */

/** "6 h 52 min", "45 min", "2 h": the server's display format. */
export function formatAiDuration(minutes: number): string {
  const total = Math.max(0, Math.round(minutes));
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

const percent = (part: number, whole: number) => Math.round((part / whole) * 100);

const clock = (at: number, timeZone: string) => formatInZone(at, timeZone, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

function hash(text: string): string {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

function demoMeta(featureId: string, input: unknown, empty: boolean, dataCategories: string[]): AiResultMetaDto {
  return {
    resultId: `demo-${featureId}-${hash(stableKey(input))}`,
    featureId,
    status: empty ? "no-data" : "demo",
    generatedAt: NOW.toISOString(),
    model: null,
    promptVersion: `${featureId}@demo`,
    dataCategories,
    disclaimer: DEMO_NOTE,
  };
}

/** Builds the F1, F2 … facts of one result as its sentences mention them. */
function factBook() {
  const facts: AiFactRefDto[] = [];
  return {
    facts,
    fact(label: string, display: string): AiFactRefDto {
      const ref = { id: `F${facts.length + 1}`, label, display };
      facts.push(ref);
      return ref;
    },
  };
}

const item = (kind: string, text: string, facts: AiFactRefDto[], routeId: string | null = null): AiInsightItemDto => ({ kind, text, facts, routeId });

/* ----------------------------- Dataset reads ----------------------------- */

/** Rows booked inside [from, to) (an activity belongs to the period holding its end), optionally narrowed. */
function rowsIn(ds: Dataset, from: number, to: number, filter: { userId?: string | null; projectId?: string | null } = {}): Activity[] {
  return ds.activities.filter((a) => {
    const at = Date.parse(a.endedAt);
    return (
      at >= from &&
      at < to &&
      (!filter.userId || a.userId === filter.userId) &&
      (!filter.projectId || a.projectId === filter.projectId)
    );
  });
}

/** The largest entry of a tally (ties: the first key in sort order); null when empty. */
function top(tally: Map<string, number>): [string, number] | null {
  let best: [string, number] | null = null;
  for (const [key, value] of [...tally].sort(([a], [b]) => a.localeCompare(b))) {
    if (!best || value > best[1]) best = [key, value];
  }
  return best;
}

/** Focused minutes per app (its canonical name) across the rows' windows. */
function appMinutes(rows: Activity[]): Map<string, number> {
  const tally = new Map<string, number>();
  for (const a of rows) {
    for (const w of a.activeWindows) {
      const app = friendlyAppName(w.appName);
      tally.set(app, (tally.get(app) ?? 0) + w.seconds / 60);
    }
  }
  return tally;
}

const projectTitle = (ds: Dataset, id: string) => ds.projects.find((p) => p.id === id)?.title ?? "Unassigned work";
const userOf = (ds: Dataset, id: string | null | undefined): User | undefined => ds.users.find((u) => u.id === id);

const UNMEASURED_CAVEAT = "Some blocks had no keyboard or mouse measurement; activity % leaves them out rather than counting them as zero.";
const ACTIVITY_MEANING = "the share of minutes with keyboard or mouse input, not a measure of output";

/** The project, app and activity findings every summary shares. */
function shareFindings(ds: Dataset, rows: Activity[], total: number, book: ReturnType<typeof factBook>, routes: { project: string | null; app: string | null; activity: string | null }) {
  const out: AiInsightItemDto[] = [];
  const project = top(minutesByProject(rows))!;
  const projectFact = book.fact("Time on the top project", formatAiDuration(project[1]));
  const shareFact = book.fact("Share of tracked time", `${percent(project[1], total)}%`);
  out.push(item("distribution", `${projectTitle(ds, project[0])} took the most time: ${projectFact.display} (${shareFact.display}).`, [projectFact, shareFact], routes.project));
  const app = top(appMinutes(rows));
  if (app) {
    const appFact = book.fact("Time in the top app", formatAiDuration(app[1]));
    out.push(item("distribution", `Most focused time was in ${app[0]}: ${appFact.display}.`, [appFact], routes.app));
  }
  const pct = activityPercent(rows);
  const pctFact = book.fact("Activity %", pct === null ? "not measured" : `${pct}%`);
  out.push(
    item(
      "coverage",
      pct === null ? "Keyboard and mouse input was not measured for this period." : `Activity was ${pctFact.display}: ${ACTIVITY_MEANING}.`,
      [pctFact],
      routes.activity,
    ),
  );
  return out;
}

/* ----------------------------- Features ----------------------------- */

function periodSummary(ds: Dataset, input: AiPeriodSummaryInput, context: DemoAiContext): AiPeriodSummaryDto {
  const from = Date.parse(input.from);
  const to = Date.parse(input.to);
  const self = input.scope === "self";
  const filter = { userId: self ? context.userId : null, projectId: input.projectId };
  const rows = rowsIn(ds, from, to, filter);
  const meta = demoMeta("period-summary", input, rows.length === 0, ["durations", "activity-percent", "project-names", "app-names"]);
  if (rows.length === 0) return { meta, headline: "Nothing was tracked for this period.", insights: [], nextSteps: [], caveats: [] };

  const book = factBook();
  const total = totalMinutes(rows);
  const previous = totalMinutes(rowsIn(ds, from - (to - from), from, filter));
  const totalFact = book.fact("Tracked time", formatAiDuration(total));
  const who = self ? "You" : input.projectId ? "This project's team" : "The team";
  let headline = `${who} tracked ${totalFact.display} in this period.`;
  if (previous > 0) {
    const delta = percent(total - previous, previous);
    const deltaFact = book.fact("Change vs the previous period", `${delta >= 0 ? "+" : ""}${delta}%`);
    headline = `${who} tracked ${totalFact.display} in this period, ${deltaFact.display} vs the period before.`;
  }

  const insights = shareFindings(ds, rows, total, book, {
    project: input.projectId ? null : "projects",
    app: "reports.apps-urls",
    activity: self ? "timesheet" : "reports.time-activity",
  });
  const timeZone = input.timeZone ?? context.timeZone ?? "UTC";
  const perDay = new Map<string, number>();
  for (const a of rows) {
    const day = dayKeyIn(a, timeZone);
    perDay.set(day, (perDay.get(day) ?? 0) + durationMinutes(a));
  }
  const busiest = top(perDay)!;
  const dayFact = book.fact("Busiest tracked day", formatDayKey(busiest[0], { weekday: true }));
  const dayTotalFact = book.fact("Tracked that day", formatAiDuration(busiest[1]));
  insights.push(item("rhythm", `The busiest day was ${dayFact.display}, with ${dayTotalFact.display}.`, [dayFact, dayTotalFact]));

  const caveats: string[] = [];
  if (rows.some((a) => !isActivityMeasured(a))) caveats.push(UNMEASURED_CAVEAT);
  if (to > NOW.getTime()) caveats.push("This period includes today, so its figures can still change.");
  return {
    meta,
    headline,
    insights: insights.slice(0, 5),
    nextSteps: [
      { label: "Open the weekly summary", routeId: "reports.weekly" },
      { label: "Open the timesheet", routeId: "timesheet" },
    ],
    caveats,
  };
}

/** The longest stretch of back-to-back blocks (gaps of 2 minutes or less merged), in minutes. */
function longestSpan(rows: Activity[]): number {
  let best = 0;
  let start = 0;
  let end = -Infinity;
  for (const a of [...rows].sort((x, y) => Date.parse(x.startedAt) - Date.parse(y.startedAt))) {
    const s = Date.parse(a.startedAt);
    const e = Date.parse(a.endedAt);
    if (s - end > 2 * 60_000) start = s;
    end = Math.max(end, e);
    best = Math.max(best, (end - start) / 60_000);
  }
  return best;
}

function memberDaySummary(ds: Dataset, input: AiMemberDaySummaryInput, context: DemoAiContext): AiMemberDaySummaryDto {
  const userId = input.userId ?? context.userId;
  const timeZone = input.timeZone ?? context.timeZone ?? "UTC";
  const rows = ds.activities.filter((a) => a.userId === userId && dayKeyIn(a, timeZone) === input.date);
  const meta = demoMeta("member-day-summary", input, rows.length === 0, ["durations", "times", "activity-percent", "project-names", "app-names"]);
  if (rows.length === 0) return { meta, headline: "Nothing was tracked on this day.", summary: "", highlights: [], caveats: [] };

  const book = factBook();
  const self = userId === context.userId;
  const name = self ? "You" : (userOf(ds, userId)?.name ?? "This member");
  const total = totalMinutes(rows);
  const totalFact = book.fact("Tracked time", formatAiDuration(total));
  const dayFact = book.fact("Day", formatDayKey(input.date, { weekday: true }));
  const first = book.fact("First activity", clock(Math.min(...rows.map((a) => Date.parse(a.startedAt))), timeZone));
  const last = book.fact("Last activity", clock(Math.max(...rows.map((a) => Date.parse(a.endedAt))), timeZone));
  const [projectId] = top(minutesByProject(rows))!;
  const app = top(appMinutes(rows));
  const spanFact = book.fact("Longest continuous span", formatAiDuration(longestSpan(rows)));
  const idleFact = book.fact("Idle time", formatAiDuration(totalIdleMinutes(rows)));
  const pct = activityPercent(rows);

  const highlights: AiInsightItemDto[] = [
    item("projects", `Most time went to ${projectTitle(ds, projectId)}.`, [totalFact]),
    item("focus", `The longest continuous span was ${spanFact.display}.`, [spanFact]),
    item("breaks", `Idle time came to ${idleFact.display}; idle minutes can be meetings, calls or reading.`, [idleFact]),
  ];
  if (app) {
    const appFact = book.fact("Time in the top app", formatAiDuration(app[1]));
    highlights.splice(1, 0, item("apps", `Most focused time was in ${app[0]}: ${appFact.display}.`, [appFact]));
  }
  if (pct !== null) {
    const pctFact = book.fact("Activity %", `${pct}%`);
    highlights.push(item("rhythm", `Activity was ${pctFact.display}: ${ACTIVITY_MEANING}.`, [pctFact]));
  }
  return {
    meta,
    headline: `${name} tracked ${totalFact.display} on ${dayFact.display}.`,
    summary: `Work ran from ${first.display} to ${last.display}. Most time went to ${projectTitle(ds, projectId)}${app ? ` in ${app[0]}` : ""}.`,
    highlights: highlights.slice(0, 5),
    caveats: rows.some((a) => !isActivityMeasured(a)) ? [UNMEASURED_CAVEAT] : [],
  };
}

const REPORT_CAVEATS: Record<string, string> = {
  attendance: "\"Absent\" here means no tracked time, not a missed shift.",
  "apps-urls": "App time counts the window in focus; background apps are not included.",
  devices: "Device figures come from the agents' own reports.",
};

function reportExplanation(ds: Dataset, input: AiReportExplanationInput): AiReportExplanationDto {
  const rows = rowsIn(ds, Date.parse(input.from), Date.parse(input.to), input);
  const meta = demoMeta("report-explanation", input, rows.length === 0, ["durations", "activity-percent", "project-names", "app-names"]);
  if (rows.length === 0) return { meta, summary: "Nothing was tracked for this report's period.", keyFindings: [], caveats: [], questionsToExplore: [] };

  const book = factBook();
  const total = totalMinutes(rows);
  const totalFact = book.fact("Tracked time", formatAiDuration(total));
  const members = book.fact("Members with tracked time", String(new Set(rows.map((a) => a.userId)).size));
  const days = book.fact("Days with tracked time", String(new Set(rows.map((a) => a.endedAt.slice(0, 10))).size));
  const keyFindings = shareFindings(ds, rows, total, book, { project: "reports.projects", app: "reports.apps-urls", activity: "reports.productivity" });
  const caveats = [REPORT_CAVEATS[input.reportSlug], rows.some((a) => !isActivityMeasured(a)) ? UNMEASURED_CAVEAT : undefined].filter(
    (c): c is string => !!c,
  );
  return {
    meta,
    summary: `This report covers ${totalFact.display} of tracked time from ${members.display} members over ${days.display} days.`,
    keyFindings,
    caveats,
    questionsToExplore: [
      "How does the split between projects compare with the period before?",
      "Which apps take most of the time on the busiest days?",
    ],
  };
}

/* ----------------------------- Data quality ----------------------------- */

/** One data-quality finding before it is written up (the server's DataQualityRules, v1 subset). */
export interface DemoDataQualityCandidate {
  kind: AiDataQualityKind;
  userId: string;
  /** The measured value the rule tripped on, already formatted. */
  value: string;
}

/** "0.1.4" < "0.2.0" on the numeric prefix; null when either isn't a version. */
function compareVersions(a: string, b: string): number | null {
  const parse = (v: string) => /^\d+(\.\d+)*/.exec(v)?.[0].split(".").map(Number) ?? null;
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return null;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

/** The longest run of weekdays without a tracked block, from the member's first tracked day to `lastDay`. */
function longestWeekdayGap(days: Set<string>, lastDay: string): number {
  let best = 0;
  let run = 0;
  for (let day = [...days].sort()[0]; day <= lastDay; day = addZonedDays(day, 1)) {
    if (isWeekendDay(day)) continue;
    run = days.has(day) ? 0 : run + 1;
    best = Math.max(best, run);
  }
  return best;
}

/**
 * The data-quality candidates in a set of rows: 3 or more weekdays in a row
 * without tracked time after a member's first tracked day, unmeasured input
 * above 30 %, clock skew above 5 minutes, software input in 3 or more blocks,
 * and (team scope) members who never tracked in the range or whose agent is
 * older than the newest one seen for its platform. Days are read in
 * `timeZone` (UTC by default) and end at `to` or the demo's now, whichever is
 * earlier. Never a judgement of anyone.
 */
export function demoDataQualityCandidates(
  rows: Activity[],
  members: User[],
  options: { scope: "team" | "self"; to: number; timeZone?: string },
): DemoDataQualityCandidate[] {
  const out: DemoDataQualityCandidate[] = [];
  const timeZone = options.timeZone ?? "UTC";
  const lastDay = zonedDayKey(Math.min(options.to, NOW.getTime()) - 1, timeZone);
  const byUser = new Map<string, Activity[]>();
  for (const a of rows) byUser.set(a.userId, [...(byUser.get(a.userId) ?? []), a]);

  for (const [userId, list] of [...byUser].sort(([a], [b]) => a.localeCompare(b))) {
    const gap = longestWeekdayGap(new Set(list.map((a) => dayKeyIn(a, timeZone))), lastDay);
    if (gap >= 3) out.push({ kind: "data-gap", userId, value: `${gap} weekdays` });
    const total = totalMinutes(list);
    const unmeasured = totalMinutes(list.filter((a) => !isActivityMeasured(a)));
    if (total > 0 && unmeasured / total > 0.3) out.push({ kind: "unmeasured-activity", userId, value: `${percent(unmeasured, total)}%` });
    const skew = Math.max(0, ...list.map((a) => Math.abs(a.clockSkewSeconds ?? 0)));
    if (skew > 300) out.push({ kind: "clock-skew", userId, value: formatAiDuration(skew / 60) });
    const injected = list.filter((a) => (a.injectedInputEvents ?? 0) > 0).length;
    if (injected >= 3) out.push({ kind: "software-input", userId, value: String(injected) });
  }
  if (options.scope === "self") return out;

  const newest = new Map<string, string>();
  const latest = new Map<string, Activity>();
  for (const a of rows) {
    if (!a.platform || !a.agentVersion) continue;
    const seen = newest.get(a.platform);
    if (!seen || (compareVersions(a.agentVersion, seen) ?? 0) > 0) newest.set(a.platform, a.agentVersion);
    const key = `${a.userId}|${a.platform}`;
    const prev = latest.get(key);
    if (!prev || Date.parse(a.endedAt) > Date.parse(prev.endedAt)) latest.set(key, a);
  }
  for (const a of [...latest.values()].sort((x, y) => x.userId.localeCompare(y.userId))) {
    if ((compareVersions(a.agentVersion!, newest.get(a.platform!)!) ?? 0) < 0) {
      out.push({ kind: "outdated-agent", userId: a.userId, value: a.agentVersion! });
    }
  }
  for (const m of members) {
    const joined = Date.parse(m.joinedAt);
    if (!isTrackedMember(m) || byUser.has(m.id) || !(joined <= options.to - 3 * DAY_MS)) continue;
    out.push({ kind: "never-tracked", userId: m.id, value: "this period" });
  }
  return out;
}

const DQ_COPY: Record<AiDataQualityKind, { severity: "info" | "notice"; title: string; explain: (v: string) => string; reasons: string[]; actionId: string | null; route: string | null }> = {
  "data-gap": {
    severity: "notice",
    title: "Gap in tracked days",
    explain: (v) => `No time was tracked for ${v} in a row.`,
    reasons: ["Leave or a day off", "The agent wasn't running"],
    actionId: "check-agent-running",
    route: "download",
  },
  "clock-skew": {
    severity: "notice",
    title: "Device clock is off",
    explain: (v) => `A device's clock was out by up to ${v}, which can shift block times.`,
    reasons: ["The clock isn't synced automatically", "A recent time zone change"],
    actionId: "sync-device-clock",
    route: null,
  },
  "unmeasured-activity": {
    severity: "info",
    title: "Input not measured",
    explain: (v) => `Keyboard and mouse input wasn't measured for ${v} of tracked time.`,
    reasons: ["The project doesn't collect input counts", "The operating system denied the permission"],
    actionId: "check-os-permissions",
    route: "download",
  },
  "software-input": {
    severity: "info",
    title: "Software-generated input",
    explain: (v) => `${v} blocks had input generated by software, which is left out of activity %.`,
    reasons: ["Remote desktop tools", "Accessibility tools", "Automation scripts"],
    actionId: null,
    route: null,
  },
  "never-tracked": {
    severity: "notice",
    title: "No tracked time yet",
    explain: (v) => `This member hasn't tracked any time in ${v}.`,
    reasons: ["The desktop agent isn't installed yet", "They joined recently"],
    actionId: "send-install-link",
    route: "download",
  },
  "outdated-agent": {
    severity: "info",
    title: "Agent update available",
    explain: (v) => `This device runs agent ${v}, older than the newest one in the workspace.`,
    reasons: ["Automatic updates are off", "The device hasn't restarted the agent"],
    actionId: "update-agent",
    route: "download",
  },
};

function dataQualityReview(ds: Dataset, input: AiDataQualityReviewInput, context: DemoAiContext): AiDataQualityReviewDto {
  const self = input.scope === "self";
  const to = Date.parse(input.to);
  const rows = rowsIn(ds, Date.parse(input.from), to, { userId: self ? context.userId : null, projectId: input.projectId });
  const timeZone = input.timeZone ?? context.timeZone;
  const candidates = demoDataQualityCandidates(rows, self ? [] : ds.users, { scope: input.scope, to, timeZone }).filter(
    (c) => !input.kinds?.length || input.kinds.includes(c.kind),
  );
  const items: AiDataQualityItemDto[] = candidates
    .map((c) => ({ c, copy: DQ_COPY[c.kind] }))
    .sort((a, b) => (a.copy.severity === b.copy.severity ? 0 : a.copy.severity === "notice" ? -1 : 1))
    .slice(0, 5)
    .map(({ c, copy }) => ({
      candidateId: `${c.kind}:${c.userId}`,
      kind: c.kind,
      severity: copy.severity,
      title: copy.title,
      explanation: copy.explain(c.value),
      possibleReasons: copy.reasons,
      actionId: copy.actionId,
      actionRouteId: copy.route,
      subjectName: c.userId === context.userId ? "you" : (userOf(ds, c.userId)?.name ?? null),
      subjectUserId: c.userId,
      facts: [{ id: "F1", label: copy.title, display: c.value }],
    }));
  return {
    meta: demoMeta("data-quality-review", input, items.length === 0, ["durations", "agent-info"]),
    items,
    candidateCount: candidates.length,
  };
}

function noteSuggestions(ds: Dataset, input: AiNoteSuggestionsInput): AiNoteSuggestionsDto {
  const block = ds.activities.find((a) => a.id === input.activityId);
  const meta = demoMeta("note-suggestions", input, !block, ["durations", "project-names", "app-names"]);
  if (!block) return { meta, suggestions: [] };
  const project = projectTitle(ds, block.projectId);
  const app = top(appMinutes([block]));
  const suggestions: AiNoteSuggestionDto[] = [];
  if (app) suggestions.push({ text: `Worked on ${project} in ${app[0]}`, basis: "apps" });
  suggestions.push({ text: `Worked on ${project}`, basis: "project" });
  const start = Date.parse(block.startedAt);
  const previous = ds.activities.find(
    (a) => a.id !== block.id && a.userId === block.userId && a.projectId === block.projectId && !!a.note && Date.parse(a.endedAt) <= start && start - Date.parse(a.endedAt) <= 2 * 60 * 60_000,
  );
  if (previous?.note) suggestions.push({ text: previous.note.slice(0, 200), basis: "previous-note" });
  return { meta, suggestions: suggestions.slice(0, 3) };
}

const TASK_TEMPLATES: { title: (goal: string) => string; description: (goal: string, project: string) => string; rationale: string }[] = [
  {
    title: (g) => `Define the scope: ${g}`,
    description: (g, p) => `List what "${g}" covers for ${p}, and what stays out of scope.`,
    rationale: "A clear scope keeps the other tasks small.",
  },
  { title: () => "List the open questions", description: (g) => `Collect what still needs an answer before work on "${g}" starts.`, rationale: "Unknowns found early are cheaper." },
  { title: () => "Draft a first version", description: (_g, p) => `Produce a first working version within ${p}.`, rationale: "A draft gives the review something concrete." },
  { title: () => "Review the draft with the team", description: () => "Walk through the draft and note what needs to change.", rationale: "A second view catches gaps." },
  { title: () => "Apply the review feedback", description: () => "Work through the notes from the review.", rationale: "Closes the loop on the review." },
  { title: () => "Check the result against the goal", description: (g) => `Confirm the outcome meets "${g}".`, rationale: "Keeps the goal in view." },
  { title: () => "Write up what changed", description: () => "A short note on what was done and why.", rationale: "Helps whoever picks this up next." },
  { title: () => "Plan the follow-up work", description: () => "Note anything left for a later iteration.", rationale: "Nothing gets lost after the goal is met." },
];

function taskDrafts(ds: Dataset, input: AiTaskDraftsInput): AiTaskDraftsDto {
  const project = ds.projects.find((p) => p.id === input.projectId);
  const goal = input.goal.trim();
  const meta = demoMeta("task-drafts", input, !project || goal === "", ["project-names", "task-titles"]);
  if (!project || goal === "") return { meta, drafts: [] };
  const count = Math.min(8, Math.max(1, Math.round(input.maxTasks ?? 5)));
  const drafts: AiTaskDraftDto[] = TASK_TEMPLATES.slice(0, count).map((t) => ({
    title: t.title(goal).slice(0, 256),
    description: t.description(goal, project.title).slice(0, 600),
    rationale: t.rationale,
    possibleDuplicateOfTaskId: null,
  }));
  return { meta, drafts };
}

/**
 * The sample result of an insight feature, computed from a demo workspace's
 * dataset. `context` names the viewer (for self scopes) and the zone days are
 * read in when the input doesn't say.
 */
export function demoAiResult<F extends AiInsightFeatureId>(featureId: F, input: AiFeatureInputs[F], workspaceId: string, context: DemoAiContext = {}): AiFeatureResults[F] {
  const ds = datasetFor(workspaceId);
  const results: { [K in AiInsightFeatureId]: () => AiFeatureResults[K] } = {
    "period-summary": () => periodSummary(ds, input as AiPeriodSummaryInput, context),
    "member-day-summary": () => memberDaySummary(ds, input as AiMemberDaySummaryInput, context),
    "report-explanation": () => reportExplanation(ds, input as AiReportExplanationInput),
    "data-quality-review": () => dataQualityReview(ds, input as AiDataQualityReviewInput, context),
    "note-suggestions": () => noteSuggestions(ds, input as AiNoteSuggestionsInput),
    "task-drafts": () => taskDrafts(ds, input as AiTaskDraftsInput),
  };
  return results[featureId]() as AiFeatureResults[F];
}

/* ----------------------------- Ask AI (prompt chips only) ----------------------------- */

const lines = (head: string, items: string[]) => [head, ...items.map((t) => `- ${t}`)].join("\n");

/**
 * The demo's answer to one of the suggested questions (AI_PROMPT_CHIPS).
 * Anything else gets a clarification listing the page's questions: the demo
 * composer is disabled, so only chips reach it.
 */
export function demoAnswer(chipId: string, page: AiPage, workspaceId: string = activeWorkspaceId(), context: DemoAiContext = {}): AiAnswerDto {
  const ds = datasetFor(workspaceId);
  const to = NOW.getTime();
  const week = rowsIn(ds, to - 7 * DAY_MS, to + 1);
  const today = rowsIn(ds, to - DAY_MS, to + 1);
  const book = factBook();
  const answer = (text: string, sources: AiAnswerDto["sources"] = [], isClarification = false): AiAnswerDto => ({
    meta: demoMeta("ask", { chipId, page }, false, isClarification ? [] : ["durations", "project-names", "app-names", "help-content"]),
    conversationId: "demo-conversation",
    answer: text,
    sources,
    facts: book.facts,
    isClarification,
  });
  const projectList = (rows: Activity[]) =>
    [...minutesByProject(rows)]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .slice(0, 3)
      .map(([id, minutes]) => `${projectTitle(ds, id)}: ${book.fact(`Time on ${projectTitle(ds, id)}`, formatAiDuration(minutes)).display}`);

  switch (chipId) {
    case "week-overview": {
      if (week.length === 0) return answer("Nothing was tracked in the last 7 days.");
      const total = book.fact("Tracked in the last 7 days", formatAiDuration(totalMinutes(week)));
      return answer(lines(`The team tracked ${total.display} in the last 7 days. Most time went to:`, projectList(week)), [
        { label: "Weekly summary", routeId: "reports.weekly", routeArg: null },
      ]);
    }
    case "top-projects":
      if (week.length === 0) return answer("Nothing was tracked in the last 7 days.");
      return answer(lines("These projects took the most time in the last 7 days:", projectList(week)), [{ label: "Projects", routeId: "projects", routeArg: null }]);
    case "day-overview": {
      const mine = context.userId ? today.filter((a) => a.userId === context.userId) : today;
      if (mine.length === 0) return answer("Nothing was tracked today yet.");
      const total = book.fact("Tracked today", formatAiDuration(totalMinutes(mine)));
      return answer(lines(`${total.display} was tracked today. By project:`, projectList(mine)), [{ label: "Monitor", routeId: "monitor", routeArg: null }]);
    }
    case "top-apps": {
      const apps = [...appMinutes(today.length ? today : week)].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 3);
      if (apps.length === 0) return answer("No app time was recorded recently.");
      return answer(
        lines(
          `The apps with the most focused time ${today.length ? "today" : "in the last 7 days"}:`,
          apps.map(([app, minutes]) => `${app}: ${book.fact(`Time in ${app}`, formatAiDuration(minutes)).display}`),
        ),
        [{ label: "Apps & Websites", routeId: "reports.apps-urls", routeArg: null }],
      );
    }
    case "data-gaps": {
      const found = demoDataQualityCandidates(week, ds.users, { scope: "team", to });
      const count = book.fact("Things worth a look", String(found.length));
      return answer(
        found.length === 0
          ? "Nothing in this week's tracking looks unusual."
          : lines(
              `${count.display} ${found.length === 1 ? "thing in this week's tracking is" : "things in this week's tracking are"} worth a look:`,
              found.slice(0, 5).map((c) => DQ_COPY[c.kind].title),
            ),
        [{ label: "Team", routeId: "team", routeArg: null }],
      );
    }
    case "help-install":
      return answer(
        lines("Open Download in the sidebar and pick your system:", [
          "Windows: run the installer, then sign in with your work account",
          "macOS: unzip the app into Applications and open it",
          "Linux: install the .deb or .rpm package, or unpack the tarball",
        ]),
        [{ label: "Download", routeId: "download", routeArg: null }],
      );
    case "help-not-measured":
      return answer(
        "\"Not measured\" means the agent couldn't count keyboard and mouse input for that time - the project doesn't collect it, or the operating system denied the permission. It is never counted as zero.",
        [{ label: "Download", routeId: "download", routeArg: null }],
      );
    default:
      return answer(
        lines(
          "In the demo, Ask AI answers the suggested questions only. Try one of these:",
          AI_PROMPT_CHIPS[page].map((c) => c.label),
        ),
        [],
        true,
      );
  }
}
