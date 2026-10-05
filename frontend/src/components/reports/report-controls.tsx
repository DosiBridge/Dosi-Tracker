"use client";

import { useId, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { CalendarRange, ChevronLeft, ChevronRight, Globe } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Popover } from "@/components/ui/popover";
import { projects, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { ALL } from "@/lib/report-math";
import { MAX_REPORT_RANGE_DAYS } from "@/lib/api-contract";
import { referenceNow } from "@/lib/live-session";
import {
  addZonedDays,
  browserTimeZone,
  countDays,
  formatDayRange,
  isDayKey,
  timeZoneLabel,
  validateDayRange,
  zonedDayKey,
} from "@/lib/time-zone";
import { PERIOD_PRESETS, type PeriodPreset } from "@/lib/work-report";
import { cn } from "@/lib/utils";

/* ============================================================================
 * Controls shared by the period-based reports (Work Report, Work Analytics):
 * a segmented toggle, and the toolbar with one-click periods, ‹ › stepping,
 * and the project / member scope.
 *
 * Opt-in extras: a "Custom" period (from/to days with Apply and validation,
 * for pay periods and the like), the period's dates and the reporting time
 * zone. A report that passes none of them renders exactly as before.
 * ========================================================================== */

export type { PeriodPreset };

/** A period preset, or "custom": a range of days the viewer picked. */
export type ToolbarPeriod = PeriodPreset | "custom";

/**
 * A custom period: whole calendar days of the reporting zone, both ends
 * included ("YYYY-MM-DD"). Its query bounds are zonedDayRange(from, to, zone).
 */
export interface CustomRange {
  from: string;
  to: string;
}

/** Where the viewer's hours format (7h 30m / 7:30 / 7.50) is remembered; shared by the reports. */
export const HOURS_FORMAT_KEY = "dosi-work-report-hours";

export interface SegmentOption<T extends string> {
  key: T;
  label: string;
  icon?: LucideIcon;
  description?: string;
}

/** One button of a segmented toggle, pressed or not. */
function segmentClass(active: boolean): string {
  return cn(
    "inline-flex h-8 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg px-3 text-xs font-medium transition-all",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
    // The muted text colour falls under 4.5:1 on the muted track; a softened foreground clears it in both themes.
    active ? "bg-card text-foreground shadow-sm" : "text-foreground/75 hover:text-foreground",
  );
}

/** A row of toggle buttons; exactly one is pressed. `children` go at the end of the row (e.g. a "Custom" button). */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
  className,
  children,
}: {
  label: string;
  options: readonly SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div role="group" aria-label={label} className={cn("inline-flex max-w-full items-center overflow-x-auto rounded-xl bg-muted p-1", className)}>
      {options.map((o) => {
        const Icon = o.icon;
        const active = o.key === value;
        return (
          <button
            key={o.key}
            type="button"
            aria-pressed={active}
            title={o.description}
            onClick={() => onChange(o.key)}
            className={segmentClass(active)}
          >
            {Icon && <Icon aria-hidden className={cn("h-3.5 w-3.5", active && "text-primary")} />}
            <span className="tabular-nums">{o.label}</span>
          </button>
        );
      })}
      {children}
    </div>
  );
}

/* ----------------------------- Custom range ----------------------------- */

/**
 * The from/to form behind "Custom": two dates, a preview of the span, the
 * zone the days belong to, and Apply — which only accepts real dates, in
 * order, not past today and at most `maxDays` long (the server's limit).
 */
