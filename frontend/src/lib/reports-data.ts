import { NOW } from "./demo-clock";
import { inRange } from "./metrics";
import type { Activity } from "./types";

/* ============================================================================
 * Pure, tenant-INDEPENDENT report primitives: shared types, category colors,
 * date-range helpers, and the report catalog. All tenant-scoped DATA (app
 * usage, productivity split, attendance, billing, notifications, insights)
 * now lives in tenant-data.ts so it can switch per workspace.
 * ========================================================================== */

/* ----------------------------- Types ----------------------------- */

export type AppCategory = "productive" | "neutral" | "unproductive";

export interface AppUsage {
  app: string;
  category: AppCategory;
  minutes: number;
  color: string;
  activeUsers: number;
}

export interface ProductivitySplit {
  userId: string;
  productive: number;
  neutral: number;
  unproductive: number;
  idle: number;
}

export type AttendanceStatus = "present" | "late" | "absent" | "remote";

export interface AttendanceRecord {
  userId: string;
  date: string; // ISO date
  status: AttendanceStatus;
  clockIn: string | null;
  clockOut: string | null;
  worked: number; // minutes
}

export interface BillingRow {
  userId: string;
  rate: number; // per hour, USD
  billable: boolean;
}

export type NotificationType = "report" | "alert" | "mention" | "member" | "system";

export interface AppNotification {
  id: string;
  type: NotificationType;
  title: string;
  body: string;
  at: string; // ISO
  read: boolean;
}

export interface Insight {
  id: string;
  tone: "success" | "warning" | "danger" | "info";
  title: string;
  detail: string;
}

/* ----------------------------- Category colors ----------------------------- */

export const categoryColor: Record<AppCategory, string> = {
  productive: "#22c55e",
  neutral: "#0ea5e9",
  unproductive: "#ef4444",
};

/* ----------------------------- Date ranges ----------------------------- */
// The Activity page's date presets (its "custom" is a real from/to pair, see
// activity-filters.ts). The reports use the period toolbar instead: whole
// days of the reporting zone (report-math resolveReportPeriod).

export type RangeKey = "today" | "yesterday" | "7d" | "30d" | "month" | "custom";

export const rangePresets: { key: RangeKey; label: string }[] = [
  { key: "today", label: "Today" },
  { key: "yesterday", label: "Yesterday" },
  { key: "7d", label: "Last 7 days" },
  { key: "30d", label: "Last 30 days" },
  { key: "month", label: "This month" },
  { key: "custom", label: "Custom" },
];

export function rangeForKey(key: RangeKey): { from: Date; to: Date; label: string } {
  const to = new Date(NOW);
  const from = new Date(NOW);
  switch (key) {
    case "today":
      from.setHours(0, 0, 0, 0);
      return { from, to, label: "Today" };
    case "yesterday":
      from.setDate(from.getDate() - 1);
      from.setHours(0, 0, 0, 0);
      to.setDate(to.getDate() - 1);
      to.setHours(23, 59, 59, 999);
      return { from, to, label: "Yesterday" };
    case "7d":
      from.setDate(from.getDate() - 7);
      return { from, to, label: "Last 7 days" };
    case "30d":
      from.setDate(from.getDate() - 30);
      return { from, to, label: "Last 30 days" };
    case "month":
      from.setDate(1);
      from.setHours(0, 0, 0, 0);
      return { from, to, label: "This month" };
    default:
      from.setDate(from.getDate() - 7);
      return { from, to, label: "Custom" };
  }
}

/** Rows landing inside [from, to]. Membership is metrics.ts's definition — one rule, everywhere. */
export function filterActivitiesByRange(list: Activity[], from: Date, to: Date) {
  return list.filter((a) => inRange(a, from, to));
}

/* ----------------------------- Report catalog ----------------------------- */

/** A permission a report needs beyond opening /reports (contract §6); the hub only lists what the viewer can use. */
export type ReportRequirement = "manageTeam";

export interface ReportMeta {
  slug: string;
  title: string;
  description: string;
  category: "Time" | "Productivity" | "People" | "Finance";
  icon: string; // lucide icon name resolved in UI
  /** Shown as the hub's hero card instead of a tile in the catalog grid. */
  featured?: boolean;
  /** Shown as the hub's analytics spotlight (below the hero) instead of a tile. */
  spotlight?: boolean;
  /** Pay and labour cost read hourly rates: only for Tracker.Team.Manage holders. */
  requires?: ReportRequirement;
}

/**
 * Wording follows the reports' one glossary: "activity" and "active time" are
 * keyboard/mouse input, "idle" the minutes without it, and "productive" an
 * app category — never an activity score.
 */
export const reportCatalog: ReportMeta[] = [
  {
    slug: "work-report",
    title: "Work Report",
    description: "A client-ready report of hours worked per day — at a glance, printable and ready to send.",
    category: "Time",
    icon: "CalendarClock",
    featured: true,
  },
  {
    slug: "analytics",
    title: "Work Analytics",
    description: "Trends, work rhythm, focus, projects and people — explored in interactive charts and diagrams.",
    category: "Productivity",
    icon: "Sparkles",
    spotlight: true,
  },
  {
    slug: "time-activity",
    title: "Time & Activity",
    description: "Hours, activity (keyboard and mouse) and idle time per member, day by day.",
    category: "Time",
    icon: "Clock",
  },
  {
    slug: "weekly",
    title: "Weekly Summary",
    description: "The week at a glance per member: hours, activity, idle time, days worked and the busiest day.",
    category: "Time",
    icon: "CalendarDays",
  },
  {
    slug: "projects",
    title: "Project Breakdown",
    description: "Time, share and labour cost per project, archived projects included.",
    category: "Time",
    icon: "FolderKanban",
  },
  {
    slug: "productivity",
    title: "Productivity",
    description: "Active and idle time per member, and time spent in productive, neutral and unproductive apps.",
    category: "Productivity",
    icon: "Gauge",
  },
  {
    slug: "apps-urls",
    title: "Apps & Websites",
    description: "The applications your team works in, by time, by member and by category.",
    category: "Productivity",
    icon: "AppWindow",
  },
  {
    slug: "attendance",
    title: "Attendance",
    description: "Days present and absent, typical start and finish times, and weekend work per member.",
    category: "People",
    icon: "CalendarCheck",
  },
  {
    slug: "devices",
    title: "Devices & Agents",
    description: "Every computer running the desktop agent: platform, agent version, live status and time tracked.",
    category: "People",
    icon: "MonitorSmartphone",
  },
  {
    slug: "payroll",
    title: "Payroll & Billing",
    description: "Pay per member and project, each hour priced at the hourly rate in effect when it was worked.",
    category: "Finance",
    icon: "DollarSign",
    requires: "manageTeam",
  },
];
