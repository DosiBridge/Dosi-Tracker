"use client";

import { useState } from "react";
import { Search, SlidersHorizontal, X, RotateCcw } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Select } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { projects, users } from "@/lib/tenant-data";
import { rangePresets } from "@/lib/reports-data";
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
  "prod-high": "Productivity: high → low",
  "prod-low": "Productivity: low → high",
  "active-high": "Most active (input)",
};

export function ActivityFilterBar({
  value,
  onChange,
  resultCount,
  totalCount,
  showMemberFilter = true,
}: {
  value: ActivityFilters;
  onChange: (f: ActivityFilters) => void;
  resultCount: number;
  totalCount: number;
  showMemberFilter?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const set = <K extends keyof ActivityFilters>(key: K, val: ActivityFilters[K]) =>
    onChange({ ...value, [key]: val });

  const activeCount = countActiveFilters(value);
  const chips = activeFilterChips(value);
  const apps = activityApps();

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
      {/* Row 1 */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            value={value.query}
            onChange={(e) => set("query", e.target.value)}
            placeholder="Search task, app, window, member, project…"
            className="h-10 w-full rounded-lg border border-border bg-background pl-9 pr-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>

        {showMemberFilter && (
          <Select value={value.memberId} onChange={(e) => set("memberId", e.target.value)} className="w-auto min-w-36">
            <option value="all">All members</option>
            {users.filter((u) => u.role !== "client").map((u) => (
              <option key={u.id} value={u.id}>{u.name}</option>
            ))}
          </Select>
        )}

        <Select value={value.projectId} onChange={(e) => set("projectId", e.target.value)} className="w-auto min-w-36">
          <option value="all">All projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>{p.title}</option>
          ))}
        </Select>

        <Select value={value.sort} onChange={(e) => set("sort", e.target.value as ActivitySort)} className="w-auto min-w-44">
          {(Object.keys(sortLabels) as ActivitySort[]).map((s) => (
            <option key={s} value={s}>{sortLabels[s]}</option>
          ))}
        </Select>

        <button
          onClick={() => setOpen((o) => !o)}
          className={cn(
            "inline-flex h-10 items-center gap-2 rounded-lg border px-3 text-sm transition-colors",
            open || activeCount > 0 ? "border-primary/40 bg-accent text-accent-foreground" : "border-border hover:bg-muted"
          )}
        >
          <SlidersHorizontal className="h-4 w-4" /> Filters
          {activeCount > 0 && (
            <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-bold text-primary-foreground">
              {activeCount}
            </span>
          )}
        </button>
      </div>

      {/* Expanded panel */}
      {open && (
        <div className="mt-3 grid grid-cols-1 gap-4 border-t border-border pt-4 md:grid-cols-2 lg:grid-cols-4">
          {/* Date range */}
          <div className="space-y-2 md:col-span-2">
            <label className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Date range</label>
            <div className="flex flex-wrap gap-1.5">
              {rangePresets.map((r) => (
                <button
                  key={r.key}
                  onClick={() => set("rangeKey", r.key)}
                  className={cn(
                    "rounded-lg border px-2.5 py-1 text-xs transition-colors",
                    value.rangeKey === r.key ? "border-primary bg-primary text-primary-foreground" : "border-border hover:bg-muted"
                  )}
                >
                  {r.label}
                </button>
              ))}
            </div>
            {value.rangeKey === "custom" && (
              <div className="flex items-center gap-2">
                <input
                  type="date"
                  value={value.customFrom}
                  onChange={(e) => set("customFrom", e.target.value)}
                  className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
                />
                <span className="text-muted-foreground">→</span>
                <input
                  type="date"
                  value={value.customTo}
                  onChange={(e) => set("customTo", e.target.value)}
                  className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
                />
              </div>
            )}
          </div>

          {/* Time of day */}
          <div className="space-y-2">
            <label className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Time of day</label>
            <div className="flex items-center gap-2">
              <input
                type="time"
                value={value.startTime}
                onChange={(e) => set("startTime", e.target.value)}
                className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
              />
              <span className="text-muted-foreground">–</span>
              <input
                type="time"
                value={value.endTime}
                onChange={(e) => set("endTime", e.target.value)}
                className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm"
              />
            </div>
          </div>

          {/* App */}
          <div className="space-y-2">
            <label className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Application</label>
            <Select value={value.app} onChange={(e) => set("app", e.target.value)}>
              <option value="all">All apps</option>
              {apps.map((a) => (
                <option key={a} value={a}>{a}</option>
              ))}
            </Select>
          </div>

          {/* Productivity range */}
          <div className="space-y-2 md:col-span-2">
            <label className="flex items-center justify-between text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Productivity <span className="text-foreground">{value.minProductivity}% – {value.maxProductivity}%</span>
            </label>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-xs">
                <span className="w-8 text-muted-foreground">Min</span>
                <input
                  type="range" min={0} max={100} step={5}
                  value={value.minProductivity}
                  onChange={(e) => set("minProductivity", Math.min(Number(e.target.value), value.maxProductivity))}
                  className="h-1.5 flex-1 cursor-pointer accent-[var(--primary)]"
                />
              </div>
              <div className="flex items-center gap-2 text-xs">
                <span className="w-8 text-muted-foreground">Max</span>
                <input
                  type="range" min={0} max={100} step={5}
                  value={value.maxProductivity}
                  onChange={(e) => set("maxProductivity", Math.max(Number(e.target.value), value.minProductivity))}
                  className="h-1.5 flex-1 cursor-pointer accent-[var(--primary)]"
                />
              </div>
            </div>
          </div>

          {/* Toggles */}
          <div className="space-y-2.5 md:col-span-2">
            <label className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Options</label>
            <Switch checked={value.onlineOnly} onChange={(v) => set("onlineOnly", v)} label="Live sessions only" />
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
              onClick={() => removeChip(c.key as string)}
              className="inline-flex items-center gap-1 rounded-full bg-accent px-2.5 py-1 text-xs text-accent-foreground hover:opacity-80"
            >
              {c.label}
              <X className="h-3 w-3" />
            </button>
          ))}
          {chips.length > 0 && (
            <button onClick={() => onChange({ ...defaultActivityFilters, query: value.query })} className="text-xs text-primary hover:underline">
              Clear filters
            </button>
          )}
        </div>
      )}
    </Card>
  );
}
