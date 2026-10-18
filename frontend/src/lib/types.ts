export type Role = "host" | "admin" | "owner" | "worker" | "client";

/**
 * The operating system of the agent that recorded a block — the same values
 * as the wire's `platform` (api-contract AgentPlatform). Declared here rather
 * than imported so this leaf module never depends on the API layer.
 */
export type DevicePlatform = "windows" | "macos" | "linux";

/** Why a block ended — the same values as the wire's `endReason` (api-contract ActivityEndReason). */
export type BlockEndReason =
  | "interval"
  | "pause"
  | "break"
  | "idle"
  | "lock"
  | "sleep"
  | "project"
  | "task"
  | "signout"
  | "quit"
  | "shutdown";

export type UserStatus = "active" | "idle" | "offline";

export interface User {
  id: string;
  name: string;
  email: string;
  role: Role;
  designation: string;
  status: UserStatus;
  timezone: string;
  /** minutes tracked today */
  trackedToday: number;
  /** 0-100 */
  productivity: number;
  joinedAt: string;
}

export interface TrackingPermissions {
  screenshot: boolean;
  webcam: boolean;
  keyboard: boolean;
  mouse: boolean;
  activeWindow: boolean;
  runningPrograms: boolean;
}

export interface Project {
  id: string;
  title: string;
  description: string;
  color: string;
  archived: boolean;
  intervalMinutes: number;
  permissions: TrackingPermissions;
  memberIds: string[];
  createdAt: string;
  /** minutes */
  loggedThisWeek: number;
  loggedThisMonth: number;
  loggedTotal: number;
  /** Minutes without input before the agents stop tracking; 0 = off (live projects; absent = the default, 5). */
  idleThresholdMinutes?: number;
  /** The agents blur screen captures before upload (live projects; absent = no). */
  blurScreenshots?: boolean;
  /** When the project was archived (live projects). */
  archivedAt?: string;
}

export interface WindowInfo {
  appName: string;
  windowTitle: string;
  /** seconds spent focused in the block */
  seconds: number;
  /** keyboard presses while this window was focused (newer agents only; absent = not reported) */
  keyboardHits?: number;
  /** mouse clicks while this window was focused (newer agents only; absent = not reported) */
  mouseClicks?: number;
  /**
   * True when `seconds` was not measured: a legacy row reported no per-window
   * time, so its focused window is credited with the whole block.
   */
  credited?: boolean;
}

/** One minute of a block's input timeline (newer agents only). */
export interface ActivityMinute {
  /** 0-based bucket index from `startedAt` (the last bucket may run slightly long) */
  minute: number;
  keyboardHits: number;
  mouseClicks: number;
  /** false when the member was idle for the whole minute */
  active: boolean;
  /** the app focused for most of the minute, when known */
  appName?: string;
  /** seconds of the bucket with any input, 0–90 (newer agents only; absent = not reported) */
  activeSeconds?: number;
}

/** "thumb" is a small JPEG rendition of the "screen" capture, meant for list tiles. */
export type CaptureKind = "screen" | "webcam" | "thumb";

/** A stored capture of a block; its bytes come from the authed content endpoint. */
export interface CaptureRef {
  id: string;
  kind: CaptureKind;
  /** When the capture was taken (newer servers; absent on older ones). */
  capturedAt?: string;
  /** The agent blurred it before upload (absent = not blurred, or an older server). */
  blurred?: boolean;
}

export interface Activity {
  id: string;
  userId: string;
  projectId: string;
  startedAt: string;
  endedAt: string;
  /** The block's headline: its task note, or a neutral default when it has none (see `note`). */
  description: string;
  /**
   * Activity % of the block, 0-100 (see metrics.ts: ACTIVITY_METRIC). 0 when
   * `activityMeasured` is false — read it through `activityPercentOf()` so an
   * unmeasured block shows "Not measured" instead of a false 0%.
   */
  productivity: number;
  mouseClicks: number;
  keyboardHits: number;
  activeWindows: WindowInfo[];
  runningPrograms: WindowInfo[];
  /** synthetic screenshot descriptor rendered client-side */
  screen: ScreenMock;
  hasWebcam: boolean;
  online: boolean;
  /** per-minute input timeline (live rows from newer agents; absent or empty otherwise) */
  timeline?: ActivityMinute[];
  /** the block's stored captures (live rows only; absent in demo data and on older backends) */
  captures?: CaptureRef[];

  /* ---- Agent data v2 (contract §5). Every field is optional: older agents and servers send none. ---- */

  /**
   * False when the block's keyboard/mouse input was not measured (capture
   * unavailable or denied, or the project allows neither). Absent = measured.
   */
  activityMeasured?: boolean;
  /** The member's own task note for the block, exactly as stored; null/absent when there is none. */
  note?: string | null;
  /** Seconds of the block without keyboard or mouse input (absent = not reported; see idleSecondsOf). */
  idleSeconds?: number;
  /** Why the block ended. */
  endReason?: BlockEndReason;
  /** The device that recorded the block. */
  deviceId?: string;
  deviceName?: string;
  platform?: DevicePlatform;
  /** e.g. "Windows 11 Pro 23H2 (10.0.22631)", "macOS 14.5 (23F79)" */
  osVersion?: string;
  agentVersion?: string;
  /** The device's IANA time zone and its UTC offset at `startedAt` (+360 for Dhaka). */
  timeZone?: string;
  utcOffsetMinutes?: number;
  /** Server time minus the device clock at upload, in seconds (positive = the device clock was behind). */
  clockSkewSeconds?: number;
  /** Software-injected input seen during the block (excluded from the counts and Activity %). */
  injectedInputEvents?: number;
}

export interface ScreenMock {
  app: string;
  kind: "editor" | "browser" | "design" | "chat" | "terminal" | "docs";
  accent: string;
}

/**
 * How complete a live session's activity rows are. The session pages in the
 * newest rows up to a bound; when more exist, `truncated` is set and the rows
 * are only complete from `completeFrom` on — a page that totals or lists rows
 * from before that instant must say its figures are partial.
 */
export interface ActivityCoverage {
  /** The window the session loads, as ISO instants: blocks that started in [from, to). */
  from: string;
  to: string;
  /** Rows held. */
  loaded: number;
  /** Rows the server holds in the window (never less than `loaded`). */
  total: number;
  /** More rows exist than are held: lists and row-derived charts are partial. */
  truncated: boolean;
  /** The held rows are complete from this instant on (the window's start when nothing was cut off). */
  completeFrom: string;
}
