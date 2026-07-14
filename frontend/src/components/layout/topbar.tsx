"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Bell,
  Menu,
  Search,
  LogOut,
  ChevronRight,
  FileBarChart,
  AlertTriangle,
  AtSign,
  UserPlus,
  Settings as SettingsIcon,
  CheckCheck,
  Crown,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { useSession } from "@/components/session-provider";
import { roleLabels } from "@/lib/roles";
import { planById, statusLabel } from "@/lib/saas-data";
import { notifications as seedNotifications } from "@/lib/tenant-data";
import { type NotificationType } from "@/lib/reports-data";
import { cn, timeAgo } from "@/lib/utils";

const titles: Record<string, string> = {
  dashboard: "Dashboard",
  projects: "Projects",
  team: "Team",
  activities: "Activities",
  screenshots: "Screenshots",
  timesheet: "Timesheet",
  reports: "Reports",
  insights: "Insights",
  settings: "Settings",
};

const notifIcon: Record<NotificationType, LucideIcon> = {
  report: FileBarChart,
  alert: AlertTriangle,
  mention: AtSign,
  member: UserPlus,
  system: SettingsIcon,
};

const notifTone: Record<NotificationType, string> = {
  report: "#6d5efc",
  alert: "#ef4444",
  mention: "#0ea5e9",
  member: "#22c55e",
  system: "#94a3b8",
};

function openPalette() {
  window.dispatchEvent(new Event("open-command-palette"));
}

