"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import {
  Plus,
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
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { CreateProjectModal } from "@/components/projects/create-project-modal";
import { EmptyState } from "@/components/ui/empty-state";
import { useSession } from "@/components/session-provider";
import { userById } from "@/lib/tenant-data";
import { useApi, postApi } from "@/hooks/useApi";
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
  const { user, workspace } = useSession();
  const canManage = user.role === "owner" || user.role === "admin";
  const { data: apiProjects, isLoading, refetch } = useApi<Project[]>('/api/app/project');
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"active" | "archived">("active");
  const [open, setOpen] = useState(false);
  const [isCreating, setIsCreating] = useState(false);

  const list = useMemo(() => apiProjects || [], [apiProjects]);

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
  const activeCount = list.filter((p) => !p.archived).length;
  const archivedCount = list.filter((p) => p.archived).length;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Workspace"
        title="Projects"
        description={`${activeCount} active · ${archivedCount} archived`}
        actions={
          canManage ? (
            <Button onClick={() => setOpen(true)}>
              <Plus className="h-4 w-4" /> New project
            </Button>
          ) : undefined
        }
      />

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search projects…" />
        <SegmentedControl
          value={tab}
          onChange={setTab}
          options={[
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
          ]}
        />
      </Toolbar>

      {filtered.length === 0 ? (
        <EmptyState
          icon={Archive}
          title={query ? "No matching projects" : `No ${tab} projects`}
          description={
            query
              ? "Try a different search, or clear the filter."
              : tab === "archived"
                ? "Archived projects will show up here."
                : canManage
                  ? "Create a project to start attributing time and captures."
                  : "You haven’t been added to a project yet."
          }
          action={
            query
              ? { label: "Clear search", onClick: () => setQuery("") }
              : canManage && tab === "active"
                ? { label: "New project", onClick: () => setOpen(true) }
                : undefined
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {filtered.map((p) => (
            <Link key={p.id} href={`/projects/${p.id}`}>
              <Card variant="interactive" className="group h-full p-5">
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div
                      className="flex h-11 w-11 items-center justify-center rounded-xl text-white"
                      style={{ background: p.color }}
                    >
                      <span className="font-display text-base font-bold">{p.title[0]}</span>
                    </div>
                    <div>
                      <h3 className="font-semibold leading-tight group-hover:text-primary">{p.title}</h3>
                      <p className="text-xs text-muted-foreground">Interval · {p.intervalMinutes}m</p>
                    </div>
                  </div>
                  {p.archived && <Badge tone="muted">Archived</Badge>}
                </div>

                <p className="mt-3 line-clamp-2 text-sm text-muted-foreground">{p.description}</p>

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

                <div className="mt-4 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="flex items-center gap-1 text-muted-foreground">
                      <Clock className="h-3 w-3" /> {formatDuration(p.loggedThisWeek)} this week
                    </span>
                    <span className="font-medium tabular-nums">{formatDuration(p.loggedTotal)}</span>
                  </div>
                  <Progress value={(p.loggedTotal / maxTotal) * 100} color={p.color} />
                </div>

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

      <CreateProjectModal
        open={open}
        onClose={() => setOpen(false)}
        onCreate={async (p) => {
          setIsCreating(true);
          try {
            await postApi('/api/app/project', {
              title: p.title,
              description: p.description,
              color: p.color,
              intervalMinutes: p.intervalMinutes,
              permissions: p.permissions
            });
            await refetch();
          } catch (e) {
            console.error(e);
          } finally {
            setIsCreating(false);
          }
        }}
      />
    </PageStack>
  );
}
