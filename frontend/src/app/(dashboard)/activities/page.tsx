"use client";

import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import {
  MousePointerClick,
  Keyboard,
  Video,
  Clock,
  AppWindow,
  ListTree,
  Camera,
  SearchX,
} from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Drawer } from "@/components/ui/drawer";
import { ScreenMockView } from "@/components/screen-mock";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SegmentedControl } from "@/components/ui/toolbar";
import { ActivityFilterBar } from "@/components/activities/activity-filter-bar";
import { useSession } from "@/components/session-provider";
import { projectById, userById } from "@/lib/tenant-data";
import { getApi, getAuthedBlobUrl, useApi } from "@/hooks/useApi";
import { applyActivityFilters, defaultActivityFilters, type ActivityFilters } from "@/lib/activity-filters";
import { scopeActivities } from "@/lib/scope";
import type { Activity } from "@/lib/types";
import { cn, formatCompact } from "@/lib/utils";

type ViewMode = "sessions" | "screens";

/** Real backend rows have GUID ids; mock demo rows use short slugs. */
const GUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const isGuidId = (id: string) => GUID_RE.test(id);

interface LiveProjectInfo {
  id: string;
  title: string;
  color?: string | null;
}

interface ProjectLabel {
  title: string;
  color: string;
}

interface CaptureView {
  id: string;
  kind: "screen" | "webcam";
  url: string;
}

