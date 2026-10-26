"use client";

import { useEffect, useMemo, useState } from "react";
import {
  Activity as ActivityIcon,
  AppWindow,
  CalendarDays,
  Camera,
  ChartColumn,
  ChevronRight,
  Clock,
  Cpu,
  Download,
  Expand,
  ExternalLink,
  Gauge,
  Hourglass,
  Keyboard,
  ListTree,
  Maximize2,
  MousePointerClick,
  Timer,
  Video,
} from "lucide-react";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Drawer } from "@/components/ui/drawer";
import { ScreenMockView } from "@/components/screen-mock";
import { ActivityTimeline } from "@/components/activities/activity-timeline";
import { captureContentEndpoint } from "@/components/activities/capture-thumb";
import { NoteEditor, type NoteEditing } from "@/components/activities/note-editor";
import { BlurredChip, EndReasonChip, LiveBadge } from "@/components/activities/session-meta";
import { ViewHint, activityValue } from "@/components/activities/session-card";
import { getApi, getAuthedBlobUrl, isLiveSession } from "@/hooks/useApi";
import {
  DEFAULT_SESSION_DESCRIPTION,
  blockSeconds,
  clockSkewNote,
  formatClock,
  formatSessionDate,
  formatTimeRange,
  friendlyAppName,
  groupWindowsByApp,
  shareOfBlock,
  summarizeTimeline,
  timelineSlots,
  type AppWindowGroup,
  type SessionDetails,
} from "@/lib/activity-report";
import { appColor } from "@/lib/apps";
import { INPUT_COUNT_METRIC, formatCount } from "@/lib/metrics";
import { captureExtension, platformLabel } from "@/lib/api-contract";
import { mapApiCaptureMetas, type CaptureMeta } from "@/lib/live-dataset";
import { userById } from "@/lib/tenant-data";
import { formatUtcOffset, timeZoneLabel, zonedParts } from "@/lib/time-zone";
import type { Activity, UserStatus } from "@/lib/types";
import { cn, formatSeconds } from "@/lib/utils";

/** Real backend rows have GUID ids; mock demo rows use short slugs. */
const GUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/** A block with more windows than this starts with its apps collapsed. */
const EXPAND_ALL_WINDOW_LIMIT = 8;

export interface ProjectLabel {
  title: string;
  color: string;
}

interface CaptureView extends CaptureMeta {
  url: string;
}

/** The captures fetched for one activity id (keyed, so a stale load never shows for another block). */
interface CaptureLoad {
  key: string;
  captures: CaptureView[];
  done: boolean;
  failed: boolean;
}

