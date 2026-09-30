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
  Download,
  Gauge,
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
import { getApi, getAuthedBlobUrl, isLiveSession } from "@/hooks/useApi";
import {
  blockSeconds,
  formatClock,
  formatTimeRange,
  groupWindowsByApp,
  shareOfBlock,
  summarizeTimeline,
  timelineSlots,
  type AppWindowGroup,
} from "@/lib/activity-report";
import { mapApiCaptureMetas, type CaptureMeta } from "@/lib/live-dataset";
import { userById } from "@/lib/tenant-data";
import type { Activity } from "@/lib/types";
import { cn, colorFromString, formatSeconds } from "@/lib/utils";

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

/** "screen-2026-09-29-1004.jpg" — a download name that sorts by time. */
function captureFileName(c: CaptureMeta): string {
  const ext = c.contentType?.includes("png") ? "png" : c.contentType?.includes("webp") ? "webp" : "jpg";
  const d = c.capturedAt ? new Date(c.capturedAt) : null;
  const pad = (v: number) => String(v).padStart(2, "0");
  const stamp = d
    ? `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`
    : c.id;
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
  onClose,
}: {
  activity: Activity | null;
  project: ProjectLabel | null;
  onClose: () => void;
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

  return (
    <Drawer open={!!a} onClose={onClose} title="Activity detail">
      {a && (
        <div className="space-y-6">
          <div className="space-y-4">
            <div className="flex items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-3">
                <Avatar name={u?.name ?? ""} size="lg" status={u?.status} />
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
              <p className="text-sm font-medium">{a.description}</p>
              <div className="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1.5">
                  <CalendarDays className="h-3.5 w-3.5" aria-hidden />
                  {new Date(a.startedAt).toLocaleDateString("en", {
                    weekday: "short",
                    month: "short",
                    day: "numeric",
                    year: "numeric",
                  })}
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <Clock className="h-3.5 w-3.5" aria-hidden />
                  {formatTimeRange(a)}
                </span>
                <span className="inline-flex items-center gap-1.5">
                  <Timer className="h-3.5 w-3.5" aria-hidden />
                  {formatSeconds(totalSeconds)}
                </span>
              </div>
            </div>
          </div>

          <Section icon={Camera} title="Screen capture" aside={!live && <Badge tone="muted">Mock preview</Badge>}>
            {live ? (
              screenCaptures.length > 0 ? (
                <div className="space-y-4">
                  {screenCaptures.map((c) => (
                    <CaptureFigure key={c.id} capture={c} />
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
                        alt={c.capturedAt ? `Webcam frame, ${formatClock(c.capturedAt)}` : "Webcam frame"}
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
              <Stat icon={Gauge} label="Activity" value={`${a.productivity}%`} />
              <Stat icon={Keyboard} label="Keyboard presses" value={count(a.keyboardHits)} />
              <Stat icon={MousePointerClick} label="Mouse clicks" value={count(a.mouseClicks)} />
            </div>
            <p className="text-xs text-muted-foreground">
              Activity is based on focused apps and input counts during this block — not a judgment score. Keys are
              counted, never recorded.
            </p>
          </div>

          {slots.length > 0 && (
            <Section icon={ChartColumn} title="Activity timeline">
              <ActivityTimeline activity={a} />
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
                  <Badge key={i} tone="muted">
                    {w.appName}
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

/** One full-size capture: click to open it at full resolution in a new tab, or download it. */
function CaptureFigure({ capture: c }: { capture: CaptureView }) {
  const when = c.capturedAt ? formatClock(c.capturedAt) : null;
  const details = [
    when ? `Captured ${when}` : "Screen capture",
    c.blurred ? "blurred" : null,
    c.sizeBytes ? formatBytes(c.sizeBytes) : null,
  ].filter(Boolean);
  return (
    <figure className="space-y-1.5">
      <a href={c.url} target="_blank" rel="noopener noreferrer" className="group relative block overflow-hidden rounded-xl border border-border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={c.url}
          alt={when ? `Screen capture from ${when} — open full size` : "Screen capture — open full size"}
          className="aspect-video w-full object-cover object-top transition-opacity group-hover:opacity-90"
        />
        <span
          className="absolute right-2 top-2 flex h-7 w-7 items-center justify-center rounded-md bg-black/60 text-white opacity-0 backdrop-blur transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
          aria-hidden
        >
          <Maximize2 className="h-3.5 w-3.5" />
        </span>
      </a>
      <figcaption className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
        <span className="truncate">{details.join(" · ")}</span>
        <a
          href={c.url}
          download={captureFileName(c)}
          className="inline-flex shrink-0 items-center gap-1 font-medium text-primary-strong hover:underline"
        >
          <Download className="h-3.5 w-3.5" aria-hidden /> Download
        </a>
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
                          <div className="truncate">{g.appName}</div>
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
                        <span className="truncate">{g.appName}</span>
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

function AppDot({ app }: { app: string }) {
  return <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: colorFromString(app, 55, 45) }} aria-hidden />;
}