export function CustomRangeForm({
  initial,
  today,
  maxDays = MAX_REPORT_RANGE_DAYS,
  timeZone,
  onApply,
  onCancel,
}: {
  /** The range the form starts with (default: the 7 days ending today). */
  initial?: CustomRange | null;
  /** Today in the reporting zone; default: read from the session clock in `timeZone`. */
  today?: string;
  maxDays?: number;
  /** The reporting zone the days belong to (shown; default the browser's). */
  timeZone?: string;
  onApply: (range: CustomRange) => void;
  onCancel: () => void;
}) {
  const id = useId();
  const fromId = `${id}from`;
  const toId = `${id}to`;
  const errorId = `${id}error`;
  // Read once when the form opens: the session's clock (the demo's frozen one in demo mode).
  const [opened] = useState(() => referenceNow());
  const zone = timeZone ?? browserTimeZone();
  const todayKey = today && isDayKey(today) ? today : zonedDayKey(opened, zone);
  const start = initial && isDayKey(initial.from) && isDayKey(initial.to) ? initial : { from: addZonedDays(todayKey, -6), to: todayKey };
  const [from, setFrom] = useState(start.from);
  const [to, setTo] = useState(start.to);
  const [error, setError] = useState<{ field: "from" | "to"; message: string } | null>(null);

  const check = validateDayRange({ from, to }, { today: todayKey, maxDays });

  function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    // The form sits in a portal; React would still bubble its submit to forms around the toolbar.
    event.stopPropagation();
    if (!check.ok) {
      setError({ field: check.field, message: check.message });
      document.getElementById(check.field === "from" ? fromId : toId)?.focus();
      return;
    }
    onApply({ from, to });
  }

  const invalid = (field: "from" | "to") => error?.field === field;
  const label = "block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground";

  return (
    <form onSubmit={submit} noValidate className="space-y-3">
      <div>
        <p className="text-sm font-semibold">Custom range</p>
        <p className="text-xs text-muted-foreground">Whole days, first and last included.</p>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div className="min-w-0 space-y-1">
          <label htmlFor={fromId} className={label}>
            From
          </label>
          <Input
            id={fromId}
            type="date"
            value={from}
            max={isDayKey(to) && to < todayKey ? to : todayKey}
            aria-invalid={invalid("from") || undefined}
            aria-describedby={invalid("from") ? errorId : undefined}
            onChange={(e) => {
              setFrom(e.target.value);
              setError(null);
            }}
            className={cn("h-9 px-2", invalid("from") && "border-danger")}
          />
        </div>
        <div className="min-w-0 space-y-1">
          <label htmlFor={toId} className={label}>
            To
          </label>
          <Input
            id={toId}
            type="date"
            value={to}
            min={isDayKey(from) ? from : undefined}
            max={todayKey}
            aria-invalid={invalid("to") || undefined}
            aria-describedby={invalid("to") ? errorId : undefined}
            onChange={(e) => {
              setTo(e.target.value);
              setError(null);
            }}
            className={cn("h-9 px-2", invalid("to") && "border-danger")}
          />
        </div>
      </div>
      {error ? (
        <p id={errorId} role="alert" className="text-xs font-medium text-danger-on-tint">
          {error.message}
        </p>
      ) : (
        <p aria-live="polite" className="text-xs text-muted-foreground">
          {check.ok ? `${check.days} ${check.days === 1 ? "day" : "days"} · ${formatDayRange(from, to)}` : "Choose the first and last day."}
        </p>
      )}
      <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground" title={zone}>
        <Globe aria-hidden className="h-3.5 w-3.5 shrink-0" />
        <span>Days in {timeZoneLabel(zone, opened)}</span>
      </p>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" size="sm">
          Apply
        </Button>
      </div>
    </form>
  );
}

/* ----------------------------- Toolbar ----------------------------- */

