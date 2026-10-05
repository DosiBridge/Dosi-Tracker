"use client";

import { Camera, Expand, ImageOff, Keyboard, MousePointerClick, StickyNote, Video } from "lucide-react";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { surfaceVariants } from "@/components/ui/surface";
import { ScreenMockView } from "@/components/screen-mock";
import { CaptureThumb } from "@/components/activities/capture-thumb";
import { BlurredChip, DeviceTag, EndReasonChip, IdleTime, LiveChip, OverlayChip } from "@/components/activities/session-meta";
import {
  blockSeconds,
  formatBlockLength,
  formatClock,
  formatTimeRange,
  friendlyAppName,
  hasScreenshot,
  sessionHeadline,
  type SessionDetails,
} from "@/lib/activity-report";
import { appColor } from "@/lib/apps";
import { thumbnailCaptureIds } from "@/lib/live-dataset";
import type { Activity, UserStatus } from "@/lib/types";
import { cn, formatCompact, scoreTone } from "@/lib/utils";

/* ============================================================================
 * SESSION CARDS — one tracked block in the Activity page's two views.
 *
 * A card is two actions side by side, never nested: the screenshot opens the
 * slideshow viewer, the text below opens the block's full report. A block with
 * nothing to view (no screenshot) is a single report button instead.
 * ========================================================================== */

export interface CardMember {
  name: string;
  status?: UserStatus;
}

export interface CardProject {
  title: string;
  color: string;
}

/**
 * Activity tones: the bar colour, readable text on a card, and text on a dark
 * overlay chip. The overlay chips sit on bg-black/75 over any image; these
 * light tints keep at least 4.5:1 even over a white screenshot (the base
 * status hues fall to ~2:1 there).
 */
const toneClasses = {
  success: { bar: "bg-success", text: "text-success-on-tint", onDark: "text-emerald-300" },
  warning: { bar: "bg-warning", text: "text-warning-on-tint", onDark: "text-amber-300" },
  danger: { bar: "bg-danger", text: "text-danger-on-tint", onDark: "text-red-300" },
} as const;

/** Whether a card can open the screenshot viewer: live blocks need a capture; demo blocks have a mock. */
export function isViewable(activity: Activity, live: boolean): boolean {
  return !live || hasScreenshot(activity);
}

/** The block's activity % — or null when its input wasn't measured (contract: null is "not measured", never 0%). */
export function activityValue(activity: Pick<Activity, "productivity">, details?: Pick<SessionDetails, "measured">): number | null {
  return details && !details.measured ? null : activity.productivity;
}

/** The block's screenshot (or mock / app glyph), filling a 16:9 frame. */
function SessionMedia({ activity: a, live, timeZone }: { activity: Activity; live: boolean; timeZone?: string }) {
  if (!live) {
    return (
      <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
    );
  }
  if (!hasScreenshot(a)) {
    // Say so plainly rather than leave an empty frame (screenshots off for the project, or an older agent).
    return (
      <div className="flex aspect-video w-full flex-col items-center justify-center gap-2 bg-muted/60 text-muted-foreground">
        <ImageOff className="h-6 w-6 opacity-60" aria-hidden />
        <span className="text-xs font-medium">No screenshot for this session</span>
      </div>
    );
  }
  return (
    <CaptureThumb
      captureIds={thumbnailCaptureIds(a)}
      app={a.screen.app}
      color={appColor(a.screen.app)}
      alt={`Screen capture — ${friendlyAppName(a.screen.app)}, ${formatTimeRange(a, timeZone)}`}
      className="aspect-video w-full rounded-none border-0"
    />
  );
}

/**
 * Flags over the media, as dark chips that stay readable on any picture:
 * whether the session is running now, when the screen was captured, the
 * webcam and blur flags — and, on a gallery tile, the block's time range.
 * They sit below a mock's title bar so they never cover it.
 */
function MediaOverlays({
  activity: a,
  details,
  liveNow,
  live,
  timeZone,
  showRange = false,
}: {
  activity: Activity;
  details?: SessionDetails;
  liveNow?: boolean;
  live: boolean;
  timeZone?: string;
  /** Gallery tiles have no body, so the time range goes on the picture. */
  showRange?: boolean;
}) {
  const captured = details?.capture?.capturedAt;
  return (
    <>
      <div className={cn("pointer-events-none absolute left-2 flex flex-wrap gap-1", live ? "top-2" : "top-8")}>
        {liveNow && <LiveChip />}
        {showRange && <OverlayChip className="tabular-nums">{formatTimeRange(a, timeZone)}</OverlayChip>}
      </div>
      <div className={cn("pointer-events-none absolute right-2 flex gap-1", live ? "top-2" : "top-8")}>
        {details?.capture?.blurred && <BlurredChip />}
        {a.hasWebcam && (
          <OverlayChip title="Webcam opt-in">
            <Video aria-hidden className="h-3.5 w-3.5" />
            <span className="sr-only">Webcam capture</span>
          </OverlayChip>
        )}
      </div>
      {captured && !showRange && (
        <div className="pointer-events-none absolute bottom-2 left-2">
          <OverlayChip className="tabular-nums" title="When the screen was captured">
            <Camera aria-hidden className="h-3 w-3" />
            <span className="sr-only">Captured at </span>
            {formatClock(captured, timeZone)}
          </OverlayChip>
        </div>
      )}
    </>
  );
}

