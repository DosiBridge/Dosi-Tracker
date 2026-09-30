"use client";

import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { MousePointerClick, Keyboard, Video, SearchX } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { surfaceVariants } from "@/components/ui/surface";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { ScreenMockView } from "@/components/screen-mock";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SegmentedControl } from "@/components/ui/toolbar";
import { ActivityDrawer, type ProjectLabel } from "@/components/activities/activity-drawer";
import { ActivityFilterBar } from "@/components/activities/activity-filter-bar";
import { CaptureThumb } from "@/components/activities/capture-thumb";
import { useSession } from "@/components/session-provider";
import { activities as demoActivities, projectById, userById } from "@/lib/tenant-data";
import { useApi } from "@/hooks/useApi";
import { agoLabel } from "@/lib/live-session";
import { formatTimeRange as timeRange } from "@/lib/activity-report";
import { mapApiActivities, thumbnailCaptureIds } from "@/lib/live-dataset";
import { applyActivityFilters, defaultActivityFilters, type ActivityFilters } from "@/lib/activity-filters";
import { scopeActivities } from "@/lib/scope";
import type { Activity } from "@/lib/types";
import { cn, formatCompact } from "@/lib/utils";

type ViewMode = "sessions" | "screens";

/** Cards rendered per page on the activity list (see visibleCount below). */
const PAGE_SIZE = 24;

/** Live rows loaded for the page: enough for every preset (the widest is 30 days / this month). */
const LIVE_WINDOW_DAYS = 31;

/**
 * Rewrite a NOW-relative date preset into explicit UTC bounds on the REAL clock.
 * The shared range presets pivot on the frozen demo NOW (2026-07-14); live activity
 * carries real timestamps, so applying the frozen window would filter everything out.
 * Custom ranges (user-typed dates) are passed through untouched.
 */
function liveEffectiveFilters(filters: ActivityFilters): ActivityFilters {
  if (filters.rangeKey === "custom") return filters;

  const to = new Date();
  const from = new Date();
  switch (filters.rangeKey) {
    case "today":
      from.setUTCHours(0, 0, 0, 0);
      break;
    case "yesterday":
      from.setUTCDate(from.getUTCDate() - 1);
      from.setUTCHours(0, 0, 0, 0);
      to.setUTCDate(to.getUTCDate() - 1);
      break;
    case "30d":
      from.setUTCDate(from.getUTCDate() - 30);
      break;
    case "month":
      from.setUTCDate(1);
      from.setUTCHours(0, 0, 0, 0);
      break;
    case "7d":
    default:
      from.setUTCDate(from.getUTCDate() - 7);
      break;
  }
  const iso = (d: Date) => d.toISOString().slice(0, 10);
  return { ...filters, rangeKey: "custom", customFrom: iso(from), customTo: iso(to) };
}

export default function ActivitiesPage() {
  return (
    <Suspense fallback={<div className="py-16 text-center text-sm text-muted-foreground">Loading activity…</div>}>
      <ActivitiesPageInner />
    </Suspense>
  );
}

