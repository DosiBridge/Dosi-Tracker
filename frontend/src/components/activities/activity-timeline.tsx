"use client";

import { useMemo, useState } from "react";
import {
  formatClock,
  formatClockShort,
  summarizeTimeline,
  timelineSlots,
  timelineTickStep,
  type TimelineSlot,
} from "@/lib/activity-report";
import type { Activity } from "@/lib/types";
import { cn } from "@/lib/utils";

/*
 * Series colors: categorical slots 1 (blue) and 2 (orange) of the dataviz
 * reference palette, each stepped for its surface. Validated as a pair against
 * this app's cards (light #ffffff, dark #1a2226): CVD ΔE ≥ 24, normal-vision
 * ΔE ≥ 31, ≥ 3:1 contrast in both modes. Idle is a neutral band, not a hue.
 */
const KEYBOARD_FILL = "bg-[#2a78d6] dark:bg-[#3987e5]";
const MOUSE_FILL = "bg-[#eb6834] dark:bg-[#d95926]";
const IDLE_FILL = "bg-muted-foreground/15";

const n = (value: number) => value.toLocaleString("en");

function slotStatus(slot: TimelineSlot): string {
  if (slot.missing) return "No data";
  return slot.active ? "Active" : "Idle";
}

/** One minute in words — the tooltip, the keyboard readout and the table share it. */
function describeSlot(slot: TimelineSlot): string {
  const app = slot.appName ? ` in ${slot.appName}` : "";
  if (slot.missing) return `${formatClock(slot.at)}: no data`;
  if (!slot.active && slot.total === 0) return `${formatClock(slot.at)}: idle`;
  return `${formatClock(slot.at)}: ${n(slot.keyboardHits)} keyboard, ${n(slot.mouseClicks)} mouse${app}`;
}

/**
 * The block's input per minute: one column per minute, keyboard and mouse
 * stacked, idle minutes shaded as a neutral band. Hover (or focus the chart and
 * use ←/→) for a minute's detail; the same numbers are in the table below it.
 * Renders nothing when the block has no timeline (legacy rows, demo data).
 */
