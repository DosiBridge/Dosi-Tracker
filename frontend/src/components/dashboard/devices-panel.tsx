"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Laptop, Monitor, MonitorSmartphone, RotateCw, Search } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { getDeviceReport, getTeamPresence } from "@/hooks/useApi";
import { platformLabel, type DeviceReportRowDto, type PresenceDto } from "@/lib/api-contract";
import { referenceNow } from "@/lib/live-session";
import { formatInZone, zonedPeriod } from "@/lib/time-zone";
import { activities, users } from "@/lib/tenant-data";
import { cn, formatDuration } from "@/lib/utils";
import { requestErrorText } from "@/components/layout/request-error";
import {
  buildDeviceRows,
  demoDeviceSources,
  deviceStatusText,
  deviceStatusTone,
  newestAgentVersion,
  osText,
  type DeviceRow,
} from "./devices";

/* ============================================================================
 * The workspace's devices: who runs the desktop agent where, on which OS and
 * agent version, what it is doing now and when it was last heard from. Live
 * sessions read the presence and device-report endpoints (contract §3, §6);
 * the demo workspace derives its devices from the demo roster.
 * ========================================================================== */

/** How far back the device report looks for devices (and their tracked time). */
const REPORT_DAYS = 30;
/** How often the live list refreshes while it is visible. */
const REFRESH_MS = 60_000;

const platformIcon: Record<string, LucideIcon> = { windows: Monitor, macos: Laptop };

const toneDot: Record<ReturnType<typeof deviceStatusTone>, string> = {
  tracking: "bg-success",
  away: "bg-warning",
  offline: "bg-muted-foreground/60",
};

interface Sources {
  presence: PresenceDto[];
  report: DeviceReportRowDto[];
  /** Which source failed (the other still renders). */
  presenceFailed: boolean;
  reportFailed: boolean;
  loadedAt: Date;
}

