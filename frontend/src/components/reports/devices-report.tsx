"use client";

import { useCallback, useMemo, useState } from "react";
import { Clock, MonitorSmartphone, PackageCheck, Wifi } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { API_ROUTES, platformLabel, presenceStatusLabel, reportingRoute } from "@/lib/api-contract";
import { downloadCSV } from "@/lib/export";
import { agoLabel, referenceNow } from "@/lib/live-session";
import {
  ALL,
  compareVersions,
  decimalHoursTotal,
  displayedTotal,
  latestAgentVersion,
  memberName,
  mergeDevices,
  type DeviceLine,
} from "@/lib/report-math";
import { trackedMembers } from "@/lib/roles";
import { activities, projectById, userById, users } from "@/lib/tenant-data";
import { formatInZone } from "@/lib/time-zone";
import type { Activity, User } from "@/lib/types";
import { decimalHours } from "@/lib/work-report";
import { cn } from "@/lib/utils";
import {
  Dash,
  ExportMenu,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  MemberCell,
  NoTimeState,
  RankedBars,
  ReportFootnote,
  ReportShell,
  ReportSkeleton,
  ReportToolbar,
  SectionHeading,
  demoActivities,
  useHoursFormat,
  usePeriodReport,
  useReportPeriod,
  useReportScope,
} from "./report-shell";

/* ============================================================================
 * DEVICES & AGENTS — every computer running the desktop agent: whose it is,
 * its platform and OS, the agent version (and whether it is the newest one in
 * the workspace), what it is doing right now, and how much time it tracked in
 * the period. Live: /reporting/devices for the period (in the reporting zone)
 * merged with /presence/team (the agents' heartbeats); presence is secondary —
 * without it the period's figures still stand. Blocks from agents too old to
 * send a device id are grouped as one "unknown device" per member.
 * ========================================================================== */

interface Loaded {
  lines: DeviceLine[];
  /** False when presence couldn't be read: nobody's live status is known. */
  presence: boolean;
  /** The instant the figures were read ("last seen" ages from it). */
  at: number;
}

interface Row extends DeviceLine {
  name: string;
}

/* ----------------------------- Demo devices ----------------------------- */

/** A stable small number from an id (demo device traits never change between renders). */
function seed(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h;
}

const DEMO_AGENT_VERSIONS = ["0.3.0", "0.3.0", "0.3.0", "0.2.4"];

/**
 * DEMO ONLY: one computer per tracked member, with traits that follow from
 * the member (platform, name, OS, agent version) and figures from their
 * seeded activity; presence follows their demo status. A live session never
 * reads it.
 */
function demoDevices(list: Activity[], roster: User[], now: number): { devices: unknown[]; presence: unknown[] } {
  const devices: unknown[] = [];
  const presence: unknown[] = [];
  for (const u of roster) {
    const s = seed(u.id);
    const mac = s % 3 === 0;
    const first = u.name.split(" ")[0] ?? u.name;
    const deviceId = `demo-device-${u.id}`;
    const traits = {
      userId: u.id,
      userName: u.name,
      deviceId,
      deviceName: mac ? `${first}'s MacBook Pro` : `${first.toUpperCase()}-LAPTOP`,
      platform: mac ? "macos" : "windows",
      osVersion: mac ? "macOS 14.5 (23F79)" : s % 2 ? "Windows 11 Pro 23H2 (10.0.22631)" : "Windows 10 Pro 22H2 (10.0.19045)",
      agentVersion: DEMO_AGENT_VERSIONS[s % DEMO_AGENT_VERSIONS.length],
    };
    const mine = list.filter((a) => a.userId === u.id);
    const minutes = mine.reduce((sum, a) => sum + Math.max(0, (Date.parse(a.endedAt) - Date.parse(a.startedAt)) / 60_000), 0);
    const last = mine.reduce((m, a) => Math.max(m, Date.parse(a.endedAt)), 0);
    if (mine.length) devices.push({ ...traits, trackedMinutes: minutes, lastActivityAt: new Date(last).toISOString() });
    const latest = [...activities].filter((a) => a.userId === u.id).sort((a, b) => Date.parse(b.endedAt) - Date.parse(a.endedAt))[0];
    const online = u.status !== "offline";
    presence.push({
      ...traits,
      status: u.status === "active" ? "tracking" : u.status === "idle" ? "idle" : "stopped",
      lastSeenAt: new Date(online ? now - 40_000 : latest ? Date.parse(latest.endedAt) : now - 6 * 3_600_000).toISOString(),
      projectTitle: online && latest ? (projectById(latest.projectId)?.title ?? null) : null,
      currentApp: u.status === "active" && latest ? latest.screen.app : null,
    });
  }
  return { devices, presence };
}