const count = (value: number) => value.toLocaleString("en");

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** "screen-2026-09-29-1004.jpg" — a download name that sorts by time (the clock of `timeZone`, default the viewer's). */
function captureFileName(c: CaptureMeta, timeZone?: string): string {
  const ext = captureExtension(c.contentType);
  const at = c.capturedAt ? Date.parse(c.capturedAt) : Number.NaN;
  const pad = (v: number) => String(v).padStart(2, "0");
  let stamp = c.id;
  if (Number.isFinite(at)) {
    const d = new Date(at);
    const p = timeZone
      ? zonedParts(at, timeZone)
      : { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate(), hour: d.getHours(), minute: d.getMinutes() };
    stamp = `${p.year}-${pad(p.month)}-${pad(p.day)}-${pad(p.hour)}${pad(p.minute)}`;
  }
  return `${c.kind}-${stamp}.${ext}`;
}

/**
 * The full report for one tracked block: who/when, the real screen capture
 * (live) or a mock preview (demo), headline stats, the per-minute timeline,
 * apps & windows with the input they received, and running programs. Each
 * section renders only when its data exists, so demo rows and legacy live rows
 * (no timeline, no per-window counts) still read cleanly.
 */
export function ActivityDrawer({
  activity,
  project,
  details,
  note,
  liveNow = false,
  memberStatus,
  timeZone,
  onClose,
  onViewScreenshot,
}: {
  activity: Activity | null;
  project: ProjectLabel | null;
  /** what newer agents report about the block (note, idle, end reason, device, capture) */
  details?: SessionDetails;
  /** how the viewer may change the block's task note; omitted: read-only */
  note?: NoteEditing;
  /** the member is tracking right now and this is their latest block */
  liveNow?: boolean;
  /** the member's presence dot (default: the roster's status) */
  memberStatus?: UserStatus;
  /** the reporting zone dates and clock times read in (default: the viewer's) */
  timeZone?: string;
  onClose: () => void;
  /** open this block in the full-screen screenshot viewer; omitted when it has nothing to view */
  onViewScreenshot?: () => void;
}) {
  const a = activity;
  const u = a ? userById(a.userId) : null;
  const p = project;

  // LIVE MODE: real (GUID) activities load their actual captures from the backend.
  const activityId = a?.id ?? null;
  const live = !!activityId && GUID_RE.test(activityId) && isLiveSession();
  const [load, setLoad] = useState<CaptureLoad | null>(null);

  useEffect(() => {
    if (!activityId || !GUID_RE.test(activityId) || !isLiveSession()) return;
    const key = activityId;
    let cancelled = false;
    const urls: string[] = [];

    (async () => {
      let metas: CaptureMeta[];
      try {
        // The activity id is a PATH parameter (ABP: GET screenshots/{activityId}).
        metas = mapApiCaptureMetas(await getApi(`/api/app/activity/screenshots/${encodeURIComponent(key)}`));
      } catch {
        if (!cancelled) setLoad({ key, captures: [], done: true, failed: true });
        return;
      }
      const loaded: CaptureView[] = [];
      for (const meta of metas) {
        try {
          const url = await getAuthedBlobUrl(captureContentEndpoint(meta.id));
          if (cancelled) {
            URL.revokeObjectURL(url);
            return;
          }
          urls.push(url);
          loaded.push({ ...meta, url });
          // Show each capture as soon as it arrives rather than after the slowest one.
          setLoad({ key, captures: [...loaded], done: false, failed: false });
        } catch {
          /* skip an unreadable capture; the rest may still load */
        }
      }
      if (!cancelled) setLoad({ key, captures: loaded, done: true, failed: metas.length > 0 && loaded.length === 0 });
    })();

    // Revoke object URLs when the drawer closes, switches activity, or unmounts.
    return () => {
      cancelled = true;
      urls.forEach((url) => URL.revokeObjectURL(url));
    };
  }, [activityId]);

  const current = live && load?.key === activityId ? load : null;
  const capturesLoading = live && !current?.done;
  const screenCaptures = current?.captures.filter((c) => c.kind === "screen") ?? [];
  const webcamCaptures = current?.captures.filter((c) => c.kind === "webcam") ?? [];

  const totalSeconds = a ? blockSeconds(a) : 0;
  const slots = useMemo(() => (a ? timelineSlots(a) : []), [a]);
  const timeline = useMemo(() => summarizeTimeline(slots), [slots]);
  const apps = useMemo(() => (a ? groupWindowsByApp(a.activeWindows) : []), [a]);
  const activityPercent = a ? activityValue(a, details) : null;
  // The member's own words: the reported note, else a demo/legacy description that isn't the placeholder.
  const noteText = details ? details.note : a && a.description !== DEFAULT_SESSION_DESCRIPTION ? a.description : null;

  return (
    <Drawer open={!!a} onClose={onClose} title="Activity detail">
      {a && (
        <div className="space-y-6">
          <div className="space-y-4">
            <div className="flex items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-3">
                <Avatar name={u?.name ?? ""} size="lg" status={memberStatus ?? u?.status} />
                <div className="min-w-0">
                  <div className="truncate font-semibold">{u?.name ?? "Unknown member"}</div>
                  <div className="truncate text-xs text-muted-foreground">{u?.designation}</div>
                </div>
              </div>
              {p && (
                <Badge tone="primary" className="max-w-[45%] shrink-0">
                  <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: p.color }} aria-hidden />
                  <span className="truncate">{p.title}</span>
                </Badge>
              )}
            </div>
            <div className="rounded-xl border border-border p-3">
              <NoteEditor note={noteText} editing={note} suggest={{ activityId: a.id }} />
              <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <CalendarDays className="h-3.5 w-3.5" aria-hidden />
                  {formatSessionDate(a.startedAt, timeZone, { year: true })}
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <Clock className="h-3.5 w-3.5" aria-hidden />
                  {formatTimeRange(a, timeZone)}
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <Timer className="h-3.5 w-3.5" aria-hidden />
                  {formatSeconds(totalSeconds)}
                </span>
                <EndReasonChip reason={details?.endReason} />
                {liveNow && <LiveBadge />}
              </div>
            </div>
          </div>

          <Section icon={Camera} title="Screen capture" aside={!live && <Badge tone="muted">Mock preview</Badge>}>
            {live ? (
              screenCaptures.length > 0 ? (
                <div className="space-y-4">
                  {screenCaptures.map((c) => (
                    <CaptureFigure key={c.id} capture={c} onView={onViewScreenshot} timeZone={timeZone} />
                  ))}
                </div>
              ) : (
                <div className="flex aspect-video w-full items-center justify-center rounded-xl border border-border bg-muted/40 text-sm text-muted-foreground">
                  {capturesLoading
                    ? "Loading captures…"
                    : current?.failed
                      ? "Couldn’t load captures."
                      : "No screenshot captured"}
                </div>
              )
            ) : onViewScreenshot ? (
              <button
                type="button"
                onClick={onViewScreenshot}
                aria-label="View screenshot large"
                className="group/media relative block w-full cursor-zoom-in overflow-hidden rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
                <ViewLargeBadge />
                <ViewHint />
              </button>
            ) : (
              <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
            )}
          </Section>

          {live
            ? webcamCaptures.length > 0 && (
                <Section icon={Video} title="Webcam (opt-in)">
                  <div className="flex flex-wrap gap-2">
                    {webcamCaptures.map((c) => (
                      // eslint-disable-next-line @next/next/no-img-element
                      <img
                        key={c.id}
                        src={c.url}
                        alt={c.capturedAt ? `Webcam frame, ${formatClock(c.capturedAt, timeZone)}` : "Webcam frame"}
                        className="aspect-video w-40 rounded-lg border border-border object-cover"
                      />
                    ))}
                  </div>
                </Section>
              )
            : a.hasWebcam && (
                <Section icon={Video} title="Webcam (opt-in)">
                  <div
                    className="flex aspect-video w-40 items-center justify-center rounded-lg text-white/70"
                    style={{ background: `radial-gradient(circle at 50% 35%, ${a.screen.accent}, #0f1320)` }}
                  >
                    <Video className="h-8 w-8" />
                  </div>
                </Section>
              )}

          <div className="space-y-2">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              <Stat icon={Timer} label="Duration" value={formatSeconds(totalSeconds)} />
              {timeline.minutes > 0 && (
                <Stat icon={ActivityIcon} label="Active minutes" value={`${timeline.activeMinutes} of ${timeline.minutes} min`} />
              )}
              <Stat icon={Gauge} label="Activity" value={activityPercent === null ? "Not measured" : `${activityPercent}%`} />
              {details?.idleSeconds !== null && details?.idleSeconds !== undefined && (
                <Stat icon={Hourglass} label="Idle" value={formatSeconds(Math.min(details.idleSeconds, totalSeconds))} />
              )}
              <Stat icon={Keyboard} label="Keyboard presses" value={formatCount(a.keyboardHits, count, INPUT_COUNT_METRIC.notMeasured)} />
              <Stat icon={MousePointerClick} label="Mouse clicks" value={formatCount(a.mouseClicks, count, INPUT_COUNT_METRIC.notMeasured)} />
            </div>
            <p className="text-xs text-muted-foreground">
              {activityPercent === null
                ? "The desktop app couldn’t measure keyboard and mouse input for this block (not allowed by the project, or not available on the device), so it has no activity %."
                : "Activity is based on focused apps and input counts during this block — not a judgment score. Keys are counted, never recorded."}
            </p>
          </div>

          {details && <DeviceSection details={details} reportingZone={timeZone} startedAt={a.startedAt} />}

          {slots.length > 0 && (
            <Section icon={ChartColumn} title="Activity timeline">
              <ActivityTimeline activity={a} timeZone={timeZone} />
            </Section>
          )}

          {apps.length > 0 && (
            <Section
              icon={AppWindow}
              title="Apps & windows"
              aside={
                <span className="text-xs font-normal text-muted-foreground">
                  {apps.length} {apps.length === 1 ? "app" : "apps"} · {a.activeWindows.length}{" "}
                  {a.activeWindows.length === 1 ? "window" : "windows"}
                </span>
              }
            >
              {/* Keyed by block: a new activity starts from its own default expansion. */}
              <AppsTable key={a.id} apps={apps} totalSeconds={totalSeconds} />
            </Section>
          )}

          {a.runningPrograms.length > 0 && (
            <Section icon={ListTree} title="Running programs">
              <div className="flex flex-wrap gap-1.5">
                {a.runningPrograms.map((w, i) => (
                  <Badge key={i} tone="muted" title={w.appName}>
                    {friendlyAppName(w.appName)}
                  </Badge>
                ))}
              </div>
            </Section>
          )}
        </div>
      )}
    </Drawer>
  );
}