export function DevicesPanel({
  live,
  zone,
  nameOf,
}: {
  live: boolean;
  /** The reporting zone (useReportingZone): the report's days and the "last seen" times are read in it. */
  zone: { timeZone: string; ready: boolean };
  /** The roster's name for a member id (the hint is the name the server sent). */
  nameOf: (userId: string, hint: string | null) => string;
}) {
  const [sources, setSources] = useState<Sources | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(live);
  const [query, setQuery] = useState("");
  const [onlyOutdated, setOnlyOutdated] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    const now = new Date();
    const range = zonedPeriod("30d", now, zone.timeZone);
    const [presence, report] = await Promise.allSettled([
      getTeamPresence(),
      getDeviceReport({ from: range.from, to: range.to, timeZone: zone.timeZone }),
    ]);
    if (presence.status === "rejected" && report.status === "rejected") {
      setError(requestErrorText(presence.reason, "Couldn't load devices from the server."));
      setSources(null);
    } else {
      setError(null);
      setSources({
        presence: presence.status === "fulfilled" ? presence.value : [],
        report: report.status === "fulfilled" ? report.value : [],
        presenceFailed: presence.status === "rejected",
        reportFailed: report.status === "rejected",
        loadedAt: new Date(),
      });
    }
    setLoading(false);
  }, [zone.timeZone]);

  useEffect(() => {
    if (!live || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
    // Live status changes every minute: refresh while the page is visible.
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, REFRESH_MS);
    return () => clearInterval(timer);
  }, [live, zone.ready, load]);

  const now = live ? (sources?.loadedAt ?? new Date()) : referenceNow();
  const rows = useMemo<DeviceRow[]>(() => {
    if (!live) {
      const demo = demoDeviceSources(users, activities, referenceNow());
      return buildDeviceRows(demo.presence, demo.report, nameOf, referenceNow());
    }
    return sources ? buildDeviceRows(sources.presence, sources.report, nameOf, sources.loadedAt) : [];
  }, [live, sources, nameOf]);

  const newest = newestAgentVersion(rows.map((r) => r.agentVersion));
  const onlineCount = rows.filter((r) => r.online).length;
  const outdatedCount = rows.filter((r) => r.outdated).length;
  const filtered = rows.filter((r) => {
    if (onlyOutdated && !r.outdated) return false;
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return [r.memberName, r.deviceName ?? "", osText(r), r.agentVersion ?? ""].some((v) => v.toLowerCase().includes(q));
  });

  const lastSeenTitle = (iso: string | null) => (iso ? formatInZone(Date.parse(iso), zone.timeZone, { dateStyle: "medium", timeStyle: "short" }) : undefined);

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 rounded-2xl border border-border bg-card/80 p-3 sm:flex-row sm:flex-wrap sm:items-center">
        <div className="relative min-w-0 flex-1">
          <Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search member, device, OS or version…"
            aria-label="Search devices"
            className="h-10 w-full rounded-xl border border-border bg-card pl-9 pr-3 text-sm placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
        <label className="inline-flex cursor-pointer items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={onlyOutdated}
            onChange={(e) => setOnlyOutdated(e.target.checked)}
            className="h-4 w-4 rounded border-border accent-[var(--primary)]"
          />
          Needs an update only
        </label>
        {live && (
          <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
            <RotateCw aria-hidden className={cn("h-3.5 w-3.5", loading && "animate-spin motion-reduce:animate-none")} /> Refresh
          </Button>
        )}
      </div>

      <dl className="grid grid-cols-3 gap-3" aria-label="Device summary">
        {[
          { label: "Devices", value: rows.length },
          { label: "Online now", value: onlineCount },
          { label: "Need an update", value: outdatedCount },
        ].map((s) => (
          <div key={s.label} className="rounded-2xl border border-border bg-card p-3 card-elev sm:p-4">
            <dt className="text-xs text-muted-foreground">{s.label}</dt>
            <dd className="mt-1 font-display text-xl font-bold tabular-nums text-ink">{loading && !sources && live ? "…" : s.value}</dd>
          </div>
        ))}
      </dl>
      {newest && (
        <p className="text-xs text-muted-foreground">
          Newest agent version in this workspace: <span className="font-medium text-foreground">{newest}</span>. Older agents are
          flagged so they can be updated from the Desktop agent page.
        </p>
      )}

      {error ? (
        <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-sm text-danger-on-tint">
          <span className="min-w-0 flex-1">{error}</span>
          <Button size="sm" variant="outline" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      ) : (
        sources &&
        (sources.presenceFailed || sources.reportFailed) && (
          <p role="status" className="rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
            {sources.presenceFailed
              ? "Live status isn't available from this server, so devices show their last upload only."
              : "Uploaded time couldn't be loaded; devices show their live status only."}
          </p>
        )
      )}

      {live && loading && !sources && !error ? (
        <p role="status" className="py-10 text-center text-sm text-muted-foreground">
          Loading devices…
        </p>
      ) : !error && rows.length === 0 ? (
        <EmptyState
          icon={MonitorSmartphone}
          title="No devices yet"
          description="A device appears here once someone signs in to the desktop agent on it."
        />
      ) : !error && filtered.length === 0 ? (
        <EmptyState
          icon={Search}
          title="No devices match"
          description="Try another search, or show every device."
          action={{
            label: "Show all devices",
            onClick: () => {
              setQuery("");
              setOnlyOutdated(false);
            },
          }}
        />
      ) : !error ? (
        <>
          {/* Phones: one card per device. */}
          <ul className="space-y-3 md:hidden">
            {filtered.map((r) => (
              <li key={r.key}>
                <Card className="p-4">
                  <DeviceSummary row={r} now={now} lastSeenTitle={lastSeenTitle(r.lastSeenAt)} />
                </Card>
              </li>
            ))}
          </ul>
          {/* Wider screens: a table. */}
          <Card className="hidden overflow-hidden p-0 md:block">
            <div className="overflow-x-auto">
              <table className="w-full min-w-[760px] text-sm">
                <caption className="sr-only">Devices running the desktop agent</caption>
                <thead>
                  <tr className="border-b border-border text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th scope="col" className="px-4 py-3 font-semibold">Member</th>
                    <th scope="col" className="px-4 py-3 font-semibold">Device</th>
                    <th scope="col" className="px-4 py-3 font-semibold">Operating system</th>
                    <th scope="col" className="px-4 py-3 font-semibold">Agent</th>
                    <th scope="col" className="px-4 py-3 font-semibold">Status</th>
                    <th scope="col" className="px-4 py-3 text-right font-semibold">Tracked ({REPORT_DAYS} days)</th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((r) => {
                    const Icon = (r.platform && platformIcon[r.platform]) || MonitorSmartphone;
                    const tone = deviceStatusTone(r);
                    return (
                      <tr key={r.key} className="border-b border-border/60 last:border-0 hover:bg-muted/40">
                        <th scope="row" className="px-4 py-3 text-left font-medium">
                          {r.memberName}
                        </th>
                        <td className="px-4 py-3">
                          <span className="flex items-center gap-2">
                            <Icon aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
                            <span className="min-w-0 truncate">{r.deviceName ?? (r.deviceId ? "Unnamed device" : "Unknown device")}</span>
                          </span>
                        </td>
                        <td className="px-4 py-3 text-muted-foreground">
                          <span className="block">{osText(r)}</span>
                          {r.osVersion && platformLabel(r.platform) && (
                            <span className="text-xs text-muted-foreground/80">{platformLabel(r.platform)}</span>
                          )}
                        </td>
                        <td className="px-4 py-3">
                          <VersionCell row={r} newest={newest} />
                        </td>
                        <td className="px-4 py-3">
                          <span className="flex items-center gap-2" title={lastSeenTitle(r.lastSeenAt)}>
                            <span aria-hidden className={cn("h-2 w-2 shrink-0 rounded-full", toneDot[tone])} />
                            <span>{deviceStatusText(r, now)}</span>
                          </span>
                          {r.online && r.projectTitle && <span className="mt-0.5 block text-xs text-muted-foreground">{r.projectTitle}</span>}
                        </td>
                        <td className="px-4 py-3 text-right tabular-nums">{formatDuration(r.trackedMinutes)}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </Card>
        </>
      ) : null}
    </div>
  );
}

function VersionCell({ row, newest }: { row: DeviceRow; newest: string | null }) {
  if (!row.agentVersion) return <span className="text-xs text-muted-foreground">Not reported</span>;
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <span className="font-mono text-xs">{row.agentVersion}</span>
      {row.outdated && (
        <Badge tone="warning" title={newest ? `Version ${newest} is available` : undefined}>
          Update available
        </Badge>
      )}
    </span>
  );
}

function DeviceSummary({ row, now, lastSeenTitle }: { row: DeviceRow; now: Date; lastSeenTitle?: string }) {
  const Icon = (row.platform && platformIcon[row.platform]) || MonitorSmartphone;
  const tone = deviceStatusTone(row);
  return (
    <div className="space-y-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate font-medium">{row.memberName}</p>
          <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
            <Icon aria-hidden className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{row.deviceName ?? (row.deviceId ? "Unnamed device" : "Unknown device")}</span>
          </p>
        </div>
        <span className="text-right text-xs tabular-nums text-muted-foreground">{formatDuration(row.trackedMinutes)}</span>
      </div>
      <p className="text-xs text-muted-foreground">{osText(row)}</p>
      <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
        <span className="flex items-center gap-2" title={lastSeenTitle}>
          <span aria-hidden className={cn("h-2 w-2 rounded-full", toneDot[tone])} />
          {deviceStatusText(row, now)}
        </span>
        <VersionCell row={row} newest={null} />
      </div>
    </div>
  );
}