function ActivitiesPageInner() {
  const { user, isLive } = useSession();
  const router = useRouter();
  const searchParams = useSearchParams();
  const [filters, setFilters] = useState<ActivityFilters>(defaultActivityFilters);
  const [selected, setSelected] = useState<Activity | null>(null);
  const [view, setView] = useState<ViewMode>(searchParams.get("view") === "screens" ? "screens" : "sessions");

  // Demo mode never touches the network. The window start is fixed per mount
  // so the endpoint (and therefore the request) is stable across renders.
  const [liveFrom] = useState(() => new Date(Date.now() - LIVE_WINDOW_DAYS * 86_400_000).toISOString());
  const { data: apiActivities, error, isLoading, refetch } = useApi<unknown[]>(
    `/api/app/activity?From=${encodeURIComponent(liveFrom)}&MaxResultCount=1000`,
    { enabled: isLive },
  );

  useEffect(() => {
    setView(searchParams.get("view") === "screens" ? "screens" : "sessions");
  }, [searchParams]);

  // Project names/colors: the session's dataset holds the tenant's REAL
  // projects in a live session and the seeded ones in demo mode.
  const resolveProject = useCallback((projectId: string): ProjectLabel | undefined => {
    const p = projectById(projectId);
    return p ? { title: p.title, color: p.color } : undefined;
  }, []);

  // Live activity, mapped with guarded JSON parsing (a malformed window list
  // from an agent must never crash the page).
  const liveActivities: Activity[] | null = useMemo(() => {
    if (!isLive || !apiActivities) return null;
    return mapApiActivities(apiActivities);
  }, [isLive, apiActivities]);

  // Live: real rows only. A failed load shows an error, never the demo rows.
  const list = useMemo(() => (isLive ? (liveActivities ?? []) : demoActivities), [isLive, liveActivities]);
  const liveLoading = isLive && isLoading && !liveActivities;
  const liveError = isLive && !!error && !liveActivities;

  function setViewMode(mode: ViewMode) {
    setView(mode);
    router.replace(mode === "screens" ? "/activities?view=screens" : "/activities", { scroll: false });
  }

  const base = useMemo(() => scopeActivities(user, list), [user, list]);
  // Live activity is timestamped on the real clock, but the shared date presets pivot on the
  // frozen demo NOW (2026-07-14). Convert the selected preset to explicit real-clock bounds so
  // real rows are not silently filtered out; demo rows keep the frozen presets.
  const effectiveFilters = useMemo(
    () => (liveActivities ? liveEffectiveFilters(filters) : filters),
    [liveActivities, filters]
  );
  const filtered = useMemo(() => applyActivityFilters(base, effectiveFilters), [base, effectiveFilters]);
  const updatedAt = liveActivities?.[0]?.endedAt;

  // A busy team produces hundreds of sessions a day. Render a page at a time
  // rather than every card at once, and reset the window whenever the result
  // set changes so a new filter always starts at the top.
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [effectiveFilters, view, base]);
  const visible = useMemo(() => filtered.slice(0, visibleCount), [filtered, visibleCount]);
  const remaining = filtered.length - visible.length;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Monitor"
        title="Activity"
        description={
          <>
            {filtered.length} tracked sessions
            {updatedAt ? ` · updated ${agoLabel(updatedAt)}` : ""}
            {!isLive && (
              <>
                {" · "}
                <span className="text-muted-foreground/80">previews are mock placeholders</span>
              </>
            )}
          </>
        }
        actions={
          <SegmentedControl
            value={view}
            onChange={setViewMode}
            options={[
              { value: "sessions", label: "Sessions" },
              { value: "screens", label: "Screens" },
            ]}
          />
        }
      />

      <ActivityFilterBar
        value={filters}
        onChange={setFilters}
        resultCount={filtered.length}
        totalCount={base.length}
        showMemberFilter={user.role === "owner" || user.role === "admin"}
      />

      {liveError && (
        <p className="text-xs text-danger">
          Couldn’t load activity from the server.{" "}
          <button
            type="button"
            onClick={() => void refetch()}
            className="underline underline-offset-2 hover:text-foreground"
          >
            Retry
          </button>
        </p>
      )}

      {isLive && !liveLoading && !liveError && base.length === 0 && (
        <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <SearchX className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No activity yet — install the desktop agent to start tracking</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Sessions appear here a few minutes after the agent uploads its first block.
            </p>
          </div>
          <Link href="/download" className="text-sm text-primary hover:underline">
            Get the desktop agent
          </Link>
        </Card>
      )}

      {!liveLoading && !liveError && base.length > 0 && filtered.length === 0 && (
        <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <SearchX className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No activities match your filters</p>
            <p className="mt-1 text-sm text-muted-foreground">Try widening the date range or clearing some filters.</p>
          </div>
          <button onClick={() => setFilters(defaultActivityFilters)} className="text-sm text-primary hover:underline">
            Reset filters
          </button>
        </Card>
      )}

      {liveLoading ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading activity…</p>
      ) : view === "sessions" ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {visible.map((a) => {
            const u = userById(a.userId);
            const p = resolveProject(a.projectId);
            return (
              // A real <button>, not a clickable <div>: opening a session is
              // this page's primary action and must be reachable by keyboard.
              // Mirrors the Screens view below.
              <button
                key={a.id}
                type="button"
                onClick={() => setSelected(a)}
                aria-label={`Open session: ${u?.name ?? "Unknown member"}, ${a.description}, ${timeRange(a)}`}
                className={cn(
                  surfaceVariants.default,
                  "group cursor-pointer overflow-hidden p-0 text-left transition-all hover:-translate-y-0.5 hover:card-elev-lg",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                )}
              >
                <div className="relative">
                  {isLive ? (
                    <LiveCaptureTile activity={a} />
                  ) : (
                    <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
                  )}
                  <div className="absolute right-2 top-2 flex gap-1">
                    {a.hasWebcam && (
                      <span className="flex h-6 w-6 items-center justify-center rounded-md bg-black/60 text-white backdrop-blur" title="Webcam opt-in">
                        <Video className="h-3.5 w-3.5" />
                      </span>
                    )}
                    {a.online && <Badge tone="success">live</Badge>}
                  </div>
                  <div className="absolute bottom-2 left-2">
                    <Badge className="bg-black/60 text-white backdrop-blur">{timeRange(a)}</Badge>
                  </div>
                </div>
                <CardContent className="p-4">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{a.description}</p>
                      <div className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                        <span className="h-2 w-2 rounded-full" style={{ background: p?.color }} />
                        <span className="truncate">{p?.title}</span>
                      </div>
                    </div>
                    <Ring value={a.productivity} size={38} stroke={4} />
                  </div>
                  <div className="mt-3 flex items-center justify-between border-t border-border pt-3">
                    <div className="flex items-center gap-2">
                      <Avatar name={u?.name ?? ""} size="sm" status={u?.status} />
                      <span className="truncate text-xs font-medium">{u?.name}</span>
                    </div>
                    <div className="flex items-center gap-3 text-xs text-muted-foreground">
                      <span className="flex items-center gap-1"><MousePointerClick className="h-3.5 w-3.5" />{formatCompact(a.mouseClicks)}</span>
                      <span className="flex items-center gap-1"><Keyboard className="h-3.5 w-3.5" />{formatCompact(a.keyboardHits)}</span>
                    </div>
                  </div>
                </CardContent>
              </button>
            );
          })}
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4">
          {visible.map((a) => {
            const u = userById(a.userId);
            const p = resolveProject(a.projectId);
            return (
              <button
                key={a.id}
                type="button"
                onClick={() => setSelected(a)}
                className="group overflow-hidden rounded-xl border border-border text-left transition-all hover:-translate-y-0.5 hover:border-primary/40"
              >
                {isLive ? (
                  <div className="relative">
                    <LiveCaptureTile activity={a} />
                  </div>
                ) : (
                  <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
                )}
                <div className="space-y-1 p-3">
                  <div className="flex items-center gap-2">
                    <Avatar name={u?.name ?? ""} size="sm" status={u?.status} />
                    <span className="truncate text-xs font-medium">{u?.name}</span>
                    <span className="ml-auto text-[11px] text-muted-foreground">{agoLabel(a.endedAt)}</span>
                  </div>
                  <div className="truncate text-[11px] text-muted-foreground">{p?.title}</div>
                </div>
              </button>
            );
          })}
        </div>
      )}

      {remaining > 0 && (
        <div className="flex flex-col items-center gap-2 pt-2">
          <p className="text-xs text-muted-foreground">
            Showing {visible.length} of {filtered.length} sessions
          </p>
          <Button variant="outline" onClick={() => setVisibleCount((n) => n + PAGE_SIZE)}>
            Show {Math.min(remaining, PAGE_SIZE)} more
          </Button>
        </div>
      )}

      <ActivityDrawer
        activity={selected}
        project={selected ? resolveProject(selected.projectId) ?? null : null}
        onClose={() => setSelected(null)}
      />
    </PageStack>
  );
}

/**
 * A live card's media: the block's real screenshot (thumb first, then the full
 * capture), or the app glyph when it has none. The main app is named over a
 * screenshot, since the image alone doesn't say what was focused. Sits inside a
 * `relative` wrapper so the card's other overlays stack on top.
 */
function LiveCaptureTile({ activity: a }: { activity: Activity }) {
  const captureIds = thumbnailCaptureIds(a);
  return (
    <>
      <CaptureThumb
        captureIds={captureIds}
        app={a.screen.app}
        color={a.screen.accent}
        className="aspect-video w-full rounded-none border-0"
      />
      {captureIds.length > 0 && (
        <div className="absolute left-2 top-2 max-w-[60%]">
          <Badge className="max-w-full bg-black/60 text-white backdrop-blur" title={a.screen.app}>
            <span className="truncate">{a.screen.app}</span>
          </Badge>
        </div>
      )}
    </>
  );
}
