"use client";

import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
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
import { PRESENCE_STATE_LABELS, type MemberPresence, type PresenceState } from "@/lib/monitor-live";
import { agoLabel } from "@/lib/live-session";
import { formatClock } from "@/lib/activity-report";
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

/** A block whose input wasn't measured: neutral, never a red "0%". */
export const UNMEASURED_COLOR = "#94a3b8";

/** A segment's colour: work by its activity % (neutral when not measured), the rest by type. */
export function segmentColor(s: Pick<DaySegment, "type" | "productivity" | "measured">): string {
  if (s.type !== "work") return typeColor[s.type];
  return s.measured === false ? UNMEASURED_COLOR : prodColor(s.productivity);
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

/**
 * Legend for a live day: real agents report activity blocks only (no meetings,
 * breaks or idle), and each block is colored by its activity % — grey when the
 * agent couldn't measure input.
 */
export function ProductivityLegend({ className, showUnmeasured = false }: { className?: string; showUnmeasured?: boolean }) {
  const bands = [
    { label: "≥ 75% active", color: prodColor(75) },
    { label: "50–74%", color: prodColor(50) },
    { label: "< 50%", color: prodColor(0) },
    ...(showUnmeasured ? [{ label: "Not measured", color: UNMEASURED_COLOR }] : []),
  ];
  return (
    <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground", className)}>
      {bands.map((b) => (
        <span key={b.label} className="flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm" style={{ background: b.color }} />
          {b.label}
        </span>
      ))}
    </div>
  );
}

/**
 * A compact day bar. `span` is the minutes-of-day axis it draws (the 08:00–20:00
 * working day by default); live views widen it so real work outside business
 * hours is not clipped away.
 */
export function MiniDayStrip({
  segments,
  span = { start: DAY_START_MIN, end: DAY_END_MIN },
}: {
  segments: DaySegment[];
  span?: { start: number; end: number };
}) {
  if (segments.length === 0) {
    return <div className="h-2.5 w-full rounded-full bg-muted" aria-hidden />;
  }
  const range = Math.max(1, span.end - span.start);
  return (
    <div
      className="relative h-2.5 w-full overflow-hidden rounded-full bg-muted/70"
      title={segments
        .map((s) => `${fmtMin(s.startMin)} ${s.type === "work" || s.type === "meeting" ? s.app : s.type}`)
        .join(" · ")}
    >
      {segments.map((s) => {
        const left = ((s.startMin - span.start) / range) * 100;
        const width = Math.max(0.8, (s.minutes / range) * 100);
        const color = segmentColor(s);
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

/** Badge tones per presence state: tracking reads green, stepping away amber or blue, offline grey (all AA on their tints). */
const PRESENCE_TONES: Record<PresenceState, "success" | "warning" | "info" | "muted"> = {
  tracking: "success",
  idle: "warning",
  locked: "warning",
  paused: "info",
  break: "info",
  choosing: "info",
  offline: "muted",
};

const PRESENCE_DOTS: Record<PresenceState, string> = {
  tracking: "bg-success",
  idle: "bg-warning",
  locked: "bg-warning",
  paused: "bg-info",
  break: "bg-info",
  choosing: "bg-info",
  offline: "bg-muted-foreground",
};

/** What a member is doing right now: "Tracking", "On a break", "Offline"… */
export function PresenceChip({ state, className }: { state: PresenceState; className?: string }) {
  return (
    <Badge tone={PRESENCE_TONES[state]} className={cn("gap-1.5", className)}>
      <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", PRESENCE_DOTS[state], state === "tracking" && "live-dot")} />
      {PRESENCE_STATE_LABELS[state]}
    </Badge>
  );
}

/**
 * The line under a member's state: since when (while online), else when they
 * were last seen — the later of their last heartbeat and their last upload.
 */
export function presenceSince(
  presence: Pick<MemberPresence, "state" | "since" | "lastSeenAt"> | undefined,
  lastUpload: number | undefined,
  now: Date,
  timeZone?: string,
): string | null {
  if (presence && presence.state !== "offline") {
    return presence.since ? `since ${formatClock(presence.since, timeZone)}` : null;
  }
  const heartbeat = presence?.lastSeenAt ? Date.parse(presence.lastSeenAt) : Number.NaN;
  const seen = Math.max(Number.isFinite(heartbeat) ? heartbeat : 0, lastUpload ?? 0);
  return seen > 0 ? `last seen ${agoLabel(new Date(seen).toISOString(), now)}` : "never seen";
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