/** Hover / focus hint over a viewable screenshot (inside a `group/media` element). */
export function ViewHint() {
  return (
    <span className="pointer-events-none absolute inset-0 flex items-center justify-center bg-black/0 opacity-0 transition-all duration-200 group-hover/media:bg-black/35 group-hover/media:opacity-100 group-focus-visible/media:bg-black/35 group-focus-visible/media:opacity-100">
      <span className="flex items-center gap-1.5 rounded-full bg-white/95 px-3 py-1.5 text-xs font-semibold text-neutral-900 shadow-lg">
        <Expand className="h-3.5 w-3.5" /> View
      </span>
    </span>
  );
}

/** A small square app mark: the app's initial on its accent colour. */
function AppMark({ app, color }: { app: string; color: string }) {
  return (
    <span
      aria-hidden
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-xs font-bold text-white shadow-sm"
      style={{ background: color }}
    >
      {(app.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

/** Activity % as a coloured bar with its value — or "Not measured" when the agent couldn't measure input. */
export function ActivityMeter({ value, className }: { value: number | null; className?: string }) {
  if (value === null) {
    return (
      <div className={cn("flex items-center gap-2", className)} title="The desktop app couldn't measure keyboard or mouse input for this block">
        <div aria-hidden className="h-1.5 w-full rounded-full border border-dashed border-muted-foreground/40" />
        <span className="shrink-0 whitespace-nowrap text-xs font-medium text-muted-foreground">
          <span className="sr-only">Activity </span>Not measured
        </span>
      </div>
    );
  }
  const tone = toneClasses[scoreTone(value)];
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <Progress value={value} className="h-1.5" indicatorClassName={tone.bar} aria-label={`Activity ${value}%`} />
      <span className={cn("w-9 shrink-0 text-right text-xs font-semibold tabular-nums", tone.text)}>{value}%</span>
    </div>
  );
}

/** Keys and clicks, labelled for hover and screen readers. */
function InputCounts({ activity: a }: { activity: Activity }) {
  return (
    <div className="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
      <span className="flex items-center gap-1" title={`${a.keyboardHits.toLocaleString()} key presses`}>
        <Keyboard className="h-3.5 w-3.5" aria-hidden />
        <span className="sr-only">Key presses: </span>
        {formatCompact(a.keyboardHits)}
      </span>
      <span className="flex items-center gap-1" title={`${a.mouseClicks.toLocaleString()} mouse clicks`}>
        <MousePointerClick className="h-3.5 w-3.5" aria-hidden />
        <span className="sr-only">Mouse clicks: </span>
        {formatCompact(a.mouseClicks)}
      </span>
    </div>
  );
}

export function SessionCard({
  activity: a,
  member,
  project,
  live,
  details,
  liveNow = false,
  timeZone,
  onView,
  onOpen,
}: {
  activity: Activity;
  member: CardMember;
  project?: CardProject;
  live: boolean;
  /** what newer agents report about the block (note, idle, end reason, device, capture) */
  details?: SessionDetails;
  /** the member is tracking right now and this is their latest block */
  liveNow?: boolean;
  /** the reporting zone clock times read in (default: the viewer's) */
  timeZone?: string;
  /** open the slideshow at this block; omitted when there is nothing to view */
  onView?: () => void;
  onOpen: () => void;
}) {
  const appName = friendlyAppName(a.screen.app);
  const headline = sessionHeadline(a);
  const noted = !!details?.note && headline === details.note;
  const range = formatTimeRange(a, timeZone);

  const body = (
    <div className="flex flex-1 flex-col gap-3 p-4">
      <div className="flex items-start gap-3">
        <AppMark app={appName} color={appColor(a.screen.app)} />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-2">
            <p className="truncate text-sm font-semibold" title={a.screen.app}>
              {appName}
            </p>
            <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{formatBlockLength(blockSeconds(a))}</span>
          </div>
          <p className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground" title={headline}>
            {noted && (
              <>
                <StickyNote aria-hidden className="h-3 w-3 shrink-0 text-primary" />
                <span className="sr-only">Task note: </span>
              </>
            )}
            <span className={cn("truncate", noted && "text-foreground")}>{headline}</span>
          </p>
        </div>
      </div>
      <ActivityMeter value={activityValue(a, details)} />
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[11px] text-muted-foreground">
        <span className="tabular-nums">{range}</span>
        <IdleTime seconds={details?.idleSeconds} />
        <EndReasonChip reason={details?.endReason} />
        {details && <DeviceTag details={details} className="max-w-[10rem]" />}
      </div>
      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border pt-3">
        <div className="flex min-w-0 items-center gap-2">
          <Avatar name={member.name} size="sm" status={member.status} />
          <div className="min-w-0">
            <p className="truncate text-xs font-medium">{member.name}</p>
            {project && (
              <p className="flex items-center gap-1 truncate text-[11px] text-muted-foreground">
                <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: project.color }} />
                <span className="truncate">{project.title}</span>
              </p>
            )}
          </div>
        </div>
        <InputCounts activity={a} />
      </div>
    </div>
  );

  const reportLabel = `Open session: ${member.name}, ${a.description}, ${range}${liveNow ? ", live now" : ""}`;
  const captured = details?.capture?.capturedAt;
  const viewLabel = `View screenshot: ${member.name}, ${range}${captured ? `, captured ${formatClock(captured, timeZone)}` : ""}${
    details?.capture?.blurred ? ", blurred" : ""
  }`;
  const focusRing =
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";
  const overlays = <MediaOverlays activity={a} details={details} liveNow={liveNow} live={live} timeZone={timeZone} />;

  return (
    <div
      className={cn(
        surfaceVariants.default,
        "group flex flex-col overflow-hidden p-0 transition-all hover:card-elev-lg motion-safe:hover:-translate-y-0.5",
      )}
    >
      {onView ? (
        <>
          <button
            type="button"
            onClick={onView}
            aria-label={viewLabel}
            className={cn("group/media relative block w-full overflow-hidden", focusRing)}
          >
            <SessionMedia activity={a} live={live} timeZone={timeZone} />
            {overlays}
            <ViewHint />
          </button>
          <button type="button" onClick={onOpen} aria-label={reportLabel} className={cn("flex flex-1 flex-col text-left", focusRing)}>
            {body}
          </button>
        </>
      ) : (
        <button type="button" onClick={onOpen} aria-label={reportLabel} className={cn("flex flex-1 flex-col text-left", focusRing)}>
          <div className="relative">
            <SessionMedia activity={a} live={live} timeZone={timeZone} />
            {overlays}
          </div>
          {body}
        </button>
      )}
    </div>
  );
}

/**
 * A gallery tile for the Screens view: the screenshot with just enough on it —
 * when, how active, which app — to scan a day image by image. Opens the viewer.
 */
export function ScreenTile({
  activity: a,
  member,
  live,
  details,
  liveNow = false,
  timeZone,
  onView,
}: {
  activity: Activity;
  member: CardMember;
  live: boolean;
  details?: SessionDetails;
  liveNow?: boolean;
  timeZone?: string;
  onView: () => void;
}) {
  const appName = friendlyAppName(a.screen.app);
  const value = activityValue(a, details);
  const tone = value === null ? null : toneClasses[scoreTone(value)];
  const captured = details?.capture?.capturedAt;
  return (
    <button
      type="button"
      onClick={onView}
      aria-label={`View screenshot: ${member.name}, ${formatTimeRange(a, timeZone)}${captured ? `, captured ${formatClock(captured, timeZone)}` : ""}${
        liveNow ? ", live now" : ""
      }`}
      className={cn(
        "group/media relative block overflow-hidden rounded-xl border border-border bg-muted text-left transition-all",
        "hover:border-primary/40 hover:card-elev-lg motion-safe:hover:-translate-y-0.5",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
      )}
    >
      <SessionMedia activity={a} live={live} timeZone={timeZone} />
      <MediaOverlays activity={a} details={details} liveNow={liveNow} live={live} timeZone={timeZone} showRange />
      <ViewHint />
      <span className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 bg-gradient-to-t from-black/85 via-black/45 to-transparent px-2.5 pb-2 pt-8 text-white">
        <span className="min-w-0">
          <span className="block truncate text-xs font-semibold" title={a.screen.app}>
            {appName}
          </span>
          <span className="block truncate text-[11px] text-white/80">{member.name}</span>
        </span>
        <span
          className={cn(
            "shrink-0 rounded-md bg-black/75 px-1.5 py-0.5 text-[11px] font-semibold tabular-nums backdrop-blur",
            tone ? tone.onDark : "text-white",
          )}
          title={value === null ? "Activity not measured" : `Activity ${value}%`}
        >
          {value === null ? "Not measured" : `${value}%`}
        </span>
      </span>
      {tone && value !== null && (
        <span className="pointer-events-none absolute inset-x-0 bottom-0 h-1 bg-black/40">
          <span className={cn("block h-full", tone.bar)} style={{ width: `${Math.min(100, Math.max(0, value))}%` }} />
        </span>
      )}
    </button>
  );
}