export function ActivityTimeline({ activity }: { activity: Pick<Activity, "startedAt" | "endedAt" | "timeline"> }) {
  const slots = useMemo(() => timelineSlots(activity), [activity]);
  const summary = useMemo(() => summarizeTimeline(slots), [slots]);
  const [hovered, setHovered] = useState<number | null>(null);

  if (slots.length === 0) return null;

  const max = Math.max(1, ...slots.map((s) => s.total));
  const step = timelineTickStep(slots.length);
  const current = hovered !== null ? slots[hovered] : undefined;

  const label =
    `Input per minute, ${formatClock(activity.startedAt)} to ${formatClock(activity.endedAt)}: ` +
    `active ${summary.activeMinutes} of ${summary.minutes} minutes, ` +
    `${n(summary.keyboardHits)} keyboard presses and ${n(summary.mouseClicks)} mouse clicks` +
    (summary.peak ? `; busiest minute ${formatClock(summary.peak.at)} with ${n(summary.peak.total)} inputs.` : ".");

  function onKeyDown(e: React.KeyboardEvent) {
    const last = slots.length - 1;
    const at = hovered ?? 0;
    const next =
      e.key === "ArrowRight" ? Math.min(last, at + 1)
      : e.key === "ArrowLeft" ? Math.max(0, at - 1)
      : e.key === "Home" ? 0
      : e.key === "End" ? last
      : null;
    if (next === null) return;
    e.preventDefault();
    setHovered(next);
  }

  // The tooltip sits inside the plot, beside the hovered column (never over it):
  // to its right in the left half of the chart, to its left in the right half.
  const tipPosition = (i: number): React.CSSProperties => {
    const count = slots.length;
    return (i + 0.5) / count < 0.5
      ? { left: `calc(${((i + 1) / count) * 100}% + 6px)` }
      : { right: `calc(${((count - i) / count) * 100}% + 6px)` };
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <LegendKey className={KEYBOARD_FILL} label="Keyboard" />
        <LegendKey className={MOUSE_FILL} label="Mouse" />
        <LegendKey className={IDLE_FILL} label="Idle" />
        <span className="ml-auto">
          {summary.activeMinutes} of {summary.minutes} min active
        </span>
      </div>

      <div className="relative pt-6">
        {/* The one gridline: the busiest minute's level, so bar heights read as values. */}
        <div className="pointer-events-none absolute inset-x-0 top-6 border-t border-border" aria-hidden />
        <span className="pointer-events-none absolute right-0 top-1 text-[10px] tabular-nums text-muted-foreground" aria-hidden>
          {n(max)} / min
        </span>

        {current && (
          <div
            className="pointer-events-none absolute top-7 z-10 w-max max-w-[14rem] rounded-lg border border-border bg-card px-2.5 py-1.5 text-xs card-elev-lg"
            style={tipPosition(current.minute)}
            aria-hidden
          >
            <div className="font-medium text-muted-foreground">
              {formatClock(current.at)} · {slotStatus(current)}
            </div>
            {!current.missing && (
              <div className="mt-0.5 space-y-0.5">
                <TipRow className={KEYBOARD_FILL} value={current.keyboardHits} label="keyboard" />
                <TipRow className={MOUSE_FILL} value={current.mouseClicks} label="mouse" />
              </div>
            )}
            {current.appName && <div className="mt-0.5 truncate text-muted-foreground">{current.appName}</div>}
          </div>
        )}

        <div
          role="group"
          aria-label={label}
          tabIndex={0}
          onKeyDown={onKeyDown}
          onFocus={() => setHovered((h) => h ?? 0)}
          onBlur={() => setHovered(null)}
          onPointerLeave={() => setHovered(null)}
          className="flex h-28 items-stretch gap-0.5 rounded-sm border-b border-border outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-card"
        >
          {slots.map((slot, i) => (
            <Column
              key={slot.minute}
              slot={slot}
              max={max}
              emphasis={hovered === null ? "none" : hovered === i ? "focus" : "dim"}
              onEnter={() => setHovered(i)}
            />
          ))}
        </div>

        <div className="mt-1 flex gap-0.5" aria-hidden>
          {slots.map((slot) => (
            <div key={slot.minute} className="relative h-4 min-w-0 flex-1">
              {slot.minute % step === 0 && (
                <span className="absolute left-1/2 top-0 -translate-x-1/2 whitespace-nowrap text-[10px] tabular-nums text-muted-foreground">
                  {formatClockShort(slot.at)}
                </span>
              )}
            </div>
          ))}
        </div>

        <p className="sr-only" aria-live="polite">
          {current ? describeSlot(current) : ""}
        </p>
      </div>

      <details className="group text-xs">
        <summary className="cursor-pointer select-none text-muted-foreground hover:text-foreground">
          Minute-by-minute table
        </summary>
        <div className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-border">
          <table className="w-full text-left">
            <caption className="sr-only">Input per minute</caption>
            <thead className="sticky top-0 bg-card text-muted-foreground">
              <tr>
                <th scope="col" className="px-3 py-1.5 font-medium">Time</th>
                <th scope="col" className="px-3 py-1.5 font-medium">Status</th>
                <th scope="col" className="px-3 py-1.5 text-right font-medium">Keyboard</th>
                <th scope="col" className="px-3 py-1.5 text-right font-medium">Mouse</th>
                <th scope="col" className="px-3 py-1.5 font-medium">App</th>
              </tr>
            </thead>
            <tbody>
              {slots.map((slot) => (
                <tr key={slot.minute} className="border-t border-border">
                  <td className="whitespace-nowrap px-3 py-1.5 tabular-nums">{formatClock(slot.at)}</td>
                  <td className="px-3 py-1.5">{slotStatus(slot)}</td>
                  <td className="px-3 py-1.5 text-right tabular-nums">{slot.missing ? "—" : n(slot.keyboardHits)}</td>
                  <td className="px-3 py-1.5 text-right tabular-nums">{slot.missing ? "—" : n(slot.mouseClicks)}</td>
                  <td className="max-w-[10rem] truncate px-3 py-1.5 text-muted-foreground">{slot.appName ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}

function Column({
  slot,
  max,
  emphasis,
  onEnter,
}: {
  slot: TimelineSlot;
  max: number;
  /** while a minute is inspected, the others recede so it stands out */
  emphasis: "none" | "focus" | "dim";
  onEnter: () => void;
}) {
  const pct = (v: number) => `${(v / max) * 100}%`;
  const hasMouse = slot.mouseClicks > 0;
  const hasKeys = slot.keyboardHits > 0;
  return (
    // The whole column is the hover target, not just the painted bar.
    <div onPointerEnter={onEnter} className="relative flex h-full min-w-0 flex-1 justify-center">
      {!slot.active && (
        <div
          className={cn(
            "absolute inset-0 rounded-t-sm transition-colors",
            emphasis === "focus" ? "bg-muted-foreground/30" : IDLE_FILL,
          )}
        />
      )}
      <div
        className={cn(
          "relative flex h-full w-full max-w-6 flex-col justify-end transition-opacity",
          emphasis === "dim" && "opacity-40",
        )}
      >
        {hasMouse && <div className={cn("rounded-t-[4px]", MOUSE_FILL)} style={{ height: pct(slot.mouseClicks), minHeight: 2 }} />}
        {hasKeys && (
          <div
            className={cn(KEYBOARD_FILL, hasMouse ? "mt-0.5" : "rounded-t-[4px]")}
            style={{ height: pct(slot.keyboardHits), minHeight: 2 }}
          />
        )}
        {/* Active with no clicks or keys (e.g. reading, scrolling): a stub, so it doesn't read as idle. */}
        {slot.active && slot.total === 0 && <div className="h-0.5 rounded-t-[2px] bg-muted-foreground/50" />}
      </div>
    </div>
  );
}

function LegendKey({ className, label }: { className: string; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className={cn("h-2.5 w-2.5 rounded-[3px]", className)} aria-hidden />
      {label}
    </span>
  );
}

function TipRow({ className, value, label }: { className: string; value: number; label: string }) {
  return (
    <div className="flex items-center gap-1.5">
      <span className={cn("h-0.5 w-3 rounded-full", className)} aria-hidden />
      <span className="font-semibold tabular-nums text-foreground">{n(value)}</span>
      <span className="text-muted-foreground">{label}</span>
    </div>
  );
}