export function Topbar({ onMenuClick }: { onMenuClick: () => void }) {
  const pathname = usePathname();
  const { user: currentUser, logout, workspace } = useSession();
  const plan = planById(workspace.planId);
  const [menuOpen, setMenuOpen] = useState(false);
  const [notifOpen, setNotifOpen] = useState(false);
  const [notifs, setNotifs] = useState(seedNotifications);

  // Reload notifications for the active tenant when the workspace changes.
  useEffect(() => {
    setNotifs(seedNotifications);
    setNotifOpen(false);
  }, [workspace.id]);
  const segments = pathname.split("/").filter(Boolean);
  const section = segments[0] ?? "dashboard";
  const unread = notifs.filter((n) => !n.read).length;

  return (
    <header className="sticky top-0 z-30 flex h-16 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur-xl lg:px-6">
      <button
        onClick={onMenuClick}
        className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted lg:hidden"
        aria-label="Open menu"
      >
        <Menu className="h-5 w-5" />
      </button>

      {/* Breadcrumb */}
      <div className="hidden items-center gap-1.5 text-sm sm:flex">
        <span className="font-semibold">{titles[section] ?? "Dosi"}</span>
        {segments[1] && (
          <>
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
            <span className="capitalize text-muted-foreground">{titles[segments[1]] ?? "Details"}</span>
          </>
        )}
      </div>

      {/* Search → opens command palette */}
      <div className="relative ml-auto hidden max-w-sm flex-1 md:block">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <button
          onClick={openPalette}
          className="flex h-9 w-full items-center rounded-lg border border-border bg-card pl-9 pr-16 text-left text-sm text-muted-foreground hover:bg-muted"
        >
          Search or jump to…
        </button>
        <kbd className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-border bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
          ⌘K
        </kbd>
      </div>

      <div className="ml-auto flex items-center gap-2 md:ml-0">
        <button
          onClick={openPalette}
          className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-card text-muted-foreground hover:bg-muted hover:text-foreground md:hidden"
          aria-label="Search"
        >
          <Search className="h-[18px] w-[18px]" />
        </button>

        {/* Plan badge (multi-tenant / SaaS) */}
        {currentUser.role === "owner" ? (
          <Link
            href="/billing"
            className={cn(
              "hidden h-9 items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-colors sm:inline-flex",
              workspace.status === "trialing"
                ? "border-warning/40 bg-warning/10 text-warning"
                : "border-border bg-card text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
            title="Billing & plan"
          >
            <Crown className="h-3.5 w-3.5" />
            {plan.name}
            {workspace.status === "trialing" && <span>· {statusLabel[workspace.status]}</span>}
          </Link>
        ) : (
          <span className="hidden h-9 items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 text-xs font-medium text-muted-foreground sm:inline-flex">
            <Crown className="h-3.5 w-3.5" /> {plan.name}
          </span>
        )}

        <ThemeToggle />

        {/* Notifications */}
        <div className="relative">
          <button
            onClick={() => setNotifOpen((o) => !o)}
            className="relative inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-card text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label="Notifications"
          >
            <Bell className="h-[18px] w-[18px]" />
            {unread > 0 && (
              <span className="absolute right-1.5 top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-danger px-1 text-[9px] font-bold text-white ring-2 ring-card">
                {unread}
              </span>
            )}
          </button>
          {notifOpen && (
            <>
              <div className="fixed inset-0 z-10" onClick={() => setNotifOpen(false)} />
              <div className="absolute right-0 top-full z-20 mt-2 w-[calc(100vw-1rem)] max-w-sm overflow-hidden rounded-xl border border-border bg-card card-elev-lg animate-scale-in sm:w-80">
                <div className="flex items-center justify-between border-b border-border p-3">
                  <div className="text-sm font-semibold">Notifications</div>
                  {unread > 0 && (
                    <button
                      onClick={() => setNotifs((prev) => prev.map((n) => ({ ...n, read: true })))}
                      className="flex items-center gap-1 text-xs text-primary hover:underline"
                    >
                      <CheckCheck className="h-3.5 w-3.5" /> Mark all read
                    </button>
                  )}
                </div>
                <div className="max-h-96 overflow-y-auto">
                  {notifs.map((n) => {
                    const Icon = notifIcon[n.type];
                    return (
                      <button
                        key={n.id}
                        onClick={() => setNotifs((prev) => prev.map((x) => (x.id === n.id ? { ...x, read: true } : x)))}
                        className={cn(
                          "flex w-full items-start gap-3 border-b border-border/60 p-3 text-left last:border-0 hover:bg-muted/60",
                          !n.read && "bg-accent/40"
                        )}
                      >
                        <span
                          className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg"
                          style={{ background: `${notifTone[n.type]}1a`, color: notifTone[n.type] }}
                        >
                          <Icon className="h-4 w-4" />
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="flex items-center justify-between gap-2">
                            <span className="truncate text-sm font-medium">{n.title}</span>
                            {!n.read && <span className="h-2 w-2 shrink-0 rounded-full bg-primary" />}
                          </span>
                          <span className="mt-0.5 block text-xs text-muted-foreground">{n.body}</span>
                          <span className="mt-1 block text-[11px] text-muted-foreground/70">{timeAgo(new Date(n.at))}</span>
                        </span>
                      </button>
                    );
                  })}
                </div>
                <Link
                  href="/insights"
                  onClick={() => setNotifOpen(false)}
                  className="block border-t border-border p-2.5 text-center text-sm text-primary hover:bg-muted"
                >
                  View all insights
                </Link>
              </div>
            </>
          )}
        </div>

        {/* User menu */}
        <div className="relative">
          <button
            onClick={() => setMenuOpen((o) => !o)}
            className="flex items-center gap-2 rounded-lg p-1 pr-2 transition-colors hover:bg-muted"
          >
            <Avatar name={currentUser.name} size="sm" status={currentUser.status} />
          </button>
          {menuOpen && (
            <>
              <div className="fixed inset-0 z-10" onClick={() => setMenuOpen(false)} />
              <div className="absolute right-0 top-full z-20 mt-2 w-60 overflow-hidden rounded-xl border border-border bg-card card-elev-lg animate-scale-in">
                <div className="flex items-center gap-3 border-b border-border p-3">
                  <Avatar name={currentUser.name} size="md" />
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">{currentUser.name}</div>
                    <div className="truncate text-xs text-muted-foreground">{currentUser.email}</div>
                  </div>
                </div>
                <div className="p-2">
                  <Badge tone="primary">{roleLabels[currentUser.role]}</Badge>
                </div>
                <div className="border-t border-border p-1">
                  <Link
                    href="/settings"
                    onClick={() => setMenuOpen(false)}
                    className="block rounded-lg px-3 py-2 text-sm hover:bg-muted"
                  >
                    Profile & Settings
                  </Link>
                  <Link
                    href="/login"
                    onClick={() => { setMenuOpen(false); logout(); }}
                    className={cn(
                      "flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-danger hover:bg-danger/10"
                    )}
                  >
                    <LogOut className="h-4 w-4" /> Sign out
                  </Link>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </header>
  );
}
