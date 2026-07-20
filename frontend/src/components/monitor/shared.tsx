"use client";

import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { brand } from "@/lib/brand";
import {
  DAY_END_MIN,
  DAY_START_MIN,
  fmtMin,
  type DaySegment,
  type SegmentType,
} from "@/lib/monitor-data";
import { cn } from "@/lib/utils";

export const RANGE = DAY_END_MIN - DAY_START_MIN;

export const typeColor: Record<SegmentType, string> = {
  work: brand.success,
  meeting: brand.info,
  break: brand.warning,
  idle: "#94a3b8",
};

export function prodColor(p: number): string {
  if (p >= 75) return brand.success;
  if (p >= 50) return brand.warning;
  return brand.danger;
}

export function TypeLegend({ className }: { className?: string }) {
  return (
    <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground", className)}>
      {(["work", "meeting", "break", "idle"] as SegmentType[]).map((t) => (
        <span key={t} className="flex items-center gap-1.5 capitalize">
          <span className="h-2.5 w-2.5 rounded-sm" style={{ background: typeColor[t] }} />
          {t}
        </span>
      ))}
    </div>
  );
}

export function MiniDayStrip({ segments }: { segments: DaySegment[] }) {
  if (segments.length === 0) {
    return <div className="h-2.5 w-full rounded-full bg-muted" aria-hidden />;
  }
  return (
    <div
      className="relative h-2.5 w-full overflow-hidden rounded-full bg-muted/70"
      title={segments
        .map((s) => `${fmtMin(s.startMin)} ${s.type === "work" || s.type === "meeting" ? s.app : s.type}`)
        .join(" · ")}
    >
      {segments.map((s) => {
        const left = ((s.startMin - DAY_START_MIN) / RANGE) * 100;
        const width = Math.max(0.8, (s.minutes / RANGE) * 100);
        const color = s.type === "work" ? prodColor(s.productivity) : typeColor[s.type];
        const muted = s.type === "idle" || s.type === "break";
        return (
          <div
            key={s.id}
            className={cn("absolute inset-y-0 rounded-[1px]", muted && "opacity-45")}
            style={{ left: `${left}%`, width: `${width}%`, background: color }}
          />
        );
      })}
    </div>
  );
}

export function DayNav({
  label,
  onPrev,
  onNext,
  canPrev,
  canNext,
  onToday,
  showToday,
}: {
  label: string;
  onPrev: () => void;
  onNext: () => void;
  canPrev: boolean;
  canNext: boolean;
  onToday?: () => void;
  showToday?: boolean;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="outline" size="icon" onClick={onPrev} disabled={!canPrev} aria-label="Previous day">
        <ChevronLeft className="h-4 w-4" />
      </Button>
      <div className="flex min-w-0 flex-1 items-center justify-center gap-2 rounded-xl border border-border bg-background px-3 py-2 text-sm sm:min-w-40 sm:flex-none">
        <CalendarDays className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate font-medium">{label}</span>
      </div>
      <Button variant="outline" size="icon" onClick={onNext} disabled={!canNext} aria-label="Next day">
        <ChevronRight className="h-4 w-4" />
      </Button>
      {showToday && onToday && (
        <Button variant="ghost" size="sm" onClick={onToday}>
          Today
        </Button>
      )}
    </div>
  );
}

export function MonitorKpi({
  icon: Icon,
  tone,
  label,
  value,
}: {
  icon: LucideIcon;
  tone: string;
  label: string;
  value: string;
}) {
  return (
    <Card className="p-3.5">
      <div className="flex items-center gap-2.5">
        <div
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl"
          style={{ background: `${tone}1a`, color: tone }}
        >
          <Icon className="h-4 w-4" />
        </div>
        <div className="min-w-0">
          <div className="truncate font-display text-base font-bold tabular-nums tracking-tight">{value}</div>
          <div className="truncate text-[11px] text-muted-foreground">{label}</div>
        </div>
      </div>
    </Card>
  );
}