function Section({
  icon: Icon,
  title,
  aside,
  children,
}: {
  icon: typeof Clock;
  title: string;
  aside?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-2 flex items-center gap-2">
        <Icon className="h-4 w-4 text-muted-foreground" aria-hidden />
        <h3 className="text-sm font-medium">{title}</h3>
        {aside && <div className="ml-auto flex items-center">{aside}</div>}
      </div>
      {children}
    </section>
  );
}

function Stat({ icon: Icon, label, value }: { icon: typeof Clock; label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border p-3">
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5" aria-hidden /> {label}
      </div>
      <div className="mt-1 truncate text-sm font-semibold">{value}</div>
    </div>
  );
}

/**
 * Where the block came from: the device, its system and agent version, the
 * device's own time zone, and anything about the clock or the input worth a
 * second look. Renders nothing for a block from an older agent.
 */
function DeviceSection({
  details: d,
  reportingZone,
  startedAt,
}: {
  details: SessionDetails;
  reportingZone?: string;
  startedAt: string;
}) {
  const at = Date.parse(startedAt);
  const zone =
    d.timeZone && Number.isFinite(at)
      ? timeZoneLabel(d.timeZone, at)
      : d.utcOffsetMinutes !== null
        ? formatUtcOffset(d.utcOffsetMinutes)
        : null;
  const skew = clockSkewNote(d.clockSkewSeconds);
  const rows: { label: string; value: string }[] = [];
  if (d.deviceName) rows.push({ label: "Device", value: d.deviceName });
  const system = d.osVersion ?? platformLabel(d.platform);
  if (system) rows.push({ label: "System", value: system });
  if (d.agentVersion) rows.push({ label: "Desktop app", value: `Version ${d.agentVersion}` });
  // Only worth a row when it differs from the zone the page reads in.
  if (zone && d.timeZone !== reportingZone) rows.push({ label: "Device time zone", value: zone });
  if (skew) rows.push({ label: "Clock", value: skew });
  if (d.injectedInputEvents) {
    rows.push({
      label: "Synthetic input",
      value: `${count(d.injectedInputEvents)} software-generated ${d.injectedInputEvents === 1 ? "event" : "events"} ignored`,
    });
  }
  if (rows.length === 0) return null;
  return (
    <Section icon={Cpu} title="Device">
      <dl className="divide-y divide-border rounded-xl border border-border text-sm">
        {rows.map((r) => (
          <div key={r.label} className="flex items-baseline justify-between gap-4 px-3 py-2">
            <dt className="shrink-0 text-xs text-muted-foreground">{r.label}</dt>
            <dd className="min-w-0 truncate text-right font-medium" title={r.value}>
              {r.value}
            </dd>
          </div>
        ))}
      </dl>
    </Section>
  );
}

