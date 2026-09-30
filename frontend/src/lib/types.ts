export type Role = "host" | "admin" | "owner" | "worker" | "client";

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
}

/** "thumb" is a small JPEG rendition of the "screen" capture, meant for list tiles. */
export type CaptureKind = "screen" | "webcam" | "thumb";

/** A stored capture of a block; its bytes come from the authed content endpoint. */
export interface CaptureRef {
  id: string;
  kind: CaptureKind;
}

export interface Activity {
  id: string;
  userId: string;
  projectId: string;
  startedAt: string;
  endedAt: string;
  description: string;
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
}

export interface ScreenMock {
  app: string;
  kind: "editor" | "browser" | "design" | "chat" | "terminal" | "docs";
  accent: string;
}
