"use client";

import type { LucideIcon } from "lucide-react";
import {
  ArrowLeftRight,
  Coffee,
  EyeOff,
  Hourglass,
  Laptop,
  ListTodo,
  Lock,
  LogOut,
  Monitor,
  Moon,
  Pause,
  Power,
  TimerReset,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { END_REASON_LABELS, platformLabel, type ActivityEndReason } from "@/lib/api-contract";
import { deviceSummary, endReasonTone, formatBlockLength, notableEndReason, type SessionDetails } from "@/lib/activity-report";
import { cn } from "@/lib/utils";

/* ============================================================================
 * SESSION META — the small pieces newer agents add to a block, drawn the same
 * way on the session cards, the block report, the screenshot viewer and the
 * Member Monitor: why the block ended, its idle time, the device, and whether
 * activity was measured at all. Each renders nothing when its data wasn't
 * reported (older agents), so legacy rows read exactly as before.
 * ========================================================================== */

const END_REASON_ICONS: Record<ActivityEndReason, LucideIcon> = {
  interval: TimerReset,
  pause: Pause,
  break: Coffee,
  idle: Hourglass,
  lock: Lock,
  sleep: Moon,
  project: ArrowLeftRight,
  task: ListTodo,
  signout: LogOut,
  quit: Power,
  shutdown: Power,
};

/** "Paused", "Screen locked"… — why the block ended; nothing for a block that simply reached its interval. */
export function EndReasonChip({ reason, className }: { reason: ActivityEndReason | null | undefined; className?: string }) {
  const notable = notableEndReason(reason);
  if (!notable) return null;
  const Icon = END_REASON_ICONS[notable];
  return (
    <Badge tone={endReasonTone(notable)} className={cn("gap-1 px-2 py-0 text-[11px] leading-5", className)} title={`Block ended: ${END_REASON_LABELS[notable]}`}>
      <Icon aria-hidden className="h-3 w-3" />
      <span className="sr-only">Ended: </span>
      {END_REASON_LABELS[notable]}
    </Badge>
  );
}

/** "4m idle" — the block's time without keyboard or mouse input, when the agent reported it. */
export function IdleTime({ seconds, className }: { seconds: number | null | undefined; className?: string }) {
  if (typeof seconds !== "number" || seconds <= 0) return null;
  return (
    <span className={cn("inline-flex items-center gap-1 whitespace-nowrap", className)} title="Time in this block with no keyboard or mouse input">
      <Hourglass aria-hidden className="h-3 w-3" />
      {formatBlockLength(seconds)} idle
    </span>
  );
}

/** The device a block came from: its name (else the platform), with the full summary on hover. */
export function DeviceTag({ details, className }: { details: Pick<SessionDetails, "platform" | "osVersion" | "deviceName" | "agentVersion">; className?: string }) {
  const label = details.deviceName ?? platformLabel(details.platform);
  if (!label) return null;
  const Icon = details.platform === "macos" ? Laptop : Monitor;
  const summary = deviceSummary(details) ?? label;
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)} title={summary}>
      <Icon aria-hidden className="h-3 w-3 shrink-0" />
      <span className="sr-only">Device: </span>
      <span className="truncate">{label}</span>
    </span>
  );
}

/** A dark chip for text laid over a screenshot: white on near-black stays readable on any image. */
export function OverlayChip({ children, className, title }: { children: React.ReactNode; className?: string; title?: string }) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex items-center gap-1 rounded-md bg-black/75 px-1.5 py-0.5 text-[11px] font-medium leading-4 text-white backdrop-blur",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** The "this capture was blurred before upload" flag over a screenshot. */
export function BlurredChip({ className }: { className?: string }) {
  return (
    <OverlayChip className={className} title="Blurred by the desktop app before upload">
      <EyeOff aria-hidden className="h-3 w-3" />
      Blurred
    </OverlayChip>
  );
}

const LIVE_TITLE = "The member is tracking right now — this is their latest block";

/** A session running right now, over a screenshot: the member is tracking and this is their latest block. */
export function LiveChip({ className }: { className?: string }) {
  return (
    <OverlayChip className={className} title={LIVE_TITLE}>
      <span aria-hidden className="live-dot h-1.5 w-1.5 rounded-full bg-emerald-400" />
      Live
    </OverlayChip>
  );
}

/** The same flag on a card surface (the report header, the viewer's details). */
export function LiveBadge({ className }: { className?: string }) {
  return (
    <Badge tone="success" className={cn("gap-1.5 px-2 py-0 text-[11px] leading-5", className)} title={LIVE_TITLE}>
      <span aria-hidden className="live-dot h-1.5 w-1.5 rounded-full bg-success" />
      Live now
    </Badge>
  );
}