/** An always-visible "View large" chip on a screenshot that opens the viewer. */
function ViewLargeBadge() {
  return (
    <span
      aria-hidden
      className="pointer-events-none absolute right-2 top-2 flex items-center gap-1 rounded-md bg-black/65 px-2 py-1 text-[11px] font-medium text-white backdrop-blur"
    >
      <Expand className="h-3 w-3" /> View large
    </span>
  );
}

/**
 * One full-size capture. With `onView` the image opens the full-screen viewer
 * (and the caption keeps a new-tab link); without it, the image itself opens
 * the capture at full resolution in a new tab. Downloadable either way.
 */
function CaptureFigure({ capture: c, onView, timeZone }: { capture: CaptureView; onView?: () => void; timeZone?: string }) {
  const when = c.capturedAt ? formatClock(c.capturedAt, timeZone) : null;
  const details = [
    when ? `Captured ${when}` : "Screen capture",
    c.blurred ? "blurred" : null,
    c.sizeBytes ? formatBytes(c.sizeBytes) : null,
  ].filter(Boolean);
  const frame =
    "relative block w-full overflow-hidden rounded-xl border border-border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
  const captionLink = "inline-flex shrink-0 items-center gap-1 font-medium text-primary-strong hover:underline";
  return (
    <figure className="space-y-1.5">
      {onView ? (
        <button
          type="button"
          onClick={onView}
          aria-label={when ? `View screenshot large (captured ${when})` : "View screenshot large"}
          className={cn("group/media cursor-zoom-in", frame)}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={c.url}
            alt={when ? `Screen capture from ${when}` : "Screen capture"}
            className="aspect-video w-full object-cover object-top"
          />
          <ViewLargeBadge />
          {c.blurred && <BlurredChip className="pointer-events-none absolute left-2 top-2" />}
          <ViewHint />
        </button>
      ) : (
        <a href={c.url} target="_blank" rel="noopener noreferrer" className={cn("group", frame)}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={c.url}
            alt={when ? `Screen capture from ${when} — open full size` : "Screen capture — open full size"}
            className="aspect-video w-full object-cover object-top transition-opacity group-hover:opacity-90"
          />
          {c.blurred && <BlurredChip className="pointer-events-none absolute left-2 top-2" />}
          <span
            className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md bg-black/60 text-white opacity-0 backdrop-blur transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
            aria-hidden
          >
            <Maximize2 className="h-3.5 w-3.5" />
          </span>
        </a>
      )}
      <figcaption className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span className="truncate">{details.join(" · ")}</span>
        <span className="flex shrink-0 items-center gap-3">
          {onView && (
            <a href={c.url} target="_blank" rel="noopener noreferrer" className={captionLink}>
              <ExternalLink className="h-3.5 w-3.5" aria-hidden /> Open full size
            </a>
          )}
          <a href={c.url} download={captureFileName(c, timeZone)} className={captionLink}>
            <Download className="h-3.5 w-3.5" aria-hidden /> Download
          </a>
        </span>
      </figcaption>
    </figure>
  );
}

