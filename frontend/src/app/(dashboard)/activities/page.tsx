"use client";

import { Suspense, useEffect, useMemo, useState } from "react";
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
import { NOW, projectById, userById } from "@/lib/tenant-data";
import { useApi } from "@/hooks/useApi";
import { applyActivityFilters, defaultActivityFilters, type ActivityFilters } from "@/lib/activity-filters";
import { scopeActivities } from "@/lib/scope";
import type { Activity } from "@/lib/types";
import { cn, formatCompact } from "@/lib/utils";

type ViewMode = "sessions" | "screens";

function timeRange(a: Activity) {
  const fmt = (iso: string) =>
    new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit" });
  return `${fmt(a.startedAt)} – ${fmt(a.endedAt)}`;
}
function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
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

  const { data: apiActivities, isLoading } = useApi<any[]>('/api/app/activity');

  useEffect(() => {
    setView(searchParams.get("view") === "screens" ? "screens" : "sessions");
  }, [searchParams]);

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
  const filtered = useMemo(() => applyActivityFilters(base, filters), [base, filters]);
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
            const p = projectById(a.projectId);
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
            const p = projectById(a.projectId);
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

      <ActivityDrawer activity={selected} onClose={() => setSelected(null)} />
    </PageStack>
  );
}

function ActivityDrawer({ activity, onClose }: { activity: Activity | null; onClose: () => void }) {
  const a = activity;
  const u = a ? userById(a.userId) : null;
  const p = a ? projectById(a.projectId) : null;

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
              <Badge tone="muted">Mock preview</Badge>
            </div>
            <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
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

          {a.hasWebcam && (
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
