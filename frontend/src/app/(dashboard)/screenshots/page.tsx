"use client";

import { useMemo, useState } from "react";
import { Video, Maximize2, MousePointerClick, Keyboard, SearchX } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Modal } from "@/components/ui/modal";
import { ScreenMockView } from "@/components/screen-mock";
import { ActivityFilterBar } from "@/components/activities/activity-filter-bar";
import { useSession } from "@/components/session-provider";
import { activities, NOW, projectById, userById } from "@/lib/tenant-data";
import { applyActivityFilters, defaultActivityFilters, type ActivityFilters } from "@/lib/activity-filters";
import { scopeActivities } from "@/lib/scope";
import type { Activity } from "@/lib/types";

function timeLabel(iso: string) {
  return new Date(iso).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit" });
}
function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export default function ScreenshotsPage() {
  const { user } = useSession();
  const [filters, setFilters] = useState<ActivityFilters>(defaultActivityFilters);
  const [active, setActive] = useState<Activity | null>(null);

  const base = useMemo(() => scopeActivities(user, activities), [user]);
  const shots = useMemo(() => applyActivityFilters(base, filters), [base, filters]);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Screenshots</h1>
          <p className="mt-1 text-sm text-muted-foreground">{shots.length} captures · auto-collected at each interval</p>
        </div>
      </div>

      <ActivityFilterBar
        value={filters}
        onChange={setFilters}
        resultCount={shots.length}
        totalCount={base.length}
        showMemberFilter={user.role !== "worker"}
      />

      {shots.length === 0 && (
        <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <SearchX className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No screenshots match your filters</p>
            <p className="mt-1 text-sm text-muted-foreground">Try widening the date range or clearing some filters.</p>
          </div>
          <button onClick={() => setFilters(defaultActivityFilters)} className="text-sm text-primary hover:underline">
            Reset filters
          </button>
        </Card>
      )}

      <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4">
        {shots.map((a) => {
          const u = userById(a.userId);
          const p = projectById(a.projectId);
          return (
            <Card key={a.id} className="group cursor-pointer overflow-hidden p-0" onClick={() => setActive(a)}>
              <div className="relative">
                <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
                <div className="absolute inset-0 flex items-center justify-center bg-black/0 opacity-0 transition-all group-hover:bg-black/40 group-hover:opacity-100">
                  <Maximize2 className="h-6 w-6 text-white" />
                </div>
                <div className="absolute right-2 top-2 flex gap-1">
                  {a.hasWebcam && (
                    <span className="flex h-6 w-6 items-center justify-center rounded-md bg-black/60 text-white backdrop-blur">
                      <Video className="h-3.5 w-3.5" />
                    </span>
                  )}
                </div>
                <div className="absolute bottom-2 left-2">
                  <Badge className="bg-black/60 text-white backdrop-blur">{timeLabel(a.endedAt)}</Badge>
                </div>
              </div>
              <div className="flex items-center gap-2 p-3">
                <Avatar name={u?.name ?? ""} size="sm" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-xs font-medium">{u?.name}</div>
                  <div className="flex items-center gap-1 text-[11px] text-muted-foreground">
                    <span className="h-1.5 w-1.5 rounded-full" style={{ background: p?.color }} />
                    <span className="truncate">{p?.title}</span>
                  </div>
                </div>
                <span className="text-[11px] text-muted-foreground">{agoLabel(a.endedAt)}</span>
              </div>
            </Card>
          );
        })}
      </div>

      {/* Lightbox */}
      <Modal
        open={!!active}
        onClose={() => setActive(null)}
        title={active ? `${userById(active.userId)?.name} · ${timeLabel(active.endedAt)}` : ""}
        className="max-w-3xl"
      >
        {active && (
          <div className="space-y-4">
            <ScreenMockView screen={active.screen} title={active.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
            <div className="flex flex-wrap items-center gap-4 text-sm">
              <Badge tone="primary">{projectById(active.projectId)?.title}</Badge>
              <span className="flex items-center gap-1 text-muted-foreground">
                <MousePointerClick className="h-4 w-4" /> {active.mouseClicks.toLocaleString()} clicks
              </span>
              <span className="flex items-center gap-1 text-muted-foreground">
                <Keyboard className="h-4 w-4" /> {active.keyboardHits.toLocaleString()} keys
              </span>
              <span className="text-muted-foreground">{active.description}</span>
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}