function AppsTable({ apps, totalSeconds }: { apps: AppWindowGroup[]; totalSeconds: number }) {
  const windowCount = apps.reduce((s, g) => s + g.windows.length, 0);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(windowCount <= EXPAND_ALL_WINDOW_LIMIT ? apps.map((g) => g.appName) : []),
  );
  const credited = apps.some((g) => g.credited);
  const secs = (s: number) => (s > 0 ? formatSeconds(s) : "—");
  const opt = (v: number | undefined) => (v === undefined ? "—" : count(v));

  function toggle(app: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(app)) next.delete(app);
      else next.add(app);
      return next;
    });
  }

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto rounded-xl border border-border">
        {/* Fixed layout: the numeric columns keep their width and the name column takes
            the rest (truncating), instead of collapsing to nothing. Narrow screens scroll. */}
        <table className="w-full min-w-[27rem] table-fixed text-sm">
          <caption className="sr-only">Focused time and input per app and window</caption>
          <colgroup>
            <col />
            <col className="w-20" />
            <col className="w-16" />
            <col className="w-[4.5rem]" />
            <col className="w-[4.5rem]" />
          </colgroup>
          <thead className="bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th scope="col" className="px-3 py-2 text-left font-medium">App</th>
              <th scope="col" className="px-3 py-2 text-right font-medium">Time</th>
              <th scope="col" className="px-3 py-2 text-right font-medium">Share</th>
              <th scope="col" className="px-3 py-2 text-right font-medium">Keys</th>
              <th scope="col" className="px-3 py-2 text-right font-medium">Clicks</th>
            </tr>
          </thead>
          {apps.map((g) => {
            const open = expanded.has(g.appName);
            const single = g.windows.length === 1;
            return (
              <tbody key={g.appName} className="border-t border-border">
                <tr>
                  <th scope="row" className="px-3 py-2 text-left font-medium">
                    {single ? (
                      <div className="flex min-w-0 items-center gap-2">
                        <AppDot app={g.appName} />
                        <div className="min-w-0">
                          <div className="truncate" title={g.appName}>
                            {g.displayName}
                          </div>
                          {g.windows[0].windowTitle && (
                            <div className="truncate text-xs font-normal text-muted-foreground" title={g.windows[0].windowTitle}>
                              {g.windows[0].windowTitle}
                            </div>
                          )}
                        </div>
                      </div>
                    ) : (
                      <button
                        type="button"
                        onClick={() => toggle(g.appName)}
                        aria-expanded={open}
                        className="flex w-full min-w-0 items-center gap-2 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        <ChevronRight
                          className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
                          aria-hidden
                        />
                        <AppDot app={g.appName} />
                        <span className="truncate" title={g.appName}>
                          {g.displayName}
                        </span>
                        <span className="hidden shrink-0 text-xs font-normal text-muted-foreground sm:inline">
                          {g.windows.length} windows
                        </span>
                      </button>
                    )}
                  </th>
                  <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums">{secs(g.seconds)}</td>
                  <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums">
                    {g.seconds > 0 ? `${shareOfBlock(g.seconds, totalSeconds)}%` : "—"}
                  </td>
                  <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums">{opt(g.keyboardHits)}</td>
                  <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums">{opt(g.mouseClicks)}</td>
                </tr>
                {!single &&
                  open &&
                  g.windows.map((w, i) => (
                    <tr key={`${w.windowTitle}-${i}`} className="text-xs text-muted-foreground">
                      <td className="py-1.5 pl-12 pr-3">
                        <div className="truncate" title={w.windowTitle || undefined}>
                          {w.windowTitle || "Untitled window"}
                        </div>
                      </td>
                      <td className="whitespace-nowrap px-3 py-1.5 text-right tabular-nums">{secs(w.seconds)}</td>
                      <td className="whitespace-nowrap px-3 py-1.5 text-right tabular-nums">
                        {w.seconds > 0 ? `${shareOfBlock(w.seconds, totalSeconds)}%` : "—"}
                      </td>
                      <td className="whitespace-nowrap px-3 py-1.5 text-right tabular-nums">{opt(w.keyboardHits)}</td>
                      <td className="whitespace-nowrap px-3 py-1.5 text-right tabular-nums">{opt(w.mouseClicks)}</td>
                    </tr>
                  ))}
              </tbody>
            );
          })}
        </table>
      </div>
      {credited && (
        <p className="text-xs text-muted-foreground">
          This block came from an agent version that didn’t report per-window time, so its focused app is credited with
          the whole block.
        </p>
      )}
    </div>
  );
}

/** The app's colour — the same one it has on every other page (lib/apps). */
function AppDot({ app }: { app: string }) {
  return <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: appColor(app) }} aria-hidden />;
}
