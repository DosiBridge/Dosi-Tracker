"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Clock, CalendarRange, Users, FolderKanban } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Select } from "@/components/ui/input";
import { StatCard } from "@/components/dashboard/stat-card";
import { arrayOf } from "@/components/dashboard/dashboard-data";
import { activitiesForProject, projects, userById } from "@/lib/tenant-data";
import { agoLabel, referenceNow } from "@/lib/live-session";
import { displayName } from "@/lib/live-dataset";
import { durationMinutes } from "@/lib/metrics";
import { brand } from "@/lib/brand";
import { formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { useSession } from "@/components/session-provider";
import { reportingRoute, type DailyReportRowDto, type ReportSummaryDto, type UserReportRowDto } from "@/lib/api-contract";
import { formatDayRange, zonedDayKey, zonedPeriod, type ZonedPeriod } from "@/lib/time-zone";
import { clientProjectStatus, clientProjectStatusOnDay, type ClientProjectStatus } from "./client-status";
import type { User } from "@/lib/types";

/** Hours on ONE project: the week per member, the last 30 days, and when work was last tracked. */
interface ProjectFigures {
  weekMinutes: number;
  monthMinutes: number;
  /** Members who tracked time on the project this week, most time first. */
  members: { id: string; name: string; minutes: number }[];
  /** When work was last tracked on the project (within the last 30 days), or null. */
  lastWorkAt: string | null;
  /**
   * LIVE: the reporting-zone day work was last tracked on (a daily series has
   * no time of day), or null. When set, it stands for `lastWorkAt`.
   */
  lastWorkDay?: string | null;
}

const toneOf: Record<ClientProjectStatus["tone"], "success" | "info" | "warning" | "muted"> = {
  success: "success",
  info: "info",
  warning: "warning",
  muted: "muted",
};

export function ClientDashboard({ user }: { user: User }) {
  const { isLive } = useSession();
  const zone = useReportingZone(isLive);
  const myProjects = useMemo(() => projects.filter((p) => (p.memberIds ?? []).includes(user.id)), [user.id]);
  const [projectId, setProjectId] = useState(myProjects[0]?.id ?? "");
  const project = myProjects.find((p) => p.id === projectId) ?? myProjects[0];
  const shownId = project?.id ?? "";
  const [now] = useState(() => referenceNow());

  // The periods, as calendar days of the reporting zone.
  const week: ZonedPeriod = useMemo(() => zonedPeriod("week", now, zone.timeZone), [now, zone.timeZone]);
  const month: ZonedPeriod = useMemo(() => zonedPeriod("30d", now, zone.timeZone), [now, zone.timeZone]);

  // LIVE: the reporting API scoped to this one project (a client sees the
  // projects they're a client of; contract §6).
  const [live, setLive] = useState<{ projectId: string; figures: ProjectFigures } | null>(null);
  const [liveError, setLiveError] = useState(false);
  const loadLive = useCallback(async () => {
    if (!shownId) return;
    const pid = shownId;
    const tz = zone.timeZone;
    setLiveError(false);
    try {
      const [weekSummary, monthSummary, series] = await Promise.all([
        getApi(reportingRoute("summary", { from: week.from, to: week.to, projectId: pid, timeZone: tz })),
        getApi(reportingRoute("summary", { from: month.from, to: month.to, projectId: pid, timeZone: tz })),
        getApi(reportingRoute("daily-series", { from: month.from, to: month.to, projectId: pid, timeZone: tz })),
      ]);
      const ws = (weekSummary ?? {}) as Partial<ReportSummaryDto>;
      const ms = (monthSummary ?? {}) as Partial<ReportSummaryDto>;
      const lastDay = arrayOf<DailyReportRowDto>(series)
        .filter((r) => r && typeof r.date === "string" && Number(r.trackedMinutes) > 0)
        .map((r) => r.date.slice(0, 10))
        .sort()
        .pop();
      setLive({
        projectId: pid,
        figures: {
          weekMinutes: Math.round(Number(ws.totalTrackedMinutes) || 0),
          monthMinutes: Math.round(Number(ms.totalTrackedMinutes) || 0),
          members: arrayOf<UserReportRowDto>(ws.perUser)
            .filter((r) => r && typeof r.userId === "string" && Number(r.trackedMinutes) > 0)
            .map((r) => ({
              id: r.userId,
              name: userById(r.userId)?.name ?? displayName({ name: r.name, userName: r.userName }, "Team member"),
              minutes: Math.round(Number(r.trackedMinutes)),
            }))
            .sort((a, b) => b.minutes - a.minutes),
          // A daily series knows the day, not the minute: report at day precision.
          lastWorkAt: null,
          lastWorkDay: lastDay ?? null,
        },
      });
      setLiveError(false);
    } catch {
      setLiveError(true);
    }
  }, [shownId, zone.timeZone, week, month]);

  useEffect(() => {
    if (!isLive || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadLive();
  }, [isLive, zone.ready, loadLive]);

  // DEMO: the same figures from the demo rows of this project only.
  const demo = useMemo<ProjectFigures | null>(() => {
    if (isLive || !shownId) return null;
    const rows = activitiesForProject(shownId);
    const inRange = (from: Date, to: Date) => rows.filter((a) => Date.parse(a.endedAt) > from.getTime() && Date.parse(a.startedAt) < to.getTime());
    const thisWeek = inRange(week.from, week.to);
    const perMember = new Map<string, number>();
    for (const a of thisWeek) perMember.set(a.userId, (perMember.get(a.userId) ?? 0) + durationMinutes(a));
    const lastEnd = rows.reduce((max, a) => Math.max(max, Math.min(Date.parse(a.endedAt), now.getTime())), 0);
    return {
      weekMinutes: Math.round(thisWeek.reduce((s, a) => s + durationMinutes(a), 0)),
      monthMinutes: Math.round(inRange(month.from, month.to).reduce((s, a) => s + durationMinutes(a), 0)),
      members: [...perMember.entries()]
        .map(([id, minutes]) => ({ id, name: userById(id)?.name ?? "Team member", minutes: Math.round(minutes) }))
        .sort((a, b) => b.minutes - a.minutes),
      lastWorkAt: lastEnd > month.from.getTime() ? new Date(lastEnd).toISOString() : null,
    };
  }, [isLive, shownId, week, month, now]);

  if (!project) {
    return (
      <Card className="flex flex-col items-center justify-center gap-3 py-20 text-center">
        <div aria-hidden className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <FolderKanban className="h-6 w-6" />
        </div>
        <div>
          <p className="font-medium">No project shared with you yet</p>
          <p className="mt-1 text-sm text-muted-foreground">Your workspace admin will grant you access to a project.</p>
        </div>
      </Card>
    );
  }

  const figures = isLive ? (live?.projectId === project.id ? live.figures : null) : demo;
  const status = !figures
    ? null
    : figures.lastWorkDay !== undefined
      ? clientProjectStatusOnDay(figures.lastWorkDay, zonedDayKey(now, zone.timeZone))
      : clientProjectStatus(figures.lastWorkAt, now);
  const sessions = isLive ? [] : activitiesForProject(project.id).slice(0, 8);
  const weekLabel = formatDayRange(week.fromKey, week.toKey);
  const dash = liveError ? "—" : "…";

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="page-title">Welcome, {user.name.split(" ")[0]}</h1>
            <Badge tone="info">Client view</Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            Hours worked on projects shared with you — not employee screen captures.
          </p>
        </div>
        {myProjects.length > 1 && (
          <div className="space-y-1">
            <label htmlFor="client-project" className="block text-xs font-medium text-muted-foreground">
              Project
            </label>
            <Select id="client-project" value={project.id} onChange={(e) => setProjectId(e.target.value)} className="w-auto min-w-48">
              {myProjects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.title}
                </option>
              ))}
            </Select>
          </div>
        )}
      </div>

      <Card>
        <CardContent className="flex flex-wrap items-center gap-4 p-5">
          <div
            aria-hidden
            className="flex h-12 w-12 items-center justify-center rounded-xl text-lg font-bold"
            style={{ background: `${project.color}22`, color: project.color }}
          >
            {project.title.charAt(0)}
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-lg font-semibold">{project.title}</div>
            <div className="truncate text-sm text-muted-foreground">{project.description || "No description yet."}</div>
          </div>
          {status && (
            <div className="text-right">
              <Badge tone={toneOf[status.tone]}>{status.label}</Badge>
              <p className="mt-1 text-xs text-muted-foreground">{status.detail}</p>
            </div>
          )}
        </CardContent>
      </Card>

      {liveError && (
        <p role="alert" className="text-xs text-danger-on-tint">
          Couldn&apos;t load this project&apos;s hours.{" "}
          <button type="button" onClick={() => void loadLive()} className="font-medium underline underline-offset-2">
            Retry
          </button>
        </p>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <StatCard label="This week" value={figures ? formatDuration(figures.weekMinutes) : dash} icon={Clock} accent={brand.primary} sub={weekLabel} />
        <StatCard
          label="Last 30 days"
          value={figures ? formatDuration(figures.monthMinutes) : dash}
          icon={CalendarRange}
          accent={brand.info}
          sub={formatDayRange(month.fromKey, month.toKey)}
        />
        <StatCard
          label="People this week"
          value={figures ? String(figures.members.length) : dash}
          icon={Users}
          accent={brand.pink}
          sub="tracked time on the project"
        />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div>
              <CardTitle>Hours on this project</CardTitle>
              <p className="mt-0.5 text-xs text-muted-foreground">This week · {weekLabel}</p>
            </div>
          </CardHeader>
          <CardContent>
            {!figures ? (
              <p className="py-6 text-center text-sm text-muted-foreground">{liveError ? "Not available right now." : "Loading…"}</p>
            ) : figures.members.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">No one has tracked time on this project this week.</p>
            ) : (
              <ul className="space-y-1">
                {figures.members.map((m) => (
                  <li key={m.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                    <Avatar name={m.name} size="sm" />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">{m.name}</span>
                    <span className="text-sm font-medium tabular-nums">{formatDuration(m.minutes)}</span>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Recent work updates</CardTitle>
          </CardHeader>
          <CardContent>
            {isLive ? (
              <p className="text-sm text-muted-foreground">
                {figures?.lastWorkDay
                  ? `${status?.detail ?? "Last work tracked recently"}.`
                  : "No work tracked on this project in the last 30 days."}
              </p>
            ) : sessions.length === 0 ? (
              <p className="text-sm text-muted-foreground">No updates logged for this project yet.</p>
            ) : (
              <ul className="space-y-3">
                {sessions.map((a) => {
                  const u = userById(a.userId);
                  return (
                    <li key={a.id} className="flex items-center gap-3 border-b border-border/60 pb-3 last:border-0 last:pb-0">
                      <Avatar name={u?.name ?? "Team member"} size="sm" />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm">
                          <span className="font-medium">{u?.name ?? "Team member"}</span>{" "}
                          <span className="text-muted-foreground">— {a.description}</span>
                        </div>
                        <div className="text-xs text-muted-foreground">{agoLabel(a.endedAt, now)}</div>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
