import {
  isPresenceOnline,
  platformLabel,
  presenceStatusLabel,
  type AgentPlatform,
  type DeviceReportRowDto,
  type PresenceDto,
  type PresenceStatus,
  type WorkspaceDeviceDto,
} from "@/lib/api-contract";
import { normalizeUtcIso } from "@/lib/live-dataset";
import { agoLabel } from "@/lib/live-session";
import { durationMinutes } from "@/lib/metrics";
import type { Activity, User, UserStatus } from "@/lib/types";

/* ============================================================================
 * DEVICES — every computer that runs a desktop agent for this workspace, from
 * two real sources (contract §3, §6):
 *
 *  - presence (GET /api/app/presence/team): what each agent reported in its
 *    last heartbeat — status, device, OS, agent version — and when;
 *  - the device report (GET /api/app/reporting/devices): the time each device
 *    uploaded over a range, which also finds devices whose agent is too old
 *    to send heartbeats.
 *
 *  - the device registry (GET /api/app/device): every install that sent a
 *    heartbeat, revoked ones included, with who revoked it and why.
 *
 * They are merged per member and device. An agent whose version is older than
 * the newest one seen in the workspace is flagged so it can be updated. Pure
 * functions with the clock as a parameter.
 * ========================================================================== */

export interface DeviceRow {
  /** Stable key: member + device (an agent that reports no device id is the member's "unknown device"). */
  key: string;
  userId: string;
  memberName: string;
  deviceId: string | null;
  deviceName: string | null;
  platform: AgentPlatform | null;
  osVersion: string | null;
  agentVersion: string | null;
  /** The last heartbeat's status; null when the agent never sent one (an older agent). */
  status: PresenceStatus | null;
  /** A heartbeat arrived within the last 150 s. */
  online: boolean;
  /** The project the agent reported in its last heartbeat. */
  projectTitle: string | null;
  /** The latest of the last heartbeat and the last uploaded block (UTC ISO). */
  lastSeenAt: string | null;
  /** Minutes uploaded from this device over the report's range. */
  trackedMinutes: number;
  /** The agent is older than the newest version seen in the workspace. */
  outdated: boolean;
  /** Revoked in this workspace: signed out, and refused until an owner or manager restores it. */
  revoked: boolean;
  revokedAt: string | null;
  revokeReason: string | null;
}

/* ----------------------------- Agent versions ----------------------------- */

interface ParsedVersion {
  parts: number[];
  prerelease: string | null;
}

