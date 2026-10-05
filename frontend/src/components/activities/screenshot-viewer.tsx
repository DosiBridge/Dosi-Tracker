"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Camera,
  ChevronLeft,
  ChevronRight,
  Download,
  FileText,
  ImageOff,
  Info,
  Keyboard,
  Loader2,
  MousePointerClick,
  PanelRight,
  Pause,
  Play,
  X,
} from "lucide-react";
import { Avatar } from "@/components/ui/avatar";
import { useDialogFocus } from "@/components/ui/use-dialog-focus";
import { ScreenMockView } from "@/components/screen-mock";
import { CaptureThumb, captureContentEndpoint } from "@/components/activities/capture-thumb";
import { ActivityTimeline } from "@/components/activities/activity-timeline";
import { NoteEditor, type NoteEditing } from "@/components/activities/note-editor";
import { ActivityMeter, activityValue, type CardMember, type CardProject } from "@/components/activities/session-card";
import { BlurredChip, DeviceTag, EndReasonChip, IdleTime, LiveBadge, OverlayChip } from "@/components/activities/session-meta";
import { getAuthedBlob } from "@/hooks/useApi";
import { captureExtension } from "@/lib/api-contract";
import {
  blockSeconds,
  formatBlockLength,
  formatClock,
  formatSessionDate,
  formatTimeRange,
  friendlyAppName,
  groupWindowsByApp,
  sessionHeadline,
  shareOfBlock,
  type SessionDetails,
} from "@/lib/activity-report";
import { appColor } from "@/lib/apps";
import { thumbnailCaptureIds } from "@/lib/live-dataset";
import type { Activity } from "@/lib/types";
import { cn } from "@/lib/utils";

/* ============================================================================
 * SCREENSHOT VIEWER — go through a list of blocks image by image.
 *
 * A full-screen slideshow over the Activity page's current (filtered, sorted)
 * list: the screenshot large, prev/next by button, arrow keys or swipe, a
 * filmstrip to jump, an optional auto-play, and a side panel with what the
 * block holds (note, activity, input, apps, per-minute timeline). Below
 * 1024px the panel is a bottom sheet behind a Details button. "Full report"
 * hands over to the activity drawer.
 *
 * Images: the small thumb shows at once (blurred) while the full capture
 * loads; neighbours are fetched ahead so stepping is instant. Everything the
 * viewer downloaded is released when it closes, downloads still in flight are
 * abandoned, and one that lands after the close is released on arrival.
 * ========================================================================== */

/** Seconds per slide while auto-playing. */
const SLIDESHOW_MS = 3000;
/** Filmstrip tiles rendered around the current one (the rest are skipped for speed). */
const STRIP_RADIUS = 30;
/** Minimum horizontal travel (px) for a touch swipe to change slides. */
const SWIPE_PX = 50;

export interface ViewerItem {
  activity: Activity;
  member: CardMember;
  project?: CardProject;
  /** what newer agents report about the block (note, idle, end reason, device, capture) */
  details?: SessionDetails;
  /** the member is tracking right now and this is their latest block */
  liveNow?: boolean;
  /** how the viewer may change the block's task note; omitted: read-only */
  note?: NoteEditing;
}

function captureOf(activity: Activity, kind: "screen" | "thumb"): string | undefined {
  return activity.captures?.find((c) => c.kind === kind)?.id;
}

/**
 * Download-once cache of capture object URLs. On close every URL is revoked,
 * downloads still running are aborted, and a download that resolves after the
 * close is revoked as it lands — nothing outlives the viewer.
 */
function useCaptureCache() {
  const promises = useRef(new Map<string, Promise<CaptureFile>>());
  const urls = useRef<string[]>([]);
  const controller = useRef<AbortController | null>(null);
  const closed = useRef(false);

  useEffect(() => {
    closed.current = false;
    const created = urls.current;
    const pending = promises.current;
    return () => {
      closed.current = true;
      controller.current?.abort();
      controller.current = null;
      created.forEach((url) => URL.revokeObjectURL(url));
      created.length = 0;
      pending.clear();
    };
  }, []);

  return useCallback((id: string): Promise<CaptureFile> => {
    let p = promises.current.get(id);
    if (!p) {
      controller.current ??= new AbortController();
      p = getAuthedBlob(captureContentEndpoint(id), { signal: controller.current.signal }).then((file) => {
        if (closed.current) {
          // The viewer closed while this was on its way: release it now, nobody will show it.
          URL.revokeObjectURL(file.url);
          throw new DOMException("The viewer closed", "AbortError");
        }
        urls.current.push(file.url);
        return file;
      });
      // A failed download may be retried the next time the slide is shown.
      p.catch(() => promises.current.delete(id));
      promises.current.set(id, p);
    }
    return p;
  }, []);
}

