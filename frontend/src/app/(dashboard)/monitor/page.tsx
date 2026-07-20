"use client";

import { useEffect, useMemo, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Clock,
  Gauge,
  Coffee,
  Video,
  Pause,
  MousePointerClick,
  Keyboard,
  LogIn,
  LogOut,
  Zap,
  Globe,
  Layers,
  ArrowLeft,
  ArrowRight,
  Users,
  Search,
  Camera,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Progress } from "@/components/ui/progress";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/modal";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import {
  DayNav,
  MiniDayStrip,
  MonitorKpi,
  RANGE,
  TypeLegend,
  prodColor,
  typeColor,
} from "@/components/monitor/shared";
import { HourlyChart } from "@/components/dashboard/charts";
import { ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import { projectById, users } from "@/lib/tenant-data";
import {
  DAY_START_MIN,
  appBreakdown,
  availableDays,
  browserSummary,
  buildDayTimeline,
  daySummary,
  fmtMin,
  hourlyBuckets,
  type BrowserOverview,
  type DaySegment,
  type SegmentType,
} from "@/lib/monitor-data";
import { categoryColor, type AppCategory } from "@/lib/reports-data";
import { roleLabels } from "@/lib/roles";
import { cn, formatDuration } from "@/lib/utils";
import type { UserStatus } from "@/lib/types";

const catLabel: Record<AppCategory, string> = {
  productive: "Productive",
  neutral: "Neutral",
  unproductive: "Unproductive",
};

const statusCopy: Record<UserStatus, string> = {
  active: "Active",
  idle: "Idle",
  offline: "Offline",
};

const statusTone: Record<UserStatus, "success" | "warning" | "muted"> = {
  active: "success",
  idle: "warning",
  offline: "muted",
};

type MonitorView = "timeline" | "schedule";
type Panel = "roster" | "detail";
type StatusFilter = "all" | UserStatus;

const typeIcon: Record<SegmentType, typeof Coffee> = {
  work: Zap,
  meeting: Video,
  break: Coffee,
  idle: Pause,
};

function sortRoster<T extends { user: { status: UserStatus }; summary: { tracked: number } }>(list: T[]) {
  return [...list].sort((a, b) => {
    const rank = (s: UserStatus) => (s === "active" ? 0 : s === "idle" ? 1 : 2);
    const byStatus = rank(a.user.status) - rank(b.user.status);
    if (byStatus !== 0) return byStatus;
    return b.summary.tracked - a.summary.tracked;
  });
}

export default function MonitorPage() {
  const { user } = useSession();
  const days = useMemo(() => availableDays(7), []);
  const monitorable = useMemo(() => users.filter((u) => u.role !== "client"), []);
  const isSelfOnly = user.role === "worker";

  const [panel, setPanel] = useState<Panel>(isSelfOnly ? "detail" : "roster");
  const [userId, setUserId] = useState(() =>
    isSelfOnly ? user.id : (monitorable.find((u) => u.role === "worker")?.id ?? user.id)
  );
  const [dayIdx, setDayIdx] = useState(0);
  const [view, setView] = useState<MonitorView>("timeline");
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");

  const activeUserId = isSelfOnly ? user.id : userId;
  const iso = days[dayIdx].iso;
  const day = days[dayIdx];
  const member = users.find((u) => u.id === activeUserId)!;

  useEffect(() => {
    window.scrollTo({ top: 0, behavior: "smooth" });
  }, [panel, activeUserId]);

  const segments = useMemo(() => buildDayTimeline(activeUserId, iso), [activeUserId, iso]);
  const summary = useMemo(() => daySummary(segments), [segments]);
  const hourly = useMemo(() => hourlyBuckets(segments), [segments]);
  const apps = useMemo(() => appBreakdown(segments), [segments]);
  const browser = useMemo(() => browserSummary(segments), [segments]);
  const shots = segments.filter((s) => s.type === "work" || s.type === "meeting");

  const rosterRows = useMemo(() => {
    return monitorable.map((u) => {
      const segs = buildDayTimeline(u.id, iso);
      return { user: u, segments: segs, summary: daySummary(segs) };
    });
  }, [monitorable, iso]);

  const navList = useMemo(() => sortRoster(rosterRows), [rosterRows]);

  const filteredRoster = useMemo(() => {
    const q = query.trim().toLowerCase();
    return sortRoster(
      rosterRows.filter((r) => {
        if (statusFilter !== "all" && r.user.status !== statusFilter) return false;
        if (!q) return true;
        return (
          r.user.name.toLowerCase().includes(q) ||
          r.user.designation.toLowerCase().includes(q) ||
          r.user.email.toLowerCase().includes(q)
        );
      })
    );
  }, [rosterRows, query, statusFilter]);

  const teamTotals = useMemo(() => {
    const tracked = rosterRows.reduce((s, r) => s + r.summary.tracked, 0);
    const activeNow = rosterRows.filter((r) => r.user.status === "active").length;
    const withTime = rosterRows.filter((r) => r.summary.tracked > 0);
    const avgProd = withTime.length
      ? Math.round(withTime.reduce((s, r) => s + r.summary.productivityAvg, 0) / withTime.length)
      : 0;
    return { tracked, activeNow, avgProd, withTime: withTime.length, count: rosterRows.length };
  }, [rosterRows]);

  const hourTicks = Array.from({ length: RANGE / 60 / 2 + 1 }, (_, i) => DAY_START_MIN + i * 120);
  const navIndex = navList.findIndex((r) => r.user.id === activeUserId);

  function openMember(id: string) {
    setUserId(id);
    setPanel("detail");
  }

  function goAdjacent(delta: -1 | 1) {
    const next = navList[navIndex + delta];
    if (next) openMember(next.user.id);
  }

  const dayLabel = day.isToday ? "Today" : `${day.weekday}, ${day.label}`;
  const dayNav = (
    <DayNav
      label={dayLabel}
      onPrev={() => setDayIdx((i) => Math.min(i + 1, days.length - 1))}
      onNext={() => setDayIdx((i) => Math.max(i - 1, 0))}
      canPrev={dayIdx < days.length - 1}
      canNext={dayIdx > 0}
      showToday={dayIdx !== 0}
      onToday={() => setDayIdx(0)}
    />
  );

  const focusCard = (
    <Card>
      <CardHeader>
        <CardTitle>Focus by hour</CardTitle>
        <Badge tone="muted">Minutes</Badge>
      </CardHeader>
      <CardContent>
        <HourlyChart data={hourly} />
      </CardContent>
    </Card>
  );

  const appsCard = (
    <Card>
      <CardHeader>
        <CardTitle>Apps used</CardTitle>
        {apps.length > 0 && <Badge tone="muted">{apps.length}</Badge>}
      </CardHeader>
      <CardContent className="space-y-2.5">
        {apps.length === 0 ? (
          <p className="text-sm text-muted-foreground">No apps recorded for this day.</p>
        ) : (
          apps.map((a) => (
            <div key={a.app}>
              <div className="mb-1 flex items-center justify-between text-sm">
                <span className="flex items-center gap-2 truncate">
                  <span className="h-2.5 w-2.5 rounded-full" style={{ background: a.accent }} />
                  <span className="truncate">{a.app}</span>
                </span>
                <span className="tabular-nums text-muted-foreground">{formatDuration(a.minutes)}</span>
              </div>
              <Progress value={(a.minutes / (apps[0]?.minutes || 1)) * 100} color={a.accent} />
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );

  /* ── Team roster (managers) ── */
  if (!isSelfOnly && panel === "roster") {
    return (
      <PageStack>
        <PageHeader
          eyebrow="Monitor"
          title="Member Monitor"
          description={`${teamTotals.activeNow} active now · ${formatDuration(teamTotals.tracked)} tracked · ${dayLabel.toLowerCase()}`}
          actions={dayNav}
        />

        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          <MonitorKpi icon={Users} tone={brand.primary} label="Team size" value={String(teamTotals.count)} />
          <MonitorKpi icon={Zap} tone={brand.success} label="Active now" value={String(teamTotals.activeNow)} />
          <MonitorKpi icon={Clock} tone={brand.info} label="Team tracked" value={formatDuration(teamTotals.tracked)} />
          <MonitorKpi
            icon={Gauge}
            tone={prodColor(teamTotals.avgProd)}
            label="Avg productivity"
            value={teamTotals.withTime ? `${teamTotals.avgProd}%` : "—"}
          />
        </div>

        <Toolbar>
          <SearchField value={query} onChange={setQuery} placeholder="Search by name, role, or email…" />
          <SegmentedControl
            value={statusFilter}
            onChange={setStatusFilter}
            options={[
              { value: "all", label: "All" },
              { value: "active", label: "Active" },
              { value: "idle", label: "Idle" },
              { value: "offline", label: "Offline" },
            ]}
          />
        </Toolbar>

        <div className="flex flex-wrap items-center justify-between gap-2 px-0.5">
          <TypeLegend />
          <p className="text-xs text-muted-foreground">
            {filteredRoster.length} of {rosterRows.length} members · click a row for the full day
          </p>
        </div>

        {filteredRoster.length === 0 ? (
          <EmptyState
            icon={Search}
            title={query || statusFilter !== "all" ? "No members match" : "No members to monitor"}
            description={
              query || statusFilter !== "all"
                ? "Try clearing search or switching the status filter."
                : "Invite workers to start seeing daily activity here."
            }
            action={
              query || statusFilter !== "all"
                ? {
                    label: "Clear filters",
                    onClick: () => {
                      setQuery("");
                      setStatusFilter("all");
                    },
                  }
                : undefined
            }
          />
        ) : (
          <Card className="overflow-hidden p-0">
            <div className="hidden grid-cols-[minmax(0,1.5fr)_6.5rem_5rem_5.5rem_minmax(8rem,1.2fr)_5.5rem] gap-3 border-b border-border bg-muted/35 px-4 py-2.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground lg:grid">
              <span>Member</span>
              <span className="text-right">Tracked</span>
              <span className="text-right">Prod.</span>
              <span className="text-right">Focus</span>
              <span>Day activity</span>
              <span className="text-right">Open</span>
            </div>

            <ul className="divide-y divide-border">
              {filteredRoster.map(({ user: u, segments: segs, summary: sum }) => (
                <li key={u.id}>
                  <button
                    type="button"
                    onClick={() => openMember(u.id)}
                    className="group grid w-full grid-cols-1 gap-3 px-4 py-3.5 text-left transition-colors hover:bg-primary/[0.04] focus-visible:bg-primary/[0.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring lg:grid-cols-[minmax(0,1.5fr)_6.5rem_5rem_5.5rem_minmax(8rem,1.2fr)_5.5rem] lg:items-center"
                  >
                    <div className="flex min-w-0 items-center gap-3">
                      <Avatar name={u.name} size="sm" status={u.status} />
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="truncate font-medium group-hover:text-primary">{u.name}</span>
                          <Badge tone={statusTone[u.status]} className="sm:hidden">
                            {statusCopy[u.status]}
                          </Badge>
                          <Badge tone="muted" className="hidden sm:inline-flex">
                            {roleLabels[u.role]}
                          </Badge>
                        </div>
                        <div className="truncate text-xs text-muted-foreground">
                          {u.designation}
                          <span className="hidden sm:inline"> · {statusCopy[u.status]}</span>
                        </div>
                      </div>
                    </div>

                    <div className="grid grid-cols-3 gap-2 text-sm lg:contents">
                      <MetricCell label="Tracked" value={sum.tracked ? formatDuration(sum.tracked) : "—"} strong />
                      <MetricCell
                        label="Prod."
                        value={sum.tracked ? `${sum.productivityAvg}%` : "—"}
                        strong
                        color={sum.tracked ? prodColor(sum.productivityAvg) : undefined}
                      />
                      <MetricCell
                        label="Focus"
                        value={sum.longestFocus ? formatDuration(sum.longestFocus) : "—"}
                        muted
                      />
                    </div>

                    <div className="min-w-0">
                      <MiniDayStrip segments={segs} />
                      <div className="mt-1 text-[10px] text-muted-foreground">
                        {sum.firstMin !== null
                          ? `${fmtMin(sum.firstMin)} – ${fmtMin(sum.lastMin!)}`
                          : "No activity this day"}
                      </div>
                    </div>

                    <span className="inline-flex items-center gap-1 text-xs font-medium text-primary lg:justify-end">
                      Details
                      <ArrowRight className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5" />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </Card>
        )}
      </PageStack>
    );
  }

  /* ── Member detail ── */
  return (
    <PageStack>
      {!isSelfOnly && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <button
            type="button"
            onClick={() => setPanel("roster")}
            className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
          >
            <ArrowLeft className="h-4 w-4" /> All members
          </button>
          <div className="flex items-center gap-1">
            <Button
              variant="outline"
              size="icon"
              disabled={navIndex <= 0}
              onClick={() => goAdjacent(-1)}
              aria-label="Previous member"
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <span className="min-w-16 px-1 text-center text-xs tabular-nums text-muted-foreground">
              {navIndex >= 0 ? `${navIndex + 1} / ${navList.length}` : "—"}
            </span>
            <Button
              variant="outline"
              size="icon"
              disabled={navIndex < 0 || navIndex >= navList.length - 1}
              onClick={() => goAdjacent(1)}
              aria-label="Next member"
            >
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}

      <PageHeader
        eyebrow="Monitor"
        title={isSelfOnly ? "My day" : member.name}
        description={
          isSelfOnly
            ? `Your activity for ${dayLabel.toLowerCase()}.`
            : `${member.designation} · ${statusCopy[member.status]} · ${dayLabel}`
        }
        actions={
          <div className="flex items-center gap-2.5">
            <Avatar name={member.name} size="sm" status={member.status} />
            <Badge tone={statusTone[member.status]}>{statusCopy[member.status]}</Badge>
          </div>
        }
      />

      <Toolbar>
        <SegmentedControl
          value={view}
          onChange={setView}
          options={[
            { value: "timeline", label: "Timeline" },
            { value: "schedule", label: "Schedule" },
          ]}
        />
        <div className="w-full sm:ml-auto sm:w-auto">{dayNav}</div>
      </Toolbar>

      {segments.length === 0 ? (
        <EmptyState
          icon={Pause}
          title="No activity tracked"
          description={`${member.name.split(" ")[0]} did not record any time on ${dayLabel.toLowerCase()}.`}
          action={
            dayIdx !== 0
              ? { label: "Jump to today", onClick: () => setDayIdx(0) }
              : !isSelfOnly
                ? { label: "Back to team", onClick: () => setPanel("roster") }
                : undefined
          }
        />
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-3 xl:grid-cols-6">
            <MonitorKpi icon={Clock} tone={brand.primary} label="Tracked" value={formatDuration(summary.tracked)} />
            <MonitorKpi
              icon={Gauge}
              tone={prodColor(summary.productivityAvg)}
              label="Productivity"
              value={`${summary.productivityAvg}%`}
            />
            <MonitorKpi icon={Zap} tone={brand.success} label="Longest focus" value={formatDuration(summary.longestFocus)} />
            <MonitorKpi
              icon={Pause}
              tone="#94a3b8"
              label="Idle + breaks"
              value={formatDuration(summary.idle + summary.breakMinutes)}
            />
            <MonitorKpi
              icon={LogIn}
              tone={brand.info}
              label="First seen"
              value={summary.firstMin !== null ? fmtMin(summary.firstMin) : "—"}
            />
            <MonitorKpi
              icon={LogOut}
              tone={brand.pink}
              label="Last seen"
              value={summary.lastMin !== null ? fmtMin(summary.lastMin) : "—"}
            />
          </div>

          <Card>
            <CardHeader>
              <CardTitle>Activity overview</CardTitle>
              <TypeLegend />
            </CardHeader>
            <CardContent>
              <div className="-mx-1 overflow-x-auto overscroll-x-contain px-1">
                <div className="relative h-16 min-w-[320px] w-full rounded-xl bg-muted/50">
                  {hourTicks.map((t) => (
                    <div
                      key={t}
                      className="absolute top-0 bottom-5 border-l border-border/70"
                      style={{ left: `${((t - DAY_START_MIN) / RANGE) * 100}%` }}
                    />
                  ))}
                  {segments.map((s) => {
                    const left = ((s.startMin - DAY_START_MIN) / RANGE) * 100;
                    const width = (s.minutes / RANGE) * 100;
                    const color = s.type === "work" ? prodColor(s.productivity) : typeColor[s.type];
                    return (
                      <div
                        key={s.id}
                        title={`${fmtMin(s.startMin)}–${fmtMin(s.endMin)} · ${
                          s.type === "idle" || s.type === "break" ? s.windowTitle : s.app
                        }`}
                        className={cn(
                          "absolute top-1.5 bottom-6 rounded-sm transition-all hover:brightness-110",
                          (s.type === "idle" || s.type === "break") && "opacity-50"
                        )}
                        style={{ left: `${left}%`, width: `calc(${width}% - 1px)`, background: color }}
                      />
                    );
                  })}
                  {hourTicks.map((t) => (
                    <span
                      key={`l-${t}`}
                      className="absolute bottom-0 -translate-x-1/2 text-[9px] text-muted-foreground sm:text-[10px]"
                      style={{ left: `${((t - DAY_START_MIN) / RANGE) * 100}%` }}
                    >
                      {fmtMin(t)}
                    </span>
                  ))}
                </div>
              </div>
            </CardContent>
          </Card>

          {view === "timeline" ? (
            <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
              <Card className="lg:col-span-2">
                <CardHeader>
                  <CardTitle>Detailed timeline</CardTitle>
                  <Badge tone="muted">{segments.length} entries</Badge>
                </CardHeader>
                <CardContent>
                  <div className="relative space-y-0 before:absolute before:left-[52px] before:top-2 before:bottom-2 before:w-px before:bg-border">
                    {segments.map((s) => (
                      <TimelineRow key={s.id} seg={s} />
                    ))}
                  </div>
                </CardContent>
              </Card>
              <div className="space-y-6">
                {focusCard}
                {appsCard}
              </div>
            </div>
          ) : (
            <>
              <DayScheduleView segments={segments} />
              <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
                {focusCard}
                {appsCard}
              </div>
            </>
          )}

          {browser.tabInstances > 0 && <BrowserActivity data={browser} />}

          {shots.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Camera className="h-4 w-4 text-muted-foreground" />
                  Screenshots
                </CardTitle>
                <Badge tone="muted">{shots.length} captures</Badge>
              </CardHeader>
              <CardContent>
                <div className="flex gap-3 overflow-x-auto pb-2">
                  {shots.map((s) => (
                    <div key={s.id} className="w-44 shrink-0">
                      <ScreenMockView screen={s.screen} title={s.windowTitle} className="aspect-video w-full" />
                      <div className="mt-1.5 flex items-center justify-between text-xs">
                        <span className="font-medium tabular-nums">{fmtMin(s.startMin)}</span>
                        <span className="truncate pl-2 text-muted-foreground">{s.app}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          )}
        </>
      )}
    </PageStack>
  );
}

function MetricCell({
  label,
  value,
  strong,
  muted,
  color,
}: {
  label: string;
  value: string;
  strong?: boolean;
  muted?: boolean;
  color?: string;
}) {
  return (
    <div className="flex items-center justify-between lg:block lg:text-right">
      <span className="text-xs text-muted-foreground lg:hidden">{label}</span>
      <span
        className={cn(
          "tabular-nums",
          strong && "font-display font-semibold",
          muted && "text-muted-foreground"
        )}
        style={color ? { color } : undefined}
      >
        {value}
      </span>
    </div>
  );
}

function DayScheduleView({ segments }: { segments: DaySegment[] }) {
  const PX_PER_MIN = 1.1;
  const height = RANGE * PX_PER_MIN;
  const hours = Array.from({ length: RANGE / 60 + 1 }, (_, i) => DAY_START_MIN + i * 60);
  const [selected, setSelected] = useState<DaySegment | null>(null);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Day schedule</CardTitle>
        <TypeLegend />
      </CardHeader>
      <CardContent>
        <p className="mb-3 text-xs text-muted-foreground">Tap any block for details and open tabs.</p>
        <div className="-mx-1 overflow-x-auto overscroll-x-contain px-1">
          <div className="flex min-w-[280px]">
            <div className="relative w-12 shrink-0 sm:w-14" style={{ height }}>
              {hours.map((h) => (
                <span
                  key={h}
                  className="absolute right-1 -translate-y-1/2 text-[10px] font-medium text-muted-foreground sm:right-2 sm:text-[11px]"
                  style={{ top: (h - DAY_START_MIN) * PX_PER_MIN }}
                >
                  {fmtMin(h)}
                </span>
              ))}
            </div>

            <div
              className="relative min-w-0 flex-1 overflow-hidden rounded-xl border border-border bg-muted/20"
              style={{ height }}
            >
              {hours.map((h) => (
                <div
                  key={h}
                  className="absolute left-0 right-0 border-t border-border/60"
                  style={{ top: (h - DAY_START_MIN) * PX_PER_MIN }}
                />
              ))}
              {hours.slice(0, -1).map((h) => (
                <div
                  key={`half-${h}`}
                  className="absolute left-0 right-0 border-t border-dashed border-border/30"
                  style={{ top: (h + 30 - DAY_START_MIN) * PX_PER_MIN }}
                />
              ))}
              {segments.map((s) => (
                <ScheduleBlock key={s.id} seg={s} pxPerMin={PX_PER_MIN} onSelect={setSelected} />
              ))}
            </div>
          </div>
        </div>
      </CardContent>

      <SegmentDetailModal seg={selected} onClose={() => setSelected(null)} />
    </Card>
  );
}

function ScheduleBlock({
  seg,
  pxPerMin,
  onSelect,
}: {
  seg: DaySegment;
  pxPerMin: number;
  onSelect: (seg: DaySegment) => void;
}) {
  const top = (seg.startMin - DAY_START_MIN) * pxPerMin;
  const height = Math.max(18, seg.minutes * pxPerMin);
  const isPassive = seg.type === "idle" || seg.type === "break";
  const color = seg.type === "work" ? prodColor(seg.productivity) : typeColor[seg.type];
  const compact = height < 34;
  const label = isPassive ? seg.type.charAt(0).toUpperCase() + seg.type.slice(1) : seg.app;

  return (
    <button
      type="button"
      onClick={() => onSelect(seg)}
      title={`${fmtMin(seg.startMin)}–${fmtMin(seg.endMin)} · ${label}${isPassive ? "" : ` · ${seg.productivity}%`}${seg.tabs ? ` · ${seg.tabs.length} tabs` : ""} — click for details`}
      className={cn(
        "absolute left-1.5 right-1.5 flex flex-col overflow-hidden rounded-md border-l-[3px] px-2 py-1 text-left transition-all hover:z-10 hover:brightness-[1.03] hover:ring-1 hover:ring-primary/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary",
        isPassive && "opacity-80"
      )}
      style={{
        top,
        height,
        borderLeftColor: color,
        background: isPassive ? "var(--color-muted)" : `${color}1f`,
      }}
    >
      <div className="flex items-center gap-2">
        <span className={cn("truncate font-semibold", compact ? "text-[11px]" : "text-xs")}>{label}</span>
        {seg.tabs && seg.tabs.length > 0 && !compact && (
          <span className="ml-auto flex shrink-0 items-center gap-0.5 text-[10px] font-medium text-muted-foreground">
            <Globe className="h-2.5 w-2.5" />
            {seg.tabs.length}
          </span>
        )}
        {!isPassive && !seg.tabs && !compact && (
          <span className="ml-auto shrink-0 text-[10px] font-semibold" style={{ color }}>
            {seg.productivity}%
          </span>
        )}
      </div>
      {!compact && (
        <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
          {fmtMin(seg.startMin)}–{fmtMin(seg.endMin)} · {formatDuration(seg.minutes)}
        </div>
      )}
    </button>
  );
}

function SegmentDetailModal({ seg, onClose }: { seg: DaySegment | null; onClose: () => void }) {
  if (!seg) return null;
  const isPassive = seg.type === "idle" || seg.type === "break";
  const p = seg.projectId ? projectById(seg.projectId) : null;
  const color = seg.type === "work" ? prodColor(seg.productivity) : typeColor[seg.type];
  const label = isPassive ? seg.type.charAt(0).toUpperCase() + seg.type.slice(1) : seg.app;

  return (
    <Modal
      open={!!seg}
      onClose={onClose}
      title={label}
      description={`${fmtMin(seg.startMin)}–${fmtMin(seg.endMin)} · ${formatDuration(seg.minutes)}`}
    >
      <div className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Badge tone="muted" className="capitalize">
            {seg.type}
          </Badge>
          {!isPassive && (
            <Badge tone="muted" style={{ color }}>
              {seg.productivity}% productive
            </Badge>
          )}
          {seg.tabs && seg.tabs.length > 0 && (
            <Badge tone="muted" className="gap-1">
              <Globe className="h-3 w-3" />
              {seg.tabs.length} tabs
            </Badge>
          )}
        </div>

        {!isPassive && (
          <div className="flex items-start gap-3">
            <ScreenMockView screen={seg.screen} className="hidden h-20 w-32 shrink-0 rounded-lg sm:block" />
            <div className="min-w-0 flex-1 space-y-1.5 text-sm">
              <div className="truncate text-muted-foreground">{seg.windowTitle}</div>
              {p && (
                <div className="flex items-center gap-1.5">
                  <span className="h-2.5 w-2.5 rounded-full" style={{ background: p.color }} /> {p.title}
                </div>
              )}
              <div className="flex flex-wrap gap-x-4 gap-y-1 text-muted-foreground">
                <span className="flex items-center gap-1">
                  <MousePointerClick className="h-3.5 w-3.5" />
                  {seg.mouseClicks.toLocaleString()} clicks
                </span>
                <span className="flex items-center gap-1">
                  <Keyboard className="h-3.5 w-3.5" />
                  {seg.keyboardHits.toLocaleString()} keys
                </span>
              </div>
            </div>
          </div>
        )}

        {seg.tabs && seg.tabs.length > 0 && (
          <div>
            <div className="mb-2 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              <Globe className="h-3.5 w-3.5" /> Open tabs ({seg.tabs.length})
            </div>
            <div className="space-y-1.5">
              {seg.tabs.map((t) => (
                <div key={t.id} className="flex items-center gap-2.5 rounded-xl border border-border p-2">
                  <span
                    className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-[10px] font-bold text-white"
                    style={{ background: t.color }}
                  >
                    {t.domain.charAt(0).toUpperCase()}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">{t.domain}</div>
                    <div className="truncate text-xs text-muted-foreground">{t.title}</div>
                  </div>
                  <span
                    className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium"
                    style={{ background: `${categoryColor[t.category]}1a`, color: categoryColor[t.category] }}
                  >
                    {catLabel[t.category]}
                  </span>
                  <span className="w-12 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                    {formatDuration(t.minutes)}
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}

function BrowserActivity({ data }: { data: BrowserOverview }) {
  const cats: AppCategory[] = ["productive", "neutral", "unproductive"];
  const maxMin = data.sites[0]?.minutes || 1;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Globe className="h-4 w-4 text-primary" /> Browser activity
        </CardTitle>
        <div className="flex items-center gap-2">
          <Badge tone="muted">{data.distinctSites} sites</Badge>
          <Badge tone="muted">{data.tabInstances} tabs opened</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <MiniStat icon={Clock} label="Browsing time" value={formatDuration(data.totalMinutes)} tone={brand.primary} />
          <MiniStat icon={Layers} label="Distinct sites" value={String(data.distinctSites)} tone={brand.info} />
          <MiniStat icon={Globe} label="Tabs opened" value={String(data.tabInstances)} tone={brand.pink} />
          <MiniStat
            icon={Gauge}
            label="Productive web"
            value={`${Math.round((data.byCategory.productive / (data.totalMinutes || 1)) * 100)}%`}
            tone={categoryColor.productive}
          />
        </div>

        <div>
          <div className="mb-1.5 flex h-2.5 overflow-hidden rounded-full bg-muted">
            {cats.map((c) =>
              data.byCategory[c] > 0 ? (
                <div
                  key={c}
                  style={{
                    width: `${(data.byCategory[c] / (data.totalMinutes || 1)) * 100}%`,
                    background: categoryColor[c],
                  }}
                />
              ) : null
            )}
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
            {cats.map((c) => (
              <span key={c} className="flex items-center gap-1.5">
                <span className="h-2.5 w-2.5 rounded-sm" style={{ background: categoryColor[c] }} />
                {catLabel[c]} · {formatDuration(data.byCategory[c])}
              </span>
            ))}
          </div>
        </div>

        <div className="space-y-2.5">
          {data.sites.map((s) => (
            <div key={s.domain} className="flex items-center gap-3">
              <span
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-[10px] font-bold text-white"
                style={{ background: s.color }}
              >
                {s.domain.replace("www.", "").charAt(0).toUpperCase()}
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-sm font-medium">{s.domain}</span>
                  <span className="shrink-0 text-sm tabular-nums text-muted-foreground">
                    {formatDuration(s.minutes)}
                  </span>
                </div>
                <div className="mt-1 flex items-center gap-2">
                  <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full"
                      style={{ width: `${(s.minutes / maxMin) * 100}%`, background: s.color }}
                    />
                  </div>
                  <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground">
                    <span className="h-1.5 w-1.5 rounded-full" style={{ background: categoryColor[s.category] }} />
                    {s.visits} {s.visits === 1 ? "tab" : "tabs"}
                  </span>
                </div>
              </div>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function MiniStat({
  icon: Icon,
  label,
  value,
  tone,
}: {
  icon: typeof Clock;
  label: string;
  value: string;
  tone: string;
}) {
  return (
    <div className="flex items-center gap-2.5 rounded-xl border border-border bg-muted/30 p-2.5">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg"
        style={{ background: `${tone}1a`, color: tone }}
      >
        <Icon className="h-4 w-4" />
      </div>
      <div className="min-w-0">
        <div className="truncate font-display text-sm font-bold tabular-nums">{value}</div>
        <div className="truncate text-[11px] text-muted-foreground">{label}</div>
      </div>
    </div>
  );
}

function TimelineRow({ seg }: { seg: DaySegment }) {
  const p = seg.projectId ? projectById(seg.projectId) : null;
  const Icon = typeIcon[seg.type];
  const isPassive = seg.type === "idle" || seg.type === "break";
  const dotColor = seg.type === "work" ? prodColor(seg.productivity) : typeColor[seg.type];

  return (
    <div className="relative flex gap-3 py-2.5">
      <div className="w-[44px] shrink-0 pt-1 text-right text-xs font-medium tabular-nums text-muted-foreground">
        {fmtMin(seg.startMin)}
      </div>
      <div className="relative z-10 mt-1 shrink-0">
        <span
          className="flex h-4 w-4 items-center justify-center rounded-full ring-4 ring-card"
          style={{ background: dotColor }}
        >
          <Icon className="h-2.5 w-2.5 text-white" />
        </span>
      </div>

      {isPassive ? (
        <div className="flex flex-1 flex-wrap items-center gap-2 text-sm text-muted-foreground">
          <span className="font-medium capitalize">{seg.type}</span>
          <span>·</span>
          <span className="tabular-nums">{formatDuration(seg.minutes)}</span>
          <span className="text-xs tabular-nums">
            ({fmtMin(seg.startMin)}–{fmtMin(seg.endMin)})
          </span>
        </div>
      ) : (
        <div className="flex flex-1 items-start gap-3 rounded-xl border border-border p-2.5 transition-colors hover:bg-muted/30">
          <ScreenMockView screen={seg.screen} className="hidden h-14 w-24 shrink-0 sm:block" />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <span className="truncate text-sm font-semibold">{seg.app}</span>
              {seg.type === "meeting" && <Badge tone="info">Meeting</Badge>}
              {seg.tabs && seg.tabs.length > 0 && (
                <Badge tone="muted" className="gap-1">
                  <Globe className="h-3 w-3" />
                  {seg.tabs.length} tabs
                </Badge>
              )}
            </div>
            <div className="truncate text-xs text-muted-foreground">{seg.windowTitle}</div>
            <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              {p && (
                <span className="flex items-center gap-1">
                  <span className="h-2 w-2 rounded-full" style={{ background: p.color }} /> {p.title}
                </span>
              )}
              <span className="tabular-nums">
                {fmtMin(seg.startMin)}–{fmtMin(seg.endMin)} · {formatDuration(seg.minutes)}
              </span>
              <span className="flex items-center gap-1">
                <MousePointerClick className="h-3 w-3" />
                {seg.mouseClicks.toLocaleString()}
              </span>
              <span className="flex items-center gap-1">
                <Keyboard className="h-3 w-3" />
                {seg.keyboardHits.toLocaleString()}
              </span>
            </div>

            {seg.tabs && seg.tabs.length > 0 && <TabList tabs={seg.tabs} />}
          </div>
          <div className="shrink-0">
            <Ring value={seg.productivity} size={38} stroke={4} />
          </div>
        </div>
      )}
    </div>
  );
}

function TabList({ tabs }: { tabs: NonNullable<DaySegment["tabs"]> }) {
  const [open, setOpen] = useState(false);
  const visible = open ? tabs : tabs.slice(0, 3);
  const hidden = tabs.length - visible.length;

  return (
    <div className="mt-2 rounded-lg border border-border/70 bg-muted/30 p-2">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        <Globe className="h-3 w-3" /> Open tabs
      </div>
      <div className="space-y-1">
        {visible.map((t) => (
          <div key={t.id} className="flex items-center gap-2 text-xs">
            <span className="h-2 w-2 shrink-0 rounded-sm" style={{ background: t.color }} />
            <span className="shrink-0 font-medium">{t.domain}</span>
            <span className="hidden min-w-0 flex-1 truncate text-muted-foreground sm:block">{t.title}</span>
            <span
              className="ml-auto shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium sm:ml-0"
              style={{ background: `${categoryColor[t.category]}1a`, color: categoryColor[t.category] }}
            >
              {catLabel[t.category]}
            </span>
            <span className="w-12 shrink-0 text-right tabular-nums text-muted-foreground">
              {formatDuration(t.minutes)}
            </span>
          </div>
        ))}
      </div>
      {tabs.length > 3 && (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="mt-1.5 text-[11px] font-medium text-primary hover:underline"
        >
          {open ? "Show less" : `+${hidden} more ${hidden === 1 ? "tab" : "tabs"}`}
        </button>
      )}
    </div>
  );
}
