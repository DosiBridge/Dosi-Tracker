"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import {
  Plus,
  Search,
  Clock,
  Camera,
  Video,
  Keyboard,
  MousePointerClick,
  AppWindow,
  Archive,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { CreateProjectModal } from "@/components/projects/create-project-modal";
import { useSession } from "@/components/session-provider";
import { userById } from "@/lib/tenant-data";
import { scopeProjects } from "@/lib/scope";
import type { Project } from "@/lib/types";
import { cn, formatDuration } from "@/lib/utils";

const permIcons = [
  { key: "screenshot", icon: Camera },
  { key: "webcam", icon: Video },
  { key: "keyboard", icon: Keyboard },
  { key: "mouse", icon: MousePointerClick },
  { key: "activeWindow", icon: AppWindow },
] as const;

export default function ProjectsPage() {
  const { user } = useSession();
  const canManage = user.role === "owner" || user.role === "admin";
  const [created, setCreated] = useState<Project[]>([]);
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"active" | "archived">("active");
  const [open, setOpen] = useState(false);

  const list = useMemo(() => [...created, ...scopeProjects(user)], [created, user]);

  const filtered = useMemo(
    () =>
      list.filter(
        (p) =>
          p.archived === (tab === "archived") &&
          p.title.toLowerCase().includes(query.toLowerCase())
      ),
    [list, query, tab]
  );

  const maxTotal = Math.max(...list.map((p) => p.loggedTotal), 1);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Projects</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {list.filter((p) => !p.archived).length} active ·{" "}
            {list.filter((p) => p.archived).length} archived
          </p>
        </div>
        {canManage && (
          <Button onClick={() => setOpen(true)}>
            <Plus className="h-4 w-4" /> New project
          </Button>
        )}
      </div>

      {/* Controls */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search projects…"
            className="h-10 w-full rounded-lg border border-border bg-card pl-9 pr-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
        <div className="flex rounded-lg border border-border bg-card p-1">
          {(["active", "archived"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={cn(
                "rounded-md px-3 py-1.5 text-sm font-medium capitalize transition-colors",
                tab === t ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground"
              )}
            >
              {t}
            </button>
          ))}
        </div>
      </div>

      {/* Grid */}
      {filtered.length === 0 ? (
        <Card className="flex flex-col items-center justify-center gap-3 p-16 text-center">
          <Archive className="h-10 w-10 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">No {tab} projects found.</p>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {filtered.map((p) => (
            <Link key={p.id} href={`/projects/${p.id}`}>
              <Card className="group h-full p-5 transition-all hover:-translate-y-0.5 hover:card-elev-lg">
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div
                      className="flex h-11 w-11 items-center justify-center rounded-xl text-white"
                      style={{ background: p.color }}
                    >
                      <span className="text-base font-bold">{p.title[0]}</span>
                    </div>
                    <div>
                      <h3 className="font-semibold leading-tight group-hover:text-primary">{p.title}</h3>
                      <p className="text-xs text-muted-foreground">Interval · {p.intervalMinutes}m</p>
                    </div>
                  </div>
                  {p.archived && <Badge tone="muted">Archived</Badge>}
                </div>

                <p className="mt-3 line-clamp-2 text-sm text-muted-foreground">{p.description}</p>

                {/* Tracking chips */}
                <div className="mt-4 flex flex-wrap gap-1.5">
                  {permIcons.map(({ key, icon: Icon }) => {
                    const on = p.permissions[key];
                    return (
                      <span
                        key={key}
                        className={cn(
                          "flex h-7 w-7 items-center justify-center rounded-lg",
                          on ? "bg-accent text-accent-foreground" : "bg-muted text-muted-foreground/40"
                        )}
                        title={key}
                      >
                        <Icon className="h-3.5 w-3.5" />
                      </span>
                    );
                  })}
                </div>

                {/* Logged progress */}
                <div className="mt-4 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="flex items-center gap-1 text-muted-foreground">
                      <Clock className="h-3 w-3" /> {formatDuration(p.loggedThisWeek)} this week
                    </span>
                    <span className="font-medium">{formatDuration(p.loggedTotal)}</span>
                  </div>
                  <Progress value={(p.loggedTotal / maxTotal) * 100} color={p.color} />
                </div>

                {/* Members */}
                <div className="mt-4 flex items-center justify-between">
                  <div className="flex -space-x-2">
                    {p.memberIds.slice(0, 4).map((id) => {
                      const u = userById(id);
                      return u ? <Avatar key={id} name={u.name} size="sm" /> : null;
                    })}
                    {p.memberIds.length > 4 && (
                      <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium ring-2 ring-card">
                        +{p.memberIds.length - 4}
                      </span>
                    )}
                  </div>
                  <span className="text-xs text-muted-foreground">{p.memberIds.length} members</span>
                </div>
              </Card>
            </Link>
          ))}
        </div>
      )}

      <CreateProjectModal open={open} onClose={() => setOpen(false)} onCreate={(p) => setCreated((l) => [p, ...l])} />
    </div>
  );
}