/** A downloaded capture: its object URL and the content type the server sent. */
type CaptureFile = { url: string; type: string };

type Slide = { key: string; thumb?: string; full?: string; fullType?: string; failed: boolean };

/** The current slide's image: thumb first (blurred), then the full capture. */
function SlideImage({
  activity,
  details,
  timeZone,
  load,
  zoomed,
  onToggleZoom,
}: {
  activity: Activity;
  details?: SessionDetails;
  timeZone?: string;
  load: (id: string) => Promise<CaptureFile>;
  zoomed: boolean;
  onToggleZoom: () => void;
}) {
  const key = activity.id;
  const [slide, setSlide] = useState<Slide>({ key, failed: false });

  useEffect(() => {
    let cancelled = false;
    const thumbId = captureOf(activity, "thumb");
    const screenId = captureOf(activity, "screen");
    const update = (patch: Partial<Slide>) =>
      !cancelled && setSlide((s) => (s.key === key ? { ...s, ...patch } : { key, failed: false, ...patch }));

    if (thumbId) load(thumbId).then((thumb) => update({ thumb: thumb.url }), () => {});
    if (screenId) {
      load(screenId).then(
        (full) => update({ full: full.url, fullType: full.type }),
        () => update({ failed: !thumbId }),
      );
    } else if (!thumbId) {
      update({ failed: true });
    }
    return () => {
      cancelled = true;
    };
  }, [activity, key, load]);

  const current = slide.key === key ? slide : { key, failed: false };
  const src = current.full ?? current.thumb;
  const captured = details?.capture?.capturedAt;
  const alt = `Screenshot — ${friendlyAppName(activity.screen.app)}, ${formatTimeRange(activity, timeZone)}${
    captured ? `, captured ${formatClock(captured, timeZone)}` : ""
  }`;

  if (!src) {
    return (
      <div className="flex aspect-video w-full max-w-5xl flex-col items-center justify-center gap-3 rounded-xl bg-white/5 text-sm text-white/60">
        {current.failed ? (
          <>
            <ImageOff className="h-8 w-8" aria-hidden /> Screenshot unavailable
          </>
        ) : (
          <>
            <Loader2 className="h-7 w-7 motion-safe:animate-spin" aria-hidden /> Loading screenshot…
          </>
        )}
      </div>
    );
  }

  const loadingFull = !current.full && !current.failed;
  return (
    <div className={cn("flex h-full w-full", zoomed ? "items-start justify-start overflow-auto" : "items-center justify-center")}>
      {/* The wrapper hugs the image, so its badges sit on the picture at any screen size;
          it is capped at the stage's width, which the image's own max-width then follows. */}
      <div className={cn("relative", zoomed ? "shrink-0" : "min-w-0 max-w-full")}>
        {/* A bearer-authed blob: next/image can't optimize it, so a plain <img> is right here. */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          key={src}
          src={src}
          alt={alt}
          onClick={onToggleZoom}
          className={cn(
            "block animate-fade-in select-none rounded-lg shadow-2xl",
            // Fit between the top bar and the filmstrip; zoom shows actual pixels.
            zoomed ? "max-w-none cursor-zoom-out" : "max-h-[calc(100dvh-13rem)] max-w-full cursor-zoom-in object-contain",
            loadingFull && "blur-[2px]",
          )}
          draggable={false}
        />
        <span className="pointer-events-none absolute left-3 top-3 flex flex-wrap gap-1">
          {captured && (
            <OverlayChip className="tabular-nums" title="When the screen was captured">
              <Camera aria-hidden className="h-3 w-3" />
              <span className="sr-only">Captured at </span>
              {formatClock(captured, timeZone)}
            </OverlayChip>
          )}
          {details?.capture?.blurred && <BlurredChip />}
        </span>
        {loadingFull && (
          <span className="pointer-events-none absolute right-3 top-3 flex items-center gap-1.5 rounded-full bg-black/75 px-2.5 py-1 text-[11px] text-white">
            <Loader2 className="h-3 w-3 motion-safe:animate-spin" aria-hidden /> Full size…
          </span>
        )}
        {current.full && (
          <a
            href={current.full}
            download={`screenshot-${activity.startedAt.slice(0, 16).replace(/[:T]/g, "-")}.${captureExtension(current.fullType)}`}
            className="absolute bottom-3 right-3 flex items-center gap-1.5 rounded-full bg-black/75 px-3 py-1.5 text-xs font-medium text-white backdrop-blur transition hover:bg-black/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
          >
            <Download className="h-3.5 w-3.5" aria-hidden /> Download
          </a>
        )}
      </div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  disabled,
  pressed,
  className,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  pressed?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      className={cn(
        "flex h-9 w-9 items-center justify-center rounded-full text-white/85 transition hover:bg-white/15 hover:text-white",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white disabled:pointer-events-none disabled:opacity-30",
        pressed && "bg-white/15 text-white",
        className,
      )}
    >
      {children}
    </button>
  );
}

/** What one block holds, beside the image (or in the bottom sheet below 1024px). */
function SlideDetails({ item, timeZone, onOpenDetails }: { item: ViewerItem; timeZone?: string; onOpenDetails: () => void }) {
  const a = item.activity;
  const d = item.details;
  const total = blockSeconds(a);
  const apps = groupWindowsByApp(a.activeWindows).slice(0, 5);
  const captured = d?.capture?.capturedAt;
  return (
    <div className="space-y-5 p-5">
      <div className="flex items-center gap-3">
        <Avatar name={item.member.name} status={item.member.status} />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold">{item.member.name}</p>
          {item.project && (
            <p className="flex items-center gap-1.5 truncate text-xs text-muted-foreground">
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: item.project.color }} />
              {item.project.title}
            </p>
          )}
        </div>
        {item.liveNow && <LiveBadge className="shrink-0" />}
      </div>

      {/* Keyed by block: a draft never follows the viewer to another screenshot. */}
      {(d?.note || item.note?.canEdit) && <NoteEditor key={a.id} note={d?.note ?? null} editing={item.note} />}

      <div className="rounded-xl border border-border p-3 text-sm">
        <p className="font-medium">{formatSessionDate(a.startedAt, timeZone)}</p>
        <p className="text-muted-foreground">
          {formatTimeRange(a, timeZone)} · {formatBlockLength(total)}
        </p>
        {(captured || d?.capture?.blurred) && (
          <p className="mt-1 flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
            <Camera aria-hidden className="h-3.5 w-3.5" />
            {captured ? `Screen captured ${formatClock(captured, timeZone)}` : "Screen captured"}
            {d?.capture?.blurred ? " · blurred before upload" : ""}
          </p>
        )}
        {d && (!!d.idleSeconds || !!d.endReason || !!d.deviceName || !!d.platform) && (
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-muted-foreground">
            <IdleTime seconds={d.idleSeconds} />
            <EndReasonChip reason={d.endReason} />
            <DeviceTag details={d} className="max-w-full" />
          </div>
        )}
      </div>

      <div>
        <p className="mb-1.5 text-xs font-medium text-muted-foreground">Activity</p>
        <ActivityMeter value={activityValue(a, d)} />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div className="rounded-xl border border-border p-3">
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Keyboard className="h-3.5 w-3.5" aria-hidden /> Key presses
          </p>
          <p className="mt-1 text-lg font-semibold tabular-nums">{a.keyboardHits.toLocaleString()}</p>
        </div>
        <div className="rounded-xl border border-border p-3">
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <MousePointerClick className="h-3.5 w-3.5" aria-hidden /> Mouse clicks
          </p>
          <p className="mt-1 text-lg font-semibold tabular-nums">{a.mouseClicks.toLocaleString()}</p>
        </div>
      </div>

      {apps.length > 0 && (
        <div>
          <p className="mb-2 text-xs font-medium text-muted-foreground">Apps used</p>
          <ul className="space-y-2.5">
            {apps.map((app) => {
              const share = shareOfBlock(app.seconds, total);
              return (
                <li key={app.appName}>
                  <div className="flex items-baseline justify-between gap-2 text-sm">
                    <span className="truncate font-medium" title={app.appName}>
                      {app.displayName}
                    </span>
                    <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                      {app.credited ? "—" : formatBlockLength(app.seconds)} · {share}%
                    </span>
                  </div>
                  <div className="mt-1 h-1 overflow-hidden rounded-full bg-muted">
                    <div className="h-full rounded-full" style={{ width: `${share}%`, background: appColor(app.appName) }} />
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {(a.timeline?.length ?? 0) > 0 && (
        <div>
          <p className="mb-2 text-xs font-medium text-muted-foreground">Minute by minute</p>
          <ActivityTimeline activity={a} timeZone={timeZone} />
        </div>
      )}

      <button
        type="button"
        onClick={onOpenDetails}
        className="flex w-full items-center justify-center gap-2 rounded-xl border border-border px-3 py-2.5 text-sm font-medium transition hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <FileText className="h-4 w-4" aria-hidden /> Full session report
      </button>
    </div>
  );
}

export function ScreenshotViewer({
  items,
  index,
  live,
  timeZone,
  onIndexChange,
  onClose,
  onOpenDetails,
}: {
  items: ViewerItem[];
  /** the slide shown; null = closed */
  index: number | null;
  live: boolean;
  /** the reporting zone dates and clock times read in (default: the viewer's) */
  timeZone?: string;
  onIndexChange: (index: number) => void;
  onClose: () => void;
  onOpenDetails: (activity: Activity) => void;
}) {
  const open = index !== null && items.length > 0;
  const dialogRef = useDialogFocus<HTMLDivElement>(open);
  const load = useCaptureCache();
  const [playing, setPlaying] = useState(false);
  const [showDetails, setShowDetails] = useState(true);
  // Below 1024px the details live in a bottom sheet, opened on demand.
  const [sheetOpen, setSheetOpen] = useState(false);
  const [zoomed, setZoomed] = useState(false);
  const stripRef = useRef<HTMLDivElement>(null);
  const sheetRef = useRef<HTMLDivElement>(null);
  const sheetButtonRef = useRef<HTMLButtonElement>(null);
  const swipeFrom = useRef<number | null>(null);
  const sheetId = useId();

  const count = items.length;
  const i = open ? Math.min(Math.max(index, 0), count - 1) : 0;
  const item = open ? items[i] : undefined;

  const go = useCallback(
    (next: number) => {
      if (count === 0) return;
      const clamped = Math.min(Math.max(next, 0), count - 1);
      if (clamped !== i) {
        setZoomed(false);
        onIndexChange(clamped);
      }
    },
    [count, i, onIndexChange],
  );

  const closeSheet = useCallback(() => {
    setSheetOpen(false);
    // Back to the button that opened it.
    window.requestAnimationFrame(() => sheetButtonRef.current?.focus());
  }, []);

  // Keyboard: arrows step, Home/End jump, Space plays, Escape closes (the details sheet first).
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      const target = e.target instanceof HTMLElement ? e.target : null;
      // Typing in a field (the task note) keeps every key for the field.
      if (target && (target.isContentEditable || target.closest("input, textarea, select"))) return;
      const onControl = !!target?.closest("button, a");
      if (e.key === "Escape") {
        if (sheetOpen) closeSheet();
        else onClose();
      } else if (e.key === "ArrowRight") go(i + 1);
      else if (e.key === "ArrowLeft") go(i - 1);
      else if (e.key === "Home") go(0);
      else if (e.key === "End") go(count - 1);
      else if (e.key === " " && !onControl) setPlaying((p) => !p);
      else return;
      e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = "";
    };
  }, [open, go, i, count, onClose, sheetOpen, closeSheet]);

  // The sheet takes focus when it opens, so a keyboard or screen-reader user lands in it.
  useEffect(() => {
    if (sheetOpen) sheetRef.current?.focus();
  }, [sheetOpen]);

  // Auto-play: one slide every SLIDESHOW_MS, stopping on the last one.
  useEffect(() => {
    if (!open || !playing) return;
    const timer = window.setTimeout(() => {
      go(i + 1);
      if (i + 1 >= count - 1) setPlaying(false);
    }, SLIDESHOW_MS);
    return () => window.clearTimeout(timer);
  }, [open, playing, i, count, go]);

  // Fetch the neighbours' full captures ahead, so stepping is instant.
  useEffect(() => {
    if (!open || !live) return;
    for (const n of [i + 1, i - 1]) {
      const id = items[n] && captureOf(items[n].activity, "screen");
      if (id) load(id).catch(() => {});
    }
  }, [open, live, i, items, load]);

  // Keep the current filmstrip tile in view.
  useEffect(() => {
    if (!open) return;
    stripRef.current
      ?.querySelector<HTMLElement>('[aria-current="true"]')
      ?.scrollIntoView?.({ block: "nearest", inline: "center", behavior: "smooth" });
  }, [open, i]);

  const strip = useMemo(() => {
    const from = Math.max(0, i - STRIP_RADIUS);
    return items.slice(from, i + STRIP_RADIUS + 1).map((it, k) => ({ it, n: from + k }));
  }, [items, i]);

  if (!open || !item) return null;
  const a = item.activity;
  const appName = friendlyAppName(a.screen.app);
  const value = activityValue(a, item.details);

  return createPortal(
    <div
      ref={dialogRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label="Screenshot viewer"
      className="fixed inset-0 z-50 flex flex-col bg-neutral-950 text-white outline-none animate-fade-in"
    >
      <p className="sr-only" aria-live="polite">
        Screenshot {i + 1} of {count}: {item.member.name}, {formatTimeRange(a, timeZone)}, {appName}
      </p>

      {/* Top bar */}
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-white/10 px-3 sm:px-5">
        <span className="shrink-0 rounded-md bg-white/10 px-2 py-1 text-xs font-semibold tabular-nums">
          {i + 1} / {count}
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold">
            {appName}
            <span className="font-normal text-white/60"> — {sessionHeadline(a)}</span>
          </p>
          <p className="truncate text-xs text-white/70">
            {item.member.name}
            {item.project ? ` · ${item.project.title}` : ""} · {formatSessionDate(a.startedAt, timeZone)} ·{" "}
            {formatTimeRange(a, timeZone)} · {value === null ? "activity not measured" : `${value}% active`}
          </p>
        </div>
        <span className="hidden shrink-0 text-[11px] text-white/45 xl:block" aria-hidden>
          ← → browse · Space play · Esc close
        </span>
        <IconButton
          label={playing ? "Pause slideshow" : "Play slideshow"}
          onClick={() => setPlaying((p) => !p)}
          pressed={playing}
          disabled={count < 2}
        >
          {playing ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}
        </IconButton>
        <IconButton
          label={showDetails ? "Hide details" : "Show details"}
          onClick={() => setShowDetails((s) => !s)}
          pressed={showDetails}
          className="hidden lg:flex"
        >
          <PanelRight className="h-4 w-4" />
        </IconButton>
        {/* Below 1024px there is no room for the side panel: the same details open as a bottom sheet. */}
        <button
          ref={sheetButtonRef}
          type="button"
          onClick={() => (sheetOpen ? closeSheet() : setSheetOpen(true))}
          aria-expanded={sheetOpen}
          aria-controls={sheetOpen ? sheetId : undefined}
          className={cn(
            "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full px-3 text-xs font-medium text-white/85 transition hover:bg-white/15 hover:text-white lg:hidden",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white",
            sheetOpen && "bg-white/15 text-white",
          )}
        >
          <Info aria-hidden className="h-4 w-4" />
          Details
        </button>
        <IconButton label="Close viewer" onClick={onClose}>
          <X className="h-5 w-5" />
        </IconButton>
      </header>

      <div className="flex min-h-0 flex-1">
        {/* Stage */}
        <div
          className="relative flex min-w-0 flex-1 items-center justify-center p-3 sm:p-6"
          onPointerDown={(e) => {
            swipeFrom.current = e.pointerType === "touch" ? e.clientX : null;
          }}
          onPointerUp={(e) => {
            if (swipeFrom.current === null) return;
            const dx = e.clientX - swipeFrom.current;
            swipeFrom.current = null;
            if (Math.abs(dx) >= SWIPE_PX) go(dx < 0 ? i + 1 : i - 1);
          }}
        >
          {live ? (
            <SlideImage
              activity={a}
              details={item.details}
              timeZone={timeZone}
              load={load}
              zoomed={zoomed}
              onToggleZoom={() => setZoomed((z) => !z)}
            />
          ) : (
            <ScreenMockView
              key={a.id}
              screen={a.screen}
              title={a.activeWindows[0]?.windowTitle}
              className="aspect-video w-full max-w-5xl animate-fade-in shadow-2xl"
            />
          )}

          <IconButton
            label="Previous screenshot"
            onClick={() => go(i - 1)}
            disabled={i === 0}
            className="absolute left-2 top-1/2 h-11 w-11 -translate-y-1/2 bg-black/50 backdrop-blur sm:left-4"
          >
            <ChevronLeft className="h-6 w-6" />
          </IconButton>
          <IconButton
            label="Next screenshot"
            onClick={() => go(i + 1)}
            disabled={i === count - 1}
            className="absolute right-2 top-1/2 h-11 w-11 -translate-y-1/2 bg-black/50 backdrop-blur sm:right-4"
          >
            <ChevronRight className="h-6 w-6" />
          </IconButton>
        </div>

        {/* Details panel */}
        {showDetails && (
          <aside className="hidden w-80 shrink-0 overflow-y-auto border-l border-border bg-card text-foreground lg:block">
            <SlideDetails item={item} timeZone={timeZone} onOpenDetails={() => onOpenDetails(a)} />
          </aside>
        )}
      </div>

      {/* Details sheet (below 1024px) */}
      {sheetOpen && (
        <>
          <div aria-hidden className="fixed inset-0 z-10 bg-black/50 lg:hidden" onClick={closeSheet} />
          <div
            ref={sheetRef}
            id={sheetId}
            role="region"
            aria-label="Screenshot details"
            tabIndex={-1}
            className="fixed inset-x-0 bottom-0 z-20 max-h-[75dvh] overflow-y-auto overscroll-contain rounded-t-2xl border-t border-border bg-card text-foreground shadow-2xl outline-none animate-fade-in lg:hidden"
          >
            <div className="sticky top-0 z-10 flex items-center justify-between gap-3 border-b border-border bg-card/95 px-5 py-3 backdrop-blur">
              <span aria-hidden className="absolute left-1/2 top-1.5 h-1 w-10 -translate-x-1/2 rounded-full bg-muted-foreground/30" />
              <h2 className="text-sm font-semibold">Screenshot details</h2>
              <button
                type="button"
                onClick={closeSheet}
                aria-label="Close details"
                className="flex h-8 w-8 items-center justify-center rounded-full text-muted-foreground transition hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <SlideDetails item={item} timeZone={timeZone} onOpenDetails={() => onOpenDetails(a)} />
          </div>
        </>
      )}

      {/* Filmstrip */}
      <div ref={stripRef} className="flex shrink-0 gap-2 overflow-x-auto border-t border-white/10 px-3 py-2.5 sm:px-5" aria-label="All screenshots">
        {strip.map(({ it, n }) => (
          <button
            key={it.activity.id}
            type="button"
            onClick={() => go(n)}
            aria-label={`Screenshot ${n + 1}: ${formatClock(it.activity.startedAt, timeZone)}`}
            aria-current={n === i ? "true" : undefined}
            className={cn(
              "group/strip relative w-28 shrink-0 overflow-hidden rounded-md text-left outline-none transition sm:w-32",
              n === i ? "ring-2 ring-primary ring-offset-2 ring-offset-neutral-950" : "opacity-60 hover:opacity-100 focus-visible:opacity-100",
              "focus-visible:ring-2 focus-visible:ring-white",
            )}
          >
            {live ? (
              <CaptureThumb
                captureIds={thumbnailCaptureIds(it.activity)}
                app={it.activity.screen.app}
                color={appColor(it.activity.screen.app)}
                alt=""
                className="aspect-video w-full rounded-none border-0"
              />
            ) : (
              <ScreenMockView screen={it.activity.screen} className="aspect-video w-full rounded-none" />
            )}
            <span className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 to-transparent px-1.5 pb-0.5 pt-3 text-[10px] font-medium tabular-nums">
              {formatClock(it.activity.startedAt, timeZone)}
            </span>
          </button>
        ))}
      </div>
    </div>,
    document.body,
  );
}
