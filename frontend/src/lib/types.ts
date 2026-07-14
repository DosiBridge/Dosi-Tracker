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
}

export interface ScreenMock {
  app: string;
  kind: "editor" | "browser" | "design" | "chat" | "terminal" | "docs";
  accent: string;
}