function timeRange(a: Activity) {
  const fmt = (iso: string) =>
    new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit" });
  return `${fmt(a.startedAt)} – ${fmt(a.endedAt)}`;
}
function agoLabel(iso: string) {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

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
  const { user } = useSession();
  const router = useRouter();
  const searchParams = useSearchParams();
  const [filters, setFilters] = useState<ActivityFilters>(defaultActivityFilters);
  const [selected, setSelected] = useState<Activity | null>(null);
  const [view, setView] = useState<ViewMode>(searchParams.get("view") === "screens" ? "screens" : "sessions");

  const { data: apiActivities, isLoading } = useApi<any[]>('/api/app/activity?MaxResultCount=200');
  const [liveProjects, setLiveProjects] = useState<LiveProjectInfo[]>([]);

  useEffect(() => {
    setView(searchParams.get("view") === "screens" ? "screens" : "sessions");
  }, [searchParams]);

  // LIVE MODE: resolve project names/colors from the real API; demo mode keeps the mock lookup.
  useEffect(() => {
    if (typeof window !== "undefined" && localStorage.getItem("dosi-token")) {
      getApi("/api/app/project")
        .then((list) => setLiveProjects(Array.isArray(list) ? list : []))
        .catch(() => setLiveProjects([]));
    }
  }, []);

  const liveProjectById = useMemo(() => {
    const map = new Map<string, LiveProjectInfo>();
    for (const p of liveProjects) map.set(p.id, p);
    return map;
  }, [liveProjects]);

  const resolveProject = useCallback(
    (projectId: string): ProjectLabel | undefined => {
      const live = liveProjectById.get(projectId);
      if (live) return { title: live.title, color: live.color || "#006bff" };
      const mock = projectById(projectId);
      return mock ? { title: mock.title, color: mock.color } : undefined;
    },
    [liveProjectById],
  );

  const backendActivities: Activity[] = useMemo(() => {
    if (!apiActivities) return [];
    return apiActivities.map(a => ({
      id: a.id,
      userId: a.userId,
      projectId: a.projectId,
      startedAt: a.startedAt,
      endedAt: a.endedAt,
      description: a.description || "Activity block",
      productivity: a.productivity || 0,
      mouseClicks: a.mouseClicks || 0,
      keyboardHits: a.keyboardHits || 0,
      activeWindows: a.activeWindowsJson ? JSON.parse(a.activeWindowsJson) : [],
      runningPrograms: a.runningProgramsJson ? JSON.parse(a.runningProgramsJson) : [],
      screen: { app: "System", kind: "desktop" as any, accent: "#1e293b" }, // Fallback for UI
      hasWebcam: false,
      online: false,
    }));
  }, [apiActivities]);

  function setViewMode(mode: ViewMode) {
    setView(mode);
    router.replace(mode === "screens" ? "/activities?view=screens" : "/activities", { scroll: false });
  }

  const base = useMemo(() => scopeActivities(user, backendActivities), [user, backendActivities]);
  // Live activity is timestamped on the real clock, but the shared date presets pivot on the
  // frozen demo NOW (2026-07-14). Convert the selected preset to explicit real-clock bounds so
  // real rows are not silently filtered out.
  const effectiveFilters = useMemo(() => liveEffectiveFilters(filters), [filters]);
  const filtered = useMemo(() => applyActivityFilters(base, effectiveFilters), [base, effectiveFilters]);
  const updatedAt = backendActivities[0]?.endedAt;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Monitor"
        title="Activity"
        description={
          <>
            {filtered.length} tracked sessions
            {updatedAt ? ` · updated ${agoLabel(updatedAt)}` : ""}
            {" · "}
            <span className="text-muted-foreground/80">previews are mock placeholders</span>
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

      {filtered.length === 0 && (
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

      {view === "sessions" ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {filtered.map((a) => {
            const u = userById(a.userId);
            const p = resolveProject(a.projectId);
            return (
              <Card
                key={a.id}
                onClick={() => setSelected(a)}
                className="group cursor-pointer overflow-hidden p-0 transition-all hover:-translate-y-0.5 hover:card-elev-lg"
              >
                <div className="relative">
                  <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
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
              </Card>
            );
          })}
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4">
          {filtered.map((a) => {
            const u = userById(a.userId);
            const p = resolveProject(a.projectId);
            return (
              <button
                key={a.id}
                type="button"
                onClick={() => setSelected(a)}
                className="group overflow-hidden rounded-xl border border-border text-left transition-all hover:-translate-y-0.5 hover:border-primary/40"
              >
                <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
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

      <ActivityDrawer
        activity={selected}
        project={selected ? resolveProject(selected.projectId) ?? null : null}
        onClose={() => setSelected(null)}
      />
    </PageStack>
  );
}

function ActivityDrawer({
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
  const live = !!activityId && isGuidId(activityId);
  const [captures, setCaptures] = useState<CaptureView[]>([]);
  const [capturesLoading, setCapturesLoading] = useState(false);
  const [capturesError, setCapturesError] = useState(false);

  useEffect(() => {
    setCaptures([]);
    setCapturesError(false);
    setCapturesLoading(false);
    if (!activityId || !isGuidId(activityId)) return;
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;

    let cancelled = false;
    const urls: string[] = [];
    setCapturesLoading(true);

    (async () => {
      try {
        const metas = await getApi(`/api/app/activity/screenshots?activityId=${activityId}`);
        const list: Array<{ id: string; kind?: string }> = Array.isArray(metas) ? metas : [];
        const loaded: CaptureView[] = [];
        for (const meta of list) {
          const url = await getAuthedBlobUrl(`/api/app/activity/screenshot/${meta.id}/content`);
          if (cancelled) {
            URL.revokeObjectURL(url);
            return;
          }
          urls.push(url);
          loaded.push({ id: meta.id, kind: meta.kind === "webcam" ? "webcam" : "screen", url });
        }
        if (!cancelled) setCaptures(loaded);
      } catch {
        if (!cancelled) setCapturesError(true);
      } finally {
        if (!cancelled) setCapturesLoading(false);
      }
    })();

    // Revoke object URLs when the drawer closes, switches activity, or unmounts.
    return () => {
      cancelled = true;
      urls.forEach((url) => URL.revokeObjectURL(url));
    };
  }, [activityId]);

  const screenCaptures = captures.filter((c) => c.kind === "screen");
  const webcamCaptures = captures.filter((c) => c.kind === "webcam");

  return (
    <Drawer open={!!a} onClose={onClose} title="Activity detail">
      {a && (
        <div className="space-y-6">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Avatar name={u?.name ?? ""} size="lg" status={u?.status} />
              <div>
                <div className="font-semibold">{u?.name}</div>
                <div className="text-xs text-muted-foreground">{u?.designation}</div>
              </div>
            </div>
            <Badge tone="primary">{p?.title}</Badge>
          </div>

          <div>
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <Camera className="h-4 w-4 text-muted-foreground" /> Screen capture
              {!live && <Badge tone="muted">Mock preview</Badge>}
            </div>
            {live ? (
              capturesLoading ? (
                <div className="flex aspect-video w-full items-center justify-center rounded-xl border border-border bg-muted/40 text-sm text-muted-foreground">
                  Loading captures…
                </div>
              ) : capturesError ? (
                <div className="flex aspect-video w-full items-center justify-center rounded-xl border border-border bg-muted/40 text-sm text-muted-foreground">
                  Couldn&apos;t load captures.
                </div>
              ) : screenCaptures.length > 0 ? (
                <div className="space-y-2">
                  {screenCaptures.map((c) => (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      key={c.id}
                      src={c.url}
                      alt="Screen capture"
                      className="aspect-video w-full rounded-xl border border-border object-cover"
                    />
                  ))}
                </div>
              ) : (
                <div className="flex aspect-video w-full items-center justify-center rounded-xl border border-border bg-muted/40 text-sm text-muted-foreground">
                  No screenshot captured
                </div>
              )
            ) : (
              <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
            )}
          </div>

          <div className="grid grid-cols-2 gap-3">
            <Metric icon={Clock} label="Time range" value={timeRange(a)} />
            <Metric icon={AppWindow} label="Description" value={a.description} />
            <Metric icon={MousePointerClick} label="Mouse clicks" value={a.mouseClicks.toLocaleString()} />
            <Metric icon={Keyboard} label="Keyboard hits (count only)" value={a.keyboardHits.toLocaleString()} />
          </div>

          <div className="flex items-center gap-4 rounded-xl border border-border p-4">
            <Ring value={a.productivity} size={64} stroke={7} />
            <div>
              <div className="text-sm font-medium">Activity level</div>
              <div className="text-xs text-muted-foreground">
                Based on focused apps and input counts during this block — not a judgment score.
              </div>
            </div>
          </div>

          {live ? (
            webcamCaptures.length > 0 && (
              <div>
                <div className="mb-2 flex items-center gap-2 text-sm font-medium">
                  <Video className="h-4 w-4 text-muted-foreground" /> Webcam (opt-in)
                </div>
                <div className="flex flex-wrap gap-2">
                  {webcamCaptures.map((c) => (
                    // eslint-disable-next-line @next/next/no-img-element
                    <img
                      key={c.id}
                      src={c.url}
                      alt="Webcam frame"
                      className="aspect-video w-40 rounded-lg border border-border object-cover"
                    />
                  ))}
                </div>
              </div>
            )
          ) : (
            a.hasWebcam && (
              <div>
                <div className="mb-2 flex items-center gap-2 text-sm font-medium">
                  <Video className="h-4 w-4 text-muted-foreground" /> Webcam (opt-in)
                </div>
                <div
                  className="flex aspect-video w-40 items-center justify-center rounded-lg text-white/70"
                  style={{ background: `radial-gradient(circle at 50% 35%, ${a.screen.accent}, #0f1320)` }}
                >
                  <Video className="h-8 w-8" />
                </div>
              </div>
            )
          )}

          <div>
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <AppWindow className="h-4 w-4 text-muted-foreground" /> Active windows
            </div>
            <div className="space-y-1.5">
              {a.activeWindows.map((w, i) => (
                <div key={i} className="flex items-center justify-between rounded-lg bg-muted/50 px-3 py-2 text-sm">
                  <div className="min-w-0">
                    <div className="truncate font-medium">{w.appName}</div>
                    <div className="truncate text-xs text-muted-foreground">{w.windowTitle}</div>
                  </div>
                  <span className="text-xs text-muted-foreground">{Math.round(w.seconds / 60)}m</span>
                </div>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <ListTree className="h-4 w-4 text-muted-foreground" /> Running programs
            </div>
            <div className="flex flex-wrap gap-1.5">
              {a.runningPrograms.map((w, i) => (
                <Badge key={i} tone="muted">{w.appName}</Badge>
              ))}
            </div>
          </div>
        </div>
      )}
    </Drawer>
  );
}

function Metric({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Clock;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-lg border border-border p-3">
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5" /> {label}
      </div>
      <div className={cn("mt-1 truncate text-sm font-medium")}>{value}</div>
    </div>
  );
}
