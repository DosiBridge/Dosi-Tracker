import { NOW } from "./mock-data";
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

export function filterActivitiesByRange(list: Activity[], from: Date, to: Date) {
  return list.filter((a) => {
    const t = new Date(a.endedAt).getTime();
    return t >= from.getTime() && t <= to.getTime();
  });
}

/* ----------------------------- Report catalog ----------------------------- */

export interface ReportMeta {
  slug: string;
  title: string;
  description: string;
  category: "Time" | "Productivity" | "People" | "Finance";
  icon: string; // lucide icon name resolved in UI
}

export const reportCatalog: ReportMeta[] = [
  { slug: "time-activity", title: "Time & Activity", description: "Time worked, activity level, and screenshots per member.", category: "Time", icon: "Clock" },
  { slug: "productivity", title: "Productivity", description: "Productive vs neutral vs unproductive time breakdown.", category: "Productivity", icon: "Gauge" },
  { slug: "apps-urls", title: "Apps & Websites", description: "Most-used applications and websites, categorized.", category: "Productivity", icon: "AppWindow" },
  { slug: "projects", title: "Project Breakdown", description: "Time and cost distribution across projects.", category: "Time", icon: "FolderKanban" },
  { slug: "attendance", title: "Attendance & Shifts", description: "Clock in/out, late arrivals, and absences.", category: "People", icon: "CalendarCheck" },
  { slug: "weekly", title: "Weekly Summary", description: "A rolled-up weekly digest per member.", category: "Time", icon: "CalendarDays" },
  { slug: "payroll", title: "Payroll & Billing", description: "Billable hours, rates, and payable amounts.", category: "Finance", icon: "DollarSign" },
];