/* ----------------------------- Report ----------------------------- */

export function DevicesReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId } = scope;
  const range = useReportPeriod("30d", isLive);
  const { period, timeZone } = range;
  const { fmt, format } = useHoursFormat();
  const [openedAt] = useState(() => referenceNow().getTime());

  const load = useCallback(async (): Promise<Loaded> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [devices, presence] = await Promise.all([
      getApi(reportingRoute("devices", query)),
      // Live status is secondary: without it the period's devices still stand.
      getApi(API_ROUTES.presenceTeam).catch(() => null),
    ]);
    const at = Date.now();
    const mine = Array.isArray(presence) ? presence.filter((p) => memberId === ALL || (p as { userId?: unknown }).userId === memberId) : [];
    return { lines: mergeDevices(devices, mine, at, { knownOnly: projectId !== ALL }), presence: Array.isArray(presence), at };
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo<Loaded | null>(() => {
    if (isLive) return null;
    const onProject = projectId === ALL ? null : new Set(projectById(projectId)?.memberIds ?? []);
    const roster = trackedMembers(users).filter((u) => (memberId === ALL || u.id === memberId) && (!onProject || onProject.has(u.id)));
    const { devices, presence } = demoDevices(demoActivities(period, projectId, memberId), roster, openedAt);
    return { lines: mergeDevices(devices, presence, openedAt, { knownOnly: projectId !== ALL }), presence: true, at: openedAt };
  }, [isLive, period, projectId, memberId, openedAt]);

  // A live session shows only real devices (or nothing) — never the demo machines.
  const loaded = isLive ? live.data : demo;
  const pending = live.pending;

  const rows = useMemo<Row[]>(
    () => (loaded?.lines ?? []).map((l) => ({ ...l, name: memberName(l.userId, userById(l.userId) ?? (l.userName ? { name: l.userName } : null)) })),
    [loaded],
  );
  const latest = latestAgentVersion(rows);
  const withVersion = rows.filter((r) => r.agentVersion);
  const upToDate = withVersion.filter((r) => latest && r.agentVersion && compareVersions(r.agentVersion, latest) === 0).length;
  const online = rows.filter((r) => r.presence?.online).length;
  const platforms = { windows: rows.filter((r) => r.platform === "windows").length, macos: rows.filter((r) => r.platform === "macos").length };
  const tracked = rows.map((r) => r.trackedMinutes);
  const at = loaded?.at ?? openedAt;
  const versions = useMemo(() => {
    const counts = new Map<string, number>();
    for (const r of rows) counts.set(r.agentVersion ?? "", (counts.get(r.agentVersion ?? "") ?? 0) + 1);
    return [...counts].sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : compareVersions(b, a)));
  }, [rows]);
  const dash = "—";

  const statusOf = (r: Row): { label: string; tone: "success" | "warning" | "muted"; detail: string | null } => {
    const p = r.presence;
    if (!p) return { label: loaded?.presence ? "No heartbeat" : "Unknown", tone: "muted", detail: null };
    if (!p.online) return { label: "Offline", tone: "muted", detail: p.lastSeenAt ? `last seen ${agoLabel(new Date(p.lastSeenAt).toISOString(), new Date(at))}` : null };
    // Heard from just now, but the agent says it stopped or signed out.
    if (p.status === "stopped" || p.status === "signedout") return { label: presenceStatusLabel(p.status), tone: "muted", detail: null };
    return {
      label: presenceStatusLabel(p.status),
      tone: p.status === "tracking" ? "success" : "warning",
      detail: [p.projectTitle, p.currentApp].filter(Boolean).join(" · ") || null,
    };
  };
  const lastActivity = (r: Row) =>
    r.lastActivityAt === null ? null : formatInZone(r.lastActivityAt, timeZone, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <MemberCell name={r.name} /> },
    {
      key: "device",
      header: "Device",
      sortValue: (r) => r.deviceName ?? "",
      render: (r) => (
        <div className="min-w-0">
          <div className="truncate font-medium">{r.deviceName ?? (r.deviceId ? "Unnamed device" : "Unknown device")}</div>
          <div className="truncate text-xs text-muted-foreground">{r.deviceId ? (r.osVersion ?? platformLabel(r.platform) ?? "OS not reported") : "Older agent — no device id"}</div>
        </div>
      ),
    },
    {
      key: "platform",
      header: "Platform",
      sortValue: (r) => r.platform ?? "",
      render: (r) => (platformLabel(r.platform) ? <span>{platformLabel(r.platform)}</span> : <Dash label="Not reported" />),
    },
    {
      key: "agent",
      header: "Agent",
      sortValue: (r) => r.agentVersion ?? "",
      render: (r) =>
        r.agentVersion ? (
          <span className="flex flex-wrap items-center gap-1.5">
            <span className="tabular-nums">{r.agentVersion}</span>
            {latest && compareVersions(r.agentVersion, latest) < 0 ? (
              <Badge tone="warning" className="whitespace-nowrap">
                Update available
              </Badge>
            ) : (
              <Badge tone="success">Latest</Badge>
            )}
          </span>
        ) : (
          <Dash label="Not reported" />
        ),
    },
    {
      key: "status",
      header: "Status",
      sortValue: (r) => (r.presence?.online ? 1 : 0),
      render: (r) => {
        const s = statusOf(r);
        return (
          <div className="min-w-0">
            <Badge tone={s.tone} className="gap-1.5">
              <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", s.tone === "success" ? "bg-success" : s.tone === "warning" ? "bg-warning" : "bg-muted-foreground")} />
              {s.label}
            </Badge>
            {s.detail && <div className="mt-0.5 truncate text-xs text-muted-foreground">{s.detail}</div>}
          </div>
        );
      },
    },
    {
      key: "tracked",
      header: "Tracked",
      align: "right",
      sortValue: (r) => r.trackedMinutes,
      render: (r) => (r.trackedMinutes > 0 ? <span className="font-semibold tabular-nums">{fmt(r.trackedMinutes)}</span> : <Dash label="Nothing tracked in this period" />),
    },
    {
      key: "last",
      header: "Last activity",
      align: "right",
      sortValue: (r) => r.lastActivityAt ?? 0,
      render: (r) => (lastActivity(r) ? <span className="whitespace-nowrap tabular-nums text-muted-foreground">{lastActivity(r)}</span> : <Dash label="None in this period" />),
    },
  ];

  function exportCsv() {
    const headers = ["Member", "Device", "Platform", "OS version", "Agent version", "Status", "Last heartbeat (UTC)", "Tracked hours (decimal)", "Last activity (UTC)"];
    const body: (string | number)[][] = rows.map((r) => [
      r.name,
      r.deviceName ?? (r.deviceId ? "Unnamed device" : "Unknown device (older agent)"),
      platformLabel(r.platform) ?? "",
      r.osVersion ?? "",
      r.agentVersion ?? "",
      statusOf(r).label,
      r.presence?.lastSeenAt ? new Date(r.presence.lastSeenAt).toISOString() : "",
      decimalHours(r.trackedMinutes),
      r.lastActivityAt === null ? "" : new Date(r.lastActivityAt).toISOString(),
    ]);
    body.push(["Total", "", "", "", "", "", "", decimalHoursTotal(tracked), ""]);
    downloadCSV(`devices-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  return (
    <ReportShell
      aiExplain={{ slug: "devices", query: { from: period.from, to: period.to, projectId, userId: memberId, timeZone } }}
      title="Devices & Agents"
      description="Every computer running the desktop agent: platform, agent version, live status and time tracked."
      actions={<ExportMenu onExportCSV={exportCsv} disabled={pending || rows.length === 0} />}
    >
      <ReportToolbar
        period={range}
        projectId={projectId}
        onProject={scope.setProjectId}
        memberId={memberId}
        onMember={scope.setMemberId}
        liveWorkerId={scope.liveWorkerId}
      />

      {isLive && (
        <div className="no-print">
          <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />
        </div>
      )}

      <KpiGrid>
        <Kpi
          label="Devices"
          icon={MonitorSmartphone}
          accent="blue"
          value={pending ? dash : String(rows.length)}
          sub={pending ? undefined : `${platforms.windows} Windows · ${platforms.macos} macOS`}
        />
        <Kpi
          label="Online now"
          icon={Wifi}
          accent="green"
          value={pending || !loaded?.presence ? dash : String(online)}
          sub={pending ? undefined : loaded?.presence ? "heartbeat in the last 2½ minutes" : "live status couldn't be loaded"}
        />
        <Kpi
          label="On the newest agent"
          icon={PackageCheck}
          accent="violet"
          value={pending || !latest ? dash : `${upToDate} of ${withVersion.length}`}
          sub={pending ? undefined : latest ? `version ${latest}` : "versions not reported"}
        />
        <Kpi label="Tracked" icon={Clock} accent="amber" value={pending ? dash : fmt(displayedTotal(tracked, format))} sub={pending ? undefined : period.label} />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : rows.length === 0 ? (
        <NoTimeState
          period={period}
          what="No devices in this period"
          onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined}
        />
      ) : (
        <>
          <Card>
            <SectionHeading title="Devices" sub={`${rows.length} ${rows.length === 1 ? "device" : "devices"} · online first`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} emptyText="No devices." />
            </CardContent>
          </Card>
          <div className="grid grid-cols-1 gap-5 md:grid-cols-2">
            <Card>
              <SectionHeading title="Agent versions" sub="Devices per version" />
              <CardContent>
                <RankedBars
                  label="Devices per agent version"
                  items={versions.map(([version, count]) => ({
                    key: version || "unknown",
                    label: version ? `Version ${version}` : "Not reported",
                    value: count,
                    display: String(count),
                    color: !version ? "var(--muted-foreground)" : version === latest ? "var(--success)" : "var(--warning)",
                    badge: version && version === latest ? <Badge tone="success" className="shrink-0">Latest</Badge> : undefined,
                  }))}
                />
              </CardContent>
            </Card>
            <Card>
              <SectionHeading title="Platforms" sub="Devices per operating system" />
              <CardContent>
                <RankedBars
                  label="Devices per platform"
                  items={[
                    { key: "windows", label: "Windows", value: platforms.windows, display: String(platforms.windows), color: "var(--primary)" },
                    { key: "macos", label: "macOS", value: platforms.macos, display: String(platforms.macos), color: "#7c3aed" },
                    ...(rows.length > platforms.windows + platforms.macos
                      ? [
                          {
                            key: "unknown",
                            label: "Not reported",
                            value: rows.length - platforms.windows - platforms.macos,
                            display: String(rows.length - platforms.windows - platforms.macos),
                            color: "var(--muted-foreground)",
                          },
                        ]
                      : []),
                  ]}
                />
              </CardContent>
            </Card>
          </div>
        </>
      )}

      <ReportFootnote>
        Devices are told apart by the id each agent install creates; blocks from agents too old to send one are grouped as an unknown
        device per member. Status comes from the agents&apos; heartbeats: a device is online when it was heard from in the last 2½
        minutes. &quot;Latest&quot; is the newest agent version seen in this workspace. Tracked time and last activity cover the period,
        on calendar days of the reporting time zone shown above.
      </ReportFootnote>
    </ReportShell>
  );
}