/** One-click periods, ‹ › stepping back through earlier ones, and the project/member scope. */
export function PeriodToolbar({
  preset,
  onPreset,
  title,
  offset,
  onStep,
  projectId,
  onProject,
  memberId,
  onMember,
  liveWorkerId,
  extra,
  allowCustom = false,
  range = null,
  onRange,
  suggestedRange,
  today,
  maxDays = MAX_REPORT_RANGE_DAYS,
  timeZone,
  spanLabel,
}: {
  /** The chosen preset, or "custom" (with `range`) when the viewer picked their own days. */
  preset: ToolbarPeriod;
  onPreset: (p: PeriodPreset) => void;
  /** "This week", "Last month"… */
  title: string;
  /** 0 = the current period, −1 the one before… */
  offset: number;
  onStep: (by: number) => void;
  projectId: string;
  onProject: (v: string) => void;
  memberId: string;
  onMember: (v: string) => void;
  /** A live worker's id: they are only offered themselves (the backend scopes their data anyway). */
  liveWorkerId: string | null;
  /** Further controls at the end (e.g. the hours format). */
  extra?: React.ReactNode;
  /** Offer "Custom" after the presets (needs `onRange`). */
  allowCustom?: boolean;
  /** The applied custom range, shown while `preset` is "custom". */
  range?: CustomRange | null;
  /** Apply a validated custom range: the caller keeps it and switches `preset` to "custom". */
  onRange?: (range: CustomRange) => void;
  /** What the custom form starts with when no range is applied yet (e.g. the current period's days). */
  suggestedRange?: CustomRange | null;
  /** Today in the reporting zone (a custom range can't end later); default: the session clock. */
  today?: string;
  /** Longest custom range, in days (default: the server's 366). */
  maxDays?: number;
  /** The reporting time zone, shown next to the period ("Dhaka (UTC+6)"). */
  timeZone?: string;
  /** The period's dates ("29 Sep – 5 Oct 2026"), shown next to a preset's title. */
  spanLabel?: string;
}) {
  const members = trackedMembers(users).filter((u) => !liveWorkerId || u.id === liveWorkerId);
  // Archived projects still hold tracked time, so they stay choosable — listed after the active ones and labelled.
  const projectOptions = [...projects].sort((a, b) => Number(a.archived) - Number(b.archived));
  // The instant the zone's UTC offset is read at (fixed for the toolbar's life; render stays pure).
  const [labelAt] = useState(() => referenceNow());
  const canCustom = allowCustom && !!onRange;
  const custom = preset === "custom";
  const applied = range && isDayKey(range.from) && isDayKey(range.to) && range.from <= range.to ? range : null;
  const customDays = custom && applied ? countDays(applied.from, applied.to) : 0;
  // Under the controls: how long a custom range is, or a preset's dates; then the zone.
  const detail = custom ? (applied ? `${customDays} ${customDays === 1 ? "day" : "days"}` : null) : (spanLabel ?? null);

  return (
    <div className="no-print flex flex-col gap-3 rounded-2xl border border-border bg-card p-3 card-elev sm:p-4 xl:flex-row xl:items-center xl:justify-between">
      <div className="flex flex-wrap items-center gap-2">
        <Segmented<ToolbarPeriod>
          label="Period"
          options={PERIOD_PRESETS}
          value={preset}
          onChange={(next) => {
            if (next !== "custom") onPreset(next);
          }}
        >
          {canCustom && (
            <Popover
              label="Custom range"
              className="w-80 p-4"
              trigger={(props, { open }) => (
                <button {...props} aria-pressed={custom} title="Choose the first and last day" className={cn(segmentClass(custom || open), "relative")}>
                  <CalendarRange aria-hidden className={cn("h-3.5 w-3.5", custom && "text-primary")} />
                  {/* On the narrowest phones the calendar alone keeps the whole period row on screen; the name stays "Custom". */}
                  <span className="sr-only min-[410px]:not-sr-only">Custom</span>
                </button>
              )}
            >
              {({ close }) => (
                <CustomRangeForm
                  initial={applied ?? suggestedRange}
                  today={today}
                  maxDays={maxDays}
                  timeZone={timeZone}
                  onApply={(next) => {
                    onRange?.(next);
                    close();
                  }}
                  onCancel={() => close()}
                />
              )}
            </Popover>
          )}
        </Segmented>
        {custom ? (
          <div className="inline-flex items-center rounded-xl border border-border p-1">
            <span aria-live="polite" className="min-w-[7.5rem] px-2 py-1.5 text-center text-sm font-semibold tabular-nums">
              {applied ? formatDayRange(applied.from, applied.to) : title}
            </span>
          </div>
        ) : (
          <>
            <div className="inline-flex items-center gap-0.5 rounded-xl border border-border p-1">
              <button
                type="button"
                aria-label="Previous period"
                onClick={() => onStep(-1)}
                className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ChevronLeft className="h-4 w-4" />
              </button>
              <span aria-live="polite" className="min-w-[7.5rem] px-1 text-center text-sm font-semibold">
                {title}
              </span>
              <button
                type="button"
                aria-label="Next period"
                disabled={offset >= 0}
                onClick={() => onStep(1)}
                className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-35"
              >
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
            {offset < 0 && (
              <button
                type="button"
                onClick={() => onStep(-offset)}
                className="rounded-lg px-2 py-1 text-xs font-medium text-primary-strong hover:bg-accent"
              >
                Back to today
              </button>
            )}
          </>
        )}
        {(detail || timeZone) && (
          <p className="flex basis-full flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
            {detail && <span className="tabular-nums">{detail}</span>}
            {detail && timeZone && <span aria-hidden>·</span>}
            {timeZone && (
              <span className="inline-flex items-center gap-1" title={timeZone}>
                <Globe aria-hidden className="h-3.5 w-3.5 shrink-0" />
                <span className="sr-only">Reporting time zone:</span>
                {timeZoneLabel(timeZone, labelAt)}
              </span>
            )}
          </p>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Select
          aria-label="Project"
          value={projectId}
          onChange={(e) => onProject(e.target.value)}
          className="h-10 w-full min-w-0 sm:w-auto sm:min-w-40"
        >
          <option value={ALL}>All projects</option>
          {projectOptions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.archived ? `${p.title} (archived)` : p.title}
            </option>
          ))}
        </Select>
        <Select
          aria-label="Member"
          value={memberId}
          onChange={(e) => onMember(e.target.value)}
          className="h-10 w-full min-w-0 sm:w-auto sm:min-w-40"
        >
          <option value={ALL}>All members</option>
          {members.map((u) => (
            <option key={u.id} value={u.id}>
              {u.name}
            </option>
          ))}
        </Select>
        {extra}
      </div>
    </div>
  );
}
