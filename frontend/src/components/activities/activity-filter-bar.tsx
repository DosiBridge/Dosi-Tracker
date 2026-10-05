"use client";

import { useId, useState } from "react";
import { Globe, Search, SlidersHorizontal, X, RotateCcw } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { projects, users } from "@/lib/tenant-data";
import { rangePresets } from "@/lib/reports-data";
import { timeZoneLabel } from "@/lib/time-zone";
import { referenceNow } from "@/lib/live-session";
import {
  activityApps,
  activeFilterChips,
  countActiveFilters,
  defaultActivityFilters,
  type ActivityFilters,
  type ActivitySort,
} from "@/lib/activity-filters";

const sortLabels: Record<ActivitySort, string> = {
  recent: "Most recent",
  oldest: "Oldest first",
  "prod-high": "Activity: high → low",
  "prod-low": "Activity: low → high",
  "active-high": "Most active (input)",
};

export function ActivityFilterBar({
  value,
  onChange,
  resultCount,
  totalCount,
  showMemberFilter = true,
  apps: appOptions,
  trackingNow = null,
  timeZone,
}: {
  value: ActivityFilters;
  onChange: (f: ActivityFilters) => void;
  resultCount: number;
  totalCount: number;
  showMemberFilter?: boolean;
  /** The apps to offer (canonical names); default: every app in the session's rows. */
  apps?: string[];
  /** How many members are tracking right now (shown under "Live sessions only"); null when unknown. */
  trackingNow?: number | null;
  /** The reporting zone the dates and times of day are read in (shown under the date range). */
  timeZone?: string;
}) {
  const id = useId();
  const panelId = `${id}panel`;
  const [open, setOpen] = useState(false);
  // The instant the zone's UTC offset is read at (the session's clock): fixed for the bar's life, so render stays pure.
  const [labelAt] = useState(() => referenceNow());
  const set = <K extends keyof ActivityFilters>(key: K, val: ActivityFilters[K]) =>
    onChange({ ...value, [key]: val });

  const activeCount = countActiveFilters(value);
  const chips = activeFilterChips(value);
  const apps = appOptions ?? activityApps();
  const liveHint =
    trackingNow === null
      ? undefined
      : trackingNow === 0
        ? "Nobody is tracking right now"
        : `${trackingNow} ${trackingNow === 1 ? "member is" : "members are"} tracking now`;
  const label = "text-xs font-semibold uppercase tracking-wide text-muted-foreground";

  function removeChip(key: string) {
    const f = { ...value };
    switch (key) {
      case "productivity":
        f.minProductivity = 0;
        f.maxProductivity = 100;
        break;
      case "time":
        f.startTime = "";
        f.endTime = "";
        break;
      case "rangeKey":
        f.rangeKey = "7d";
        f.customFrom = "";
        f.customTo = "";
        break;
      case "onlineOnly":
        f.onlineOnly = false;
        break;
      case "webcamOnly":
        f.webcamOnly = false;
        break;
      default:
        // memberId, projectId, app
        (f as unknown as Record<string, string>)[key] = "all";
    }
    onChange(f);
  }

  return (
    <Card className="p-3">
      {/* Row 1 — search + filters always; selects hide below md */}
      <div className="flex flex-wrap items-center gap-2 sm:gap-3">
        <div className="relative min-w-0 flex-1 basis-full sm:basis-auto">
          <Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            type="search"
            aria-label="Search sessions"
            value={value.query}
            onChange={(e) => set("query", e.target.value)}
            placeholder="Search task, app, member…"
            className="h-10 w-full rounded-xl border border-border bg-card pl-9 pr-3 text-sm shadow-[inset_0_1px_0_rgb(255_255_255_/_0.04)] focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>

        {showMemberFilter && (
          <Select aria-label="Member" value={value.memberId} onChange={(e) => set("memberId", e.target.value)} className="hidden w-auto min-w-36 md:block">
            <option value="all">All members</option>
            {users.filter((u) => u.role !== "client").map((u) => (
              <option key={u.id} value={u.id}>{u.name}</option>
            ))}
          </Select>
        )}

        <Select aria-label="Project" value={value.projectId} onChange={(e) => set("projectId", e.target.value)} className="hidden w-auto min-w-36 md:block">
          <option value="all">All projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>{p.title}</option>
          ))}
        </Select>

        <Select aria-label="Sort" value={value.sort} onChange={(e) => set("sort", e.target.value as ActivitySort)} className="hidden w-auto min-w-44 lg:block">
          {(Object.keys(sortLabels) as ActivitySort[]).map((s) => (
            <option key={s} value={s}>{sortLabels[s]}</option>
          ))}
        </Select>

        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls={open ? panelId : undefined}
          aria-label={activeCount > 0 ? `Filters, ${activeCount} active` : "Filters"}
          className={cn(
            "inline-flex h-10 shrink-0 items-center gap-2 rounded-lg border px-3 text-sm transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            open || activeCount > 0 ? "border-primary/40 bg-accent text-accent-foreground" : "border-border hover:bg-muted"
          )}
        >
          <SlidersHorizontal aria-hidden className="h-4 w-4" />
          <span className="hidden xs:inline sm:inline">Filters</span>
          {activeCount > 0 && (
            <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-bold text-primary-foreground">
              {activeCount}
            </span>
          )}
        </button>
      </div>

      {/* Expanded panel */}
      {open && (
        <div id={panelId} className="mt-3 grid grid-cols-1 gap-4 border-t border-border pt-4 md:grid-cols-2 lg:grid-cols-4">
          {/* Mobile-only primary filters */}
          <div className="space-y-3 md:hidden">
            {showMemberFilter && (
              <div className="space-y-1.5">
                <label htmlFor={`${id}member`} className={label}>Member</label>
                <Select id={`${id}member`} value={value.memberId} onChange={(e) => set("memberId", e.target.value)}>
                  <option value="all">All members</option>
                  {users.filter((u) => u.role !== "client").map((u) => (
                    <option key={u.id} value={u.id}>{u.name}</option>
                  ))}
                </Select>
              </div>
            )}
            <div className="space-y-1.5">
              <label htmlFor={`${id}project`} className={label}>Project</label>
              <Select id={`${id}project`} value={value.projectId} onChange={(e) => set("projectId", e.target.value)}>
                <option value="all">All projects</option>
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>{p.title}</option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <label htmlFor={`${id}sort`} className={label}>Sort</label>
              <Select id={`${id}sort`} value={value.sort} onChange={(e) => set("sort", e.target.value as ActivitySort)}>
                {(Object.keys(sortLabels) as ActivitySort[]).map((s) => (
                  <option key={s} value={s}>{sortLabels[s]}</option>
                ))}
              </Select>
            </div>
          </div>

          {/* Date range */}
          <div className="space-y-2 md:col-span-2">
            <p id={`${id}dates`} className={label}>Date range</p>
            <div role="group" aria-labelledby={`${id}dates`} className="flex flex-wrap gap-1.5">
              {rangePresets.map((r) => (
                <button
                  key={r.key}
                  type="button"
                  aria-pressed={value.rangeKey === r.key}
                  onClick={() => set("rangeKey", r.key)}
                  className={cn(
                    "rounded-lg border px-2.5 py-1 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    value.rangeKey === r.key ? "border-primary bg-primary text-primary-foreground" : "border-border hover:bg-muted"
                  )}
                >
                  {r.label}
                </button>
              ))}
            </div>
            {value.rangeKey === "custom" && (
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                <input
                  type="date"
                  aria-label="From day"
                  value={value.customFrom}
                  max={value.customTo || undefined}
                  onChange={(e) => set("customFrom", e.target.value)}
                  className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
                />
                <span aria-hidden className="hidden text-muted-foreground sm:inline">→</span>
                <input
                  type="date"
                  aria-label="To day"
                  value={value.customTo}
                  min={value.customFrom || undefined}
                  onChange={(e) => set("customTo", e.target.value)}
                  className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
                />
              </div>
            )}
            {timeZone && (
              <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground" title={timeZone}>
                <Globe aria-hidden className="h-3.5 w-3.5 shrink-0" />
                Days and times in {timeZoneLabel(timeZone, labelAt)}
              </p>
            )}
          </div>

          {/* Time of day */}
          <div className="space-y-2">
            <p id={`${id}time`} className={label}>Time of day (start)</p>
            <div role="group" aria-labelledby={`${id}time`} className="flex items-center gap-2">
              <input
                type="time"
                aria-label="Started at or after"
                value={value.startTime}
                onChange={(e) => set("startTime", e.target.value)}
                className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
              />
              <span aria-hidden className="text-muted-foreground">–</span>
              <input
                type="time"
                aria-label="Started at or before"
                value={value.endTime}
                onChange={(e) => set("endTime", e.target.value)}
                className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
              />
            </div>
          </div>

          {/* App */}
          <div className="space-y-2">
            <label htmlFor={`${id}app`} className={label}>Application</label>
            <Select id={`${id}app`} value={value.app} onChange={(e) => set("app", e.target.value)}>
              <option value="all">All apps</option>
              {apps.map((a) => (
                <option key={a} value={a}>{a}</option>
              ))}
            </Select>
          </div>

          {/* Activity % range */}
          <div className="space-y-2 md:col-span-2">
            <p className={cn(label, "flex items-center justify-between")}>
              Activity <span className="text-foreground">{value.minProductivity}% – {value.maxProductivity}%</span>
            </p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-xs">
                <span className="w-8 text-muted-foreground">Min</span>
                <input
                  type="range" min={0} max={100} step={5}
                  aria-label="Minimum activity %"
                  value={value.minProductivity}
                  onChange={(e) => set("minProductivity", Math.min(Number(e.target.value), value.maxProductivity))}
                  className="h-1.5 flex-1 cursor-pointer accent-[var(--primary)]"
                />
              </div>
              <div className="flex items-center gap-2 text-xs">
                <span className="w-8 text-muted-foreground">Max</span>
                <input
                  type="range" min={0} max={100} step={5}
                  aria-label="Maximum activity %"
                  value={value.maxProductivity}
                  onChange={(e) => set("maxProductivity", Math.max(Number(e.target.value), value.minProductivity))}
                  className="h-1.5 flex-1 cursor-pointer accent-[var(--primary)]"
                />
              </div>
            </div>
            {(value.minProductivity !== 0 || value.maxProductivity !== 100) && (
              <p className="text-[11px] text-muted-foreground">Sessions whose activity wasn’t measured are left out.</p>
            )}
          </div>

          {/* Toggles */}
          <div className="space-y-2.5 md:col-span-2">
            <p className={label}>Options</p>
            <Switch checked={value.onlineOnly} onChange={(v) => set("onlineOnly", v)} label="Live sessions only" description={liveHint} />
            <Switch checked={value.webcamOnly} onChange={(v) => set("webcamOnly", v)} label="With webcam capture" />
          </div>

          <div className="flex items-end md:col-span-2 lg:col-span-4">
            <Button variant="outline" size="sm" onClick={() => onChange({ ...defaultActivityFilters })}>
              <RotateCcw className="h-3.5 w-3.5" /> Reset all filters
            </Button>
          </div>
        </div>
      )}

      {/* Active chips + result count */}
      {(chips.length > 0 || value.query) && (
        <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-border pt-3">
          <span className="text-xs text-muted-foreground">{resultCount} of {totalCount}</span>
          {chips.map((c) => (
            <button
              key={c.key}
              type="button"
              aria-label={`Remove filter: ${c.label}`}
              onClick={() => removeChip(c.key as string)}
              className="inline-flex items-center gap-1 rounded-full bg-accent px-2.5 py-1 text-xs text-accent-foreground hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {c.label}
              <X aria-hidden className="h-3 w-3" />
            </button>
          ))}
          {chips.length > 0 && (
            <button
              type="button"
              onClick={() => onChange({ ...defaultActivityFilters, query: value.query })}
              className="text-xs font-medium text-primary-strong hover:underline"
            >
              Clear filters
            </button>
          )}
        </div>
      )}
    </Card>
  );
}
