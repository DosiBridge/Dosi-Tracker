"use client";

import { useMemo, useState } from "react";
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
  CalendarDays,
  MonitorDot,
  LogIn,
  LogOut,
  Zap,
  List,
  CalendarRange,
  Globe,
  Layers,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Progress } from "@/components/ui/progress";
import { Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/modal";
import { HourlyChart } from "@/components/dashboard/charts";
import { ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { projectById, users } from "@/lib/tenant-data";
import {
  DAY_END_MIN,
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
import { cn, formatDuration } from "@/lib/utils";

const catLabel: Record<AppCategory, string> = {
  productive: "Productive",
  neutral: "Neutral",
  unproductive: "Unproductive",
};

const RANGE = DAY_END_MIN - DAY_START_MIN;

type MonitorView = "timeline" | "schedule";

const typeColor: Record<SegmentType, string> = {
  work: "#22c55e",
  meeting: "#0ea5e9",
  break: "#f59e0b",
  idle: "#94a3b8",
};

function prodColor(p: number): string {
  if (p >= 75) return "#22c55e";
  if (p >= 50) return "#f59e0b";
  return "#ef4444";
}

const typeIcon: Record<SegmentType, typeof Coffee> = {
  work: Zap,
  meeting: Video,
  break: Coffee,
  idle: Pause,
};

export default function MonitorPage() {
  const { user } = useSession();
  const days = useMemo(() => availableDays(7), []);
  const monitorable = users.filter((u) => u.role !== "client");
  const isSelfOnly = user.role === "worker";

  const [userId, setUserId] = useState(() =>
    isSelfOnly ? user.id : (monitorable.find((u) => u.role === "worker")?.id ?? user.id)
  );
  const [dayIdx, setDayIdx] = useState(0);
  const [view, setView] = useState<MonitorView>("timeline");

  const activeUserId = isSelfOnly ? user.id : userId;
  const iso = days[dayIdx].iso;
  const member = users.find((u) => u.id === activeUserId)!;

  const segments = useMemo(() => buildDayTimeline(activeUserId, iso), [activeUserId, iso]);
  const summary = useMemo(() => daySummary(segments), [segments]);
  const hourly = useMemo(() => hourlyBuckets(segments), [segments]);
  const apps = useMemo(() => appBreakdown(segments), [segments]);
  const browser = useMemo(() => browserSummary(segments), [segments]);
  const shots = segments.filter((s) => s.type === "work" || s.type === "meeting");

  const hourTicks = Array.from({ length: RANGE / 60 / 2 + 1 }, (_, i) => DAY_START_MIN + i * 120);

  const focusCard = (
    <Card>
      <CardHeader><CardTitle>Focus by hour</CardTitle><Badge tone="muted">Minutes</Badge></CardHeader>
      <CardContent><HourlyChart data={hourly} /></CardContent>
    </Card>
  );

  const appsCard = (
    <Card>
      <CardHeader><CardTitle>Apps used</CardTitle></CardHeader>
      <CardContent className="space-y-2.5">
        {apps.map((a) => (
          <div key={a.app}>
            <div className="mb-1 flex items-center justify-between text-sm">
              <span className="flex items-center gap-2 truncate">
                <span className="h-2.5 w-2.5 rounded-full" style={{ background: a.accent }} />
                <span className="truncate">{a.app}</span>
              </span>
              <span className="text-muted-foreground">{formatDuration(a.minutes)}</span>
            </div>
            <Progress value={(a.minutes / (apps[0]?.minutes || 1)) * 100} color={a.accent} />
          </div>
        ))}
      </CardContent>
    </Card>
  );

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <MonitorDot className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">Member Monitor</h1>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            A detailed daily timeline of what each member worked on and when.
          </p>
        </div>
      </div>

      {/* Controls */}
      <Card className="p-3">
        <div className="flex flex-wrap items-center gap-3">
          {/* Member */}
          <div className="flex items-center gap-2">
            <Avatar name={member.name} size="sm" status={member.status} />
            {isSelfOnly ? (
              <div className="text-sm font-medium">{member.name}</div>
            ) : (
              <Select value={userId} onChange={(e) => setUserId(e.target.value)} className="w-auto min-w-48">
                {monitorable.map((u) => (
                  <option key={u.id} value={u.id}>{u.name} · {u.designation}</option>
                ))}
              </Select>
            )}
          </div>

          {/* View switch */}
          <div className="ml-auto flex items-center gap-1 rounded-lg border border-border bg-card p-1">
            <button
              onClick={() => setView("timeline")}
              className={cn(
                "flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
                view === "timeline" ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground"
              )}
            >
              <List className="h-4 w-4" /> Timeline
            </button>
            <button
              onClick={() => setView("schedule")}
              className={cn(
                "flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
                view === "schedule" ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground"
              )}
            >
              <CalendarRange className="h-4 w-4" /> Schedule
            </button>
          </div>

          {/* Date nav */}
          <div className="flex items-center gap-2">
            <Button variant="outline" size="icon" onClick={() => setDayIdx((i) => Math.min(i + 1, days.length - 1))} disabled={dayIdx >= days.length - 1} aria-label="Previous day">
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <div className="flex min-w-32 items-center justify-center gap-2 rounded-lg border border-border px-3 py-2 text-sm">
              <CalendarDays className="h-4 w-4 text-muted-foreground" />
              <span className="font-medium">{days[dayIdx].isToday ? "Today" : `${days[dayIdx].weekday}, ${days[dayIdx].label}`}</span>
            </div>
            <Button variant="outline" size="icon" onClick={() => setDayIdx((i) => Math.max(i - 1, 0))} disabled={dayIdx === 0} aria-label="Next day">
              <ChevronRight className="h-4 w-4" />
            </Button>
            {dayIdx !== 0 && (
              <Button variant="ghost" size="sm" onClick={() => setDayIdx(0)}>Today</Button>
            )}
          </div>
        </div>
      </Card>

      {segments.length === 0 ? (
        <Card className="flex flex-col items-center justify-center gap-3 py-20 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <Pause className="h-6 w-6" />
          </div>
          <div>
            <p className="font-medium">No activity tracked</p>
            <p className="mt-1 text-sm text-muted-foreground">{member.name.split(" ")[0]} didn&apos;t track any time on this day.</p>
          </div>
        </Card>
      ) : (
        <>
          {/* KPIs */}
          <div className="grid grid-cols-2 gap-4 lg:grid-cols-4 xl:grid-cols-6">
            <Kpi icon={Clock} tone="#6d5efc" label="Tracked" value={formatDuration(summary.tracked)} />
            <Kpi icon={Gauge} tone={prodColor(summary.productivityAvg)} label="Productivity" value={`${summary.productivityAvg}%`} />
            <Kpi icon={Zap} tone="#22c55e" label="Longest focus" value={formatDuration(summary.longestFocus)} />
            <Kpi icon={Pause} tone="#94a3b8" label="Idle + breaks" value={formatDuration(summary.idle + summary.breakMinutes)} />
            <Kpi icon={LogIn} tone="#0ea5e9" label="First seen" value={summary.firstMin !== null ? fmtMin(summary.firstMin) : "—"} />
            <Kpi icon={LogOut} tone="#ec4899" label="Last seen" value={summary.lastMin !== null ? fmtMin(summary.lastMin) : "—"} />
          </div>

          {/* Activity band */}
          <Card>
            <CardHeader>
              <CardTitle>Activity timeline</CardTitle>
              <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                {(["work", "meeting", "break", "idle"] as SegmentType[]).map((t) => (
                  <span key={t} className="flex items-center gap-1.5 capitalize">
                    <span className="h-2.5 w-2.5 rounded-sm" style={{ background: typeColor[t] }} /> {t}
                  </span>
                ))}
              </div>
            </CardHeader>
            <CardContent>
              <div className="relative h-16 w-full rounded-lg bg-muted/50">
                {/* hour gridlines */}
                {hourTicks.map((t) => (
                  <div
                    key={t}
                    className="absolute top-0 bottom-5 border-l border-border/70"
                    style={{ left: `${((t - DAY_START_MIN) / RANGE) * 100}%` }}
                  />
                ))}
                {/* segments */}
                {segments.map((s) => {
                  const left = ((s.startMin - DAY_START_MIN) / RANGE) * 100;
                  const width = (s.minutes / RANGE) * 100;
                  const color = s.type === "work" ? prodColor(s.productivity) : typeColor[s.type];
                  return (
                    <div
                      key={s.id}
                      title={`${fmtMin(s.startMin)}–${fmtMin(s.endMin)} · ${s.type === "idle" || s.type === "break" ? s.windowTitle : s.app}`}
                      className={cn(
                        "absolute top-1.5 bottom-6 rounded-sm transition-all hover:brightness-110",
                        (s.type === "idle" || s.type === "break") && "opacity-50"
                      )}
                      style={{ left: `${left}%`, width: `calc(${width}% - 1px)`, background: color }}
                    />
                  );
                })}
                {/* hour labels */}
                {hourTicks.map((t) => (
                  <span
                    key={`l-${t}`}
                    className="absolute bottom-0 -translate-x-1/2 text-[10px] text-muted-foreground"
                    style={{ left: `${((t - DAY_START_MIN) / RANGE) * 100}%` }}
                  >
                    {fmtMin(t)}
                  </span>
                ))}
              </div>
            </CardContent>
          </Card>

          {view === "timeline" ? (
            <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
              {/* Detailed timeline */}
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

              {/* Right column */}
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

          {/* Browser activity */}
          {browser.tabInstances > 0 && <BrowserActivity data={browser} />}

          {/* Screenshots strip */}
          <Card>
            <CardHeader><CardTitle>Screenshots timeline</CardTitle><Badge tone="muted">{shots.length} captures</Badge></CardHeader>
            <CardContent>
              <div className="flex gap-3 overflow-x-auto pb-2">
                {shots.map((s) => (
                  <div key={s.id} className="w-44 shrink-0">
                    <ScreenMockView screen={s.screen} title={s.windowTitle} className="aspect-video w-full" />
                    <div className="mt-1.5 flex items-center justify-between text-xs">
                      <span className="font-medium">{fmtMin(s.startMin)}</span>
                      <span className="text-muted-foreground">{s.app}</span>
                    </div>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}

function DayScheduleView({ segments }: { segments: DaySegment[] }) {
  const PX_PER_MIN = 1.25;
  const height = RANGE * PX_PER_MIN;
  const hours = Array.from({ length: RANGE / 60 + 1 }, (_, i) => DAY_START_MIN + i * 60);
  const [selected, setSelected] = useState<DaySegment | null>(null);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Day schedule</CardTitle>
        <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
          {(["work", "meeting", "break", "idle"] as SegmentType[]).map((t) => (
            <span key={t} className="flex items-center gap-1.5 capitalize">
              <span className="h-2.5 w-2.5 rounded-sm" style={{ background: typeColor[t] }} /> {t}
            </span>
          ))}
        </div>
      </CardHeader>
      <CardContent>
        <p className="mb-2 text-xs text-muted-foreground">Click any block to see full details and open tabs.</p>
        <div className="flex">
          {/* Hour axis */}
          <div className="relative w-14 shrink-0" style={{ height }}>
            {hours.map((h) => (
              <span
                key={h}
                className="absolute right-2 -translate-y-1/2 text-[11px] font-medium text-muted-foreground"
                style={{ top: (h - DAY_START_MIN) * PX_PER_MIN }}
              >
                {fmtMin(h)}
              </span>
            ))}
          </div>

          {/* Day column */}
          <div className="relative flex-1 overflow-hidden rounded-lg border border-border bg-muted/20" style={{ height }}>
            {/* hour gridlines */}
            {hours.map((h) => (
              <div
                key={h}
                className="absolute left-0 right-0 border-t border-border/60"
                style={{ top: (h - DAY_START_MIN) * PX_PER_MIN }}
              />
            ))}
            {/* half-hour lighter lines */}
            {hours.slice(0, -1).map((h) => (
              <div
                key={`half-${h}`}
                className="absolute left-0 right-0 border-t border-dashed border-border/30"
                style={{ top: (h + 30 - DAY_START_MIN) * PX_PER_MIN }}
              />
            ))}
            {/* segments */}
            {segments.map((s) => (
              <ScheduleBlock key={s.id} seg={s} pxPerMin={PX_PER_MIN} onSelect={setSelected} />
            ))}
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
        <div className="flex items-center gap-2">
          <Badge tone="muted" className="capitalize">{seg.type}</Badge>
          {!isPassive && (
            <Badge tone="muted" style={{ color }}>{seg.productivity}% productive</Badge>
          )}
          {seg.tabs && seg.tabs.length > 0 && (
            <Badge tone="muted" className="gap-1"><Globe className="h-3 w-3" />{seg.tabs.length} tabs</Badge>
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
                <span className="flex items-center gap-1"><MousePointerClick className="h-3.5 w-3.5" />{seg.mouseClicks.toLocaleString()} clicks</span>
                <span className="flex items-center gap-1"><Keyboard className="h-3.5 w-3.5" />{seg.keyboardHits.toLocaleString()} keys</span>
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
                <div key={t.id} className="flex items-center gap-2.5 rounded-lg border border-border p-2">
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
                  <span className="w-12 shrink-0 text-right text-xs text-muted-foreground">{formatDuration(t.minutes)}</span>
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
        {/* Summary strip */}
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <MiniStat icon={Clock} label="Browsing time" value={formatDuration(data.totalMinutes)} tone="#6d5efc" />
          <MiniStat icon={Layers} label="Distinct sites" value={String(data.distinctSites)} tone="#0ea5e9" />
          <MiniStat icon={Globe} label="Tabs opened" value={String(data.tabInstances)} tone="#ec4899" />
          <MiniStat
            icon={Gauge}
            label="Productive web"
            value={`${Math.round((data.byCategory.productive / (data.totalMinutes || 1)) * 100)}%`}
            tone={categoryColor.productive}
          />
        </div>

        {/* Category split bar */}
        <div>
          <div className="mb-1.5 flex h-2.5 overflow-hidden rounded-full bg-muted">
            {cats.map((c) =>
              data.byCategory[c] > 0 ? (
                <div
                  key={c}
                  style={{ width: `${(data.byCategory[c] / (data.totalMinutes || 1)) * 100}%`, background: categoryColor[c] }}
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

        {/* Per-site breakdown */}
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
                  <span className="shrink-0 text-sm text-muted-foreground">{formatDuration(s.minutes)}</span>
                </div>
                <div className="mt-1 flex items-center gap-2">
                  <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                    <div className="h-full rounded-full" style={{ width: `${(s.minutes / maxMin) * 100}%`, background: s.color }} />
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

function MiniStat({ icon: Icon, label, value, tone }: { icon: typeof Clock; label: string; value: string; tone: string }) {
  return (
    <div className="flex items-center gap-2.5 rounded-lg border border-border bg-muted/30 p-2.5">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg" style={{ background: `${tone}1a`, color: tone }}>
        <Icon className="h-4 w-4" />
      </div>
      <div className="min-w-0">
        <div className="truncate text-sm font-bold">{value}</div>
        <div className="truncate text-[11px] text-muted-foreground">{label}</div>
      </div>
    </div>
  );
}

function Kpi({ icon: Icon, tone, label, value }: { icon: typeof Clock; tone: string; label: string; value: string }) {
  return (
    <Card className="p-3">
      <div className="flex items-center gap-2.5">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg" style={{ background: `${tone}1a`, color: tone }}>
          <Icon className="h-4 w-4" />
        </div>
        <div className="min-w-0">
          <div className="truncate text-base font-bold">{value}</div>
          <div className="truncate text-[11px] text-muted-foreground">{label}</div>
        </div>
      </div>
    </Card>
  );
}

function TimelineRow({ seg }: { seg: DaySegment }) {
  const p = seg.projectId ? projectById(seg.projectId) : null;
  const Icon = typeIcon[seg.type];
  const isPassive = seg.type === "idle" || seg.type === "break";
  const dotColor = seg.type === "work" ? prodColor(seg.productivity) : typeColor[seg.type];

  return (
    <div className="relative flex gap-3 py-2.5">
      {/* time */}
      <div className="w-[44px] shrink-0 pt-1 text-right text-xs font-medium text-muted-foreground">
        {fmtMin(seg.startMin)}
      </div>
      {/* node */}
      <div className="relative z-10 mt-1 shrink-0">
        <span className="flex h-4 w-4 items-center justify-center rounded-full ring-4 ring-card" style={{ background: dotColor }}>
          <Icon className="h-2.5 w-2.5 text-white" />
        </span>
      </div>

      {isPassive ? (
        <div className="flex flex-1 items-center gap-2 text-sm text-muted-foreground">
          <span className="font-medium capitalize">{seg.type}</span>
          <span>·</span>
          <span>{formatDuration(seg.minutes)}</span>
          <span className="text-xs">({fmtMin(seg.startMin)}–{fmtMin(seg.endMin)})</span>
        </div>
      ) : (
        <div className="flex flex-1 items-start gap-3 rounded-xl border border-border p-2.5">
          <ScreenMockView screen={seg.screen} className="hidden h-14 w-24 shrink-0 sm:block" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
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
              <span>{fmtMin(seg.startMin)}–{fmtMin(seg.endMin)} · {formatDuration(seg.minutes)}</span>
              <span className="flex items-center gap-1"><MousePointerClick className="h-3 w-3" />{seg.mouseClicks.toLocaleString()}</span>
              <span className="flex items-center gap-1"><Keyboard className="h-3 w-3" />{seg.keyboardHits.toLocaleString()}</span>
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
            <span className="w-12 shrink-0 text-right text-muted-foreground">{formatDuration(t.minutes)}</span>
          </div>
        ))}
      </div>
      {tabs.length > 3 && (
        <button
          onClick={() => setOpen((o) => !o)}
          className="mt-1.5 text-[11px] font-medium text-primary hover:underline"
        >
          {open ? "Show less" : `+${hidden} more ${hidden === 1 ? "tab" : "tabs"}`}
        </button>
      )}
    </div>
  );
}