/** "v0.2.0-beta.1+build.7" → [0,2,0] + "beta.1"; null for anything without a leading number. */
function parseVersion(raw: string | null | undefined): ParsedVersion | null {
  const m = /^\s*v?(\d+(?:\.\d+)*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?\s*$/.exec(raw ?? "");
  if (!m) return null;
  return { parts: m[1].split(".").map(Number), prerelease: m[2] ?? null };
}

/** Whether an agent version can be compared at all. */
export function isComparableVersion(raw: string | null | undefined): boolean {
  return parseVersion(raw) !== null;
}

/**
 * Order two agent versions (semantic-version style): negative when `a` is
 * older. Missing trailing parts count as 0 ("0.2" = "0.2.0"), and a
 * pre-release sorts before its release ("0.2.0-beta" < "0.2.0").
 * Unreadable versions sort before every readable one.
 */
export function compareAgentVersions(a: string | null | undefined, b: string | null | undefined): number {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  if (!pa || !pb) return pa ? 1 : pb ? -1 : 0;
  const length = Math.max(pa.parts.length, pb.parts.length);
  for (let i = 0; i < length; i++) {
    const diff = (pa.parts[i] ?? 0) - (pb.parts[i] ?? 0);
    if (diff !== 0) return diff;
  }
  if (pa.prerelease === pb.prerelease) return 0;
  if (pa.prerelease === null) return 1;
  if (pb.prerelease === null) return -1;
  return pa.prerelease.localeCompare(pb.prerelease, "en", { numeric: true });
}

/** The newest readable version in the list, or null when none can be read. */
export function newestAgentVersion(versions: readonly (string | null | undefined)[]): string | null {
  let newest: string | null = null;
  for (const v of versions) {
    if (!isComparableVersion(v)) continue;
    if (newest === null || compareAgentVersions(v, newest) > 0) newest = (v as string).trim();
  }
  return newest;
}

/* ----------------------------- Merging ----------------------------- */

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

function platformOf(value: unknown): AgentPlatform | null {
  return value === "windows" || value === "macos" || value === "linux" ? value : null;
}

function later(a: string | null, b: string | null): string | null {
  if (!a) return b;
  if (!b) return a;
  return Date.parse(a) >= Date.parse(b) ? a : b;
}

const UNKNOWN_DEVICE = "unknown";

/**
 * One row per member device, merged from the heartbeat list and the device
 * report. `nameOf` turns a member id into the name the page shows (the
 * roster's), given the name the server sent as a hint.
 */
export function buildDeviceRows(
  presence: readonly PresenceDto[],
  report: readonly DeviceReportRowDto[],
  nameOf: (userId: string, hint: string | null) => string,
  now: Date,
  registry: readonly WorkspaceDeviceDto[] = [],
): DeviceRow[] {
  const rows = new Map<string, DeviceRow>();
  const rowFor = (userId: string, deviceId: string | null, hint: string | null): DeviceRow => {
    const key = `${userId}|${deviceId ?? UNKNOWN_DEVICE}`;
    let row = rows.get(key);
    if (!row) {
      row = {
        key,
        userId,
        memberName: nameOf(userId, hint),
        deviceId,
        deviceName: null,
        platform: null,
        osVersion: null,
        agentVersion: null,
        status: null,
        online: false,
        projectTitle: null,
        lastSeenAt: null,
        trackedMinutes: 0,
        outdated: false,
        revoked: false,
        revokedAt: null,
        revokeReason: null,
      };
      rows.set(key, row);
    }
    return row;
  };

  for (const r of report) {
    if (!r || typeof r.userId !== "string") continue;
    const row = rowFor(r.userId, text(r.deviceId), text(r.userName));
    row.deviceName = row.deviceName ?? text(r.deviceName);
    row.platform = row.platform ?? platformOf(r.platform);
    row.osVersion = row.osVersion ?? text(r.osVersion);
    row.agentVersion = row.agentVersion ?? text(r.agentVersion);
    row.lastSeenAt = later(row.lastSeenAt, normalizeUtcIso(r.lastActivityAt));
    const minutes = Number(r.trackedMinutes);
    row.trackedMinutes += Number.isFinite(minutes) && minutes > 0 ? minutes : 0;
  }

  // The heartbeat is the fresher source: it wins for what the device is and runs now.
  for (const p of presence) {
    if (!p || typeof p.userId !== "string" || !text(p.deviceId)) continue;
    const row = rowFor(p.userId, text(p.deviceId), text(p.name) ?? text(p.userName));
    const seen = normalizeUtcIso(p.lastSeenAt);
    row.deviceName = text(p.deviceName) ?? row.deviceName;
    row.platform = platformOf(p.platform) ?? row.platform;
    row.osVersion = text(p.osVersion) ?? row.osVersion;
    row.agentVersion = text(p.agentVersion) ?? row.agentVersion;
    row.status = p.status;
    row.projectTitle = text(p.projectTitle);
    // Age the server's verdict with the page's clock, so a list kept open goes offline on time.
    row.online = p.online !== false && isPresenceOnline(seen, now);
    row.lastSeenAt = later(row.lastSeenAt, seen);
  }

  // The registry says which installs are revoked (and keeps one whose heartbeats aged out of presence).
  for (const d of registry) {
    if (!d || typeof d.userId !== "string" || !text(d.deviceId)) continue;
    const row = rowFor(d.userId, text(d.deviceId), text(d.name) ?? text(d.userName));
    row.deviceName = row.deviceName ?? text(d.deviceName);
    row.platform = row.platform ?? platformOf(d.platform);
    row.osVersion = row.osVersion ?? text(d.osVersion);
    row.agentVersion = row.agentVersion ?? text(d.agentVersion);
    row.lastSeenAt = later(row.lastSeenAt, normalizeUtcIso(d.lastSeenAt));
    row.revoked = d.isRevoked === true;
    row.revokedAt = row.revoked ? normalizeUtcIso(d.revokedAt) : null;
    row.revokeReason = row.revoked ? text(d.reason) : null;
    // A revoked install is refused: whatever its last heartbeat said, it is not working here now.
    if (row.revoked) row.online = false;
  }

  const newest = newestAgentVersion([...rows.values()].map((r) => r.agentVersion));
  const list = [...rows.values()].map((r) => ({
    ...r,
    trackedMinutes: Math.round(r.trackedMinutes),
    outdated: newest !== null && isComparableVersion(r.agentVersion) && compareAgentVersions(r.agentVersion, newest) < 0,
  }));
  return list.sort(
    (a, b) =>
      Number(b.online) - Number(a.online) ||
      (b.lastSeenAt ? Date.parse(b.lastSeenAt) : 0) - (a.lastSeenAt ? Date.parse(a.lastSeenAt) : 0) ||
      a.memberName.localeCompare(b.memberName),
  );
}

/** "Windows 11 Pro 23H2" — the OS as reported, else the platform's name, else "Unknown OS". */
export function osText(row: Pick<DeviceRow, "platform" | "osVersion">): string {
  return row.osVersion ?? platformLabel(row.platform) ?? "Unknown OS";
}

/**
 * The status cell: the heartbeat status while online ("Tracking", "On a
 * break"…), "Offline · last seen 3h ago" otherwise, or — for an agent that
 * never sent a heartbeat — when it last uploaded.
 */
export function deviceStatusText(row: Pick<DeviceRow, "status" | "online" | "lastSeenAt"> & { revoked?: boolean }, now: Date): string {
  if (row.revoked) return "Revoked · signed out";
  if (row.status !== null && row.online) return presenceStatusLabel(row.status);
  const seen = row.lastSeenAt ? agoLabel(row.lastSeenAt, now) : null;
  if (row.status === null) return seen ? `No live status · last upload ${seen}` : "No live status";
  return seen ? `Offline · last seen ${seen}` : "Offline";
}

/** The tone of the status cell's dot. */
export function deviceStatusTone(row: Pick<DeviceRow, "status" | "online"> & { revoked?: boolean }): "tracking" | "away" | "offline" {
  if (row.revoked || !row.online || row.status === null) return "offline";
  return row.status === "tracking" ? "tracking" : "away";
}

/* ----------------------------- Member presence ----------------------------- */

/** Statuses of a running, signed-in agent that is not tracking right now. */
const AWAY: ReadonlySet<PresenceStatus> = new Set<PresenceStatus>(["paused", "break", "idle", "locked", "choosing"]);

/**
 * Each member's status from their devices' heartbeats: "active" when any
 * device is tracking, "idle" when one is online but paused, on a break, idle,
 * locked or choosing a project, otherwise "offline". Members without any
 * heartbeat are absent from the map.
 */
export function memberPresence(presence: readonly PresenceDto[], now: Date): Map<string, UserStatus> {
  const rank: Record<UserStatus, number> = { offline: 0, idle: 1, active: 2 };
  const out = new Map<string, UserStatus>();
  for (const p of presence) {
    if (!p || typeof p.userId !== "string") continue;
    const online = p.online !== false && isPresenceOnline(normalizeUtcIso(p.lastSeenAt), now);
    const status: UserStatus = !online ? "offline" : p.status === "tracking" ? "active" : AWAY.has(p.status) ? "idle" : "offline";
    const current = out.get(p.userId);
    if (current === undefined || rank[status] > rank[current]) out.set(p.userId, status);
  }
  return out;
}

/* ----------------------------- Demo workspace ----------------------------- */

const DEMO_AGENT_VERSIONS = ["0.2.0", "0.2.0", "0.1.9"] as const;
const DEMO_STATUS: Record<UserStatus, PresenceStatus> = { active: "tracking", idle: "idle", offline: "stopped" };

function hash(value: string): number {
  let h = 0;
  for (let i = 0; i < value.length; i++) h = (h * 31 + value.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/**
 * The demo workspace's devices: one agent per tracked demo member, derived
 * deterministically from the member and their demo activity (status from the
 * member's status, last seen from their latest block, minutes from the last
 * 30 days). Demo mode only — a live workspace reads the real endpoints.
 */
export function demoDeviceSources(
  users: readonly User[],
  activities: readonly Activity[],
  now: Date,
): { presence: PresenceDto[]; report: DeviceReportRowDto[] } {
  const since = now.getTime() - 30 * 86_400_000;
  const presence: PresenceDto[] = [];
  const report: DeviceReportRowDto[] = [];
  for (const u of users) {
    if (u.role !== "owner" && u.role !== "admin" && u.role !== "worker") continue;
    const h = hash(u.id);
    const mac = h % 3 === 0;
    const first = u.name.split(" ")[0] || "Team";
    const deviceId = `demo-${u.id}`;
    const deviceName = mac ? `${first}'s MacBook Pro` : `${first.toUpperCase()}-PC`;
    const osVersion = mac ? "macOS 14.5 (23F79)" : "Windows 11 Pro 23H2 (10.0.22631)";
    const agentVersion = DEMO_AGENT_VERSIONS[h % DEMO_AGENT_VERSIONS.length];
    const mine = activities.filter((a) => a.userId === u.id);
    const recent = mine.filter((a) => Date.parse(a.endedAt) >= since && Date.parse(a.startedAt) <= now.getTime());
    const lastEnd = mine.reduce((max, a) => Math.max(max, Date.parse(a.endedAt)), 0);
    const lastSeen =
      u.status === "offline" ? (lastEnd > 0 ? new Date(Math.min(lastEnd, now.getTime())) : null) : new Date(now.getTime() - 30_000);
    if (recent.length > 0) {
      report.push({
        userId: u.id,
        userName: u.name,
        deviceId,
        deviceName,
        platform: mac ? "macos" : "windows",
        osVersion,
        agentVersion,
        lastActivityAt: new Date(Math.min(lastEnd, now.getTime())).toISOString(),
        trackedMinutes: Math.round(recent.reduce((s, a) => s + durationMinutes(a), 0)),
      });
    }
    if (lastSeen) {
      presence.push({
        userId: u.id,
        userName: u.name,
        name: u.name,
        deviceId,
        deviceName,
        platform: mac ? "macos" : "windows",
        osVersion,
        agentVersion,
        status: DEMO_STATUS[u.status],
        lastSeenAt: lastSeen.toISOString(),
        online: u.status !== "offline",
      });
    }
  }
  return { presence, report };
}

/* ----------------------------- Revocation ----------------------------- */

/**
 * Whether the viewer may revoke or restore a member's device (E2E-09): owners
 * and managers (Tracker.Devices.Manage) for anyone but an owner, owners for
 * owners too. A member revokes only their own (and never restores).
 */
export function deviceActions(input: {
  canManageDevices: boolean;
  viewerIsOwner: boolean;
  viewerId: string;
  rowUserId: string;
  rowUserIsOwner: boolean;
  revoked: boolean;
  /** The install has an id the server knows (an agent too old to send one can't be revoked). */
  hasDeviceId: boolean;
}): { revoke: boolean; restore: boolean } {
  if (!input.hasDeviceId) return { revoke: false, restore: false };
  const manages = input.canManageDevices && (input.viewerIsOwner || !input.rowUserIsOwner);
  const own = input.rowUserId === input.viewerId;
  return { revoke: !input.revoked && (manages || own), restore: input.revoked && manages };
}
