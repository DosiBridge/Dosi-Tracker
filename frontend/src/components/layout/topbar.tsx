"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Bell,
  Menu,
  Search,
  LogOut,
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
import { planNameFor, statusLabel } from "@/lib/saas-data";
import { notifications as seedNotifications } from "@/lib/tenant-data";
import { type AppNotification, type NotificationType } from "@/lib/reports-data";
import { mapApiNotifications } from "@/lib/live-dataset";
import { getApi, postApi } from "@/hooks/useApi";
import { cn, timeAgo } from "@/lib/utils";

const titles: Record<string, string> = {
  dashboard: "Dashboard",
  projects: "Projects",
  team: "Team",
  activities: "Activity",
  screenshots: "Activity",
  timesheet: "Timesheet",
  reports: "Reports",
  insights: "Dashboard",
  settings: "Settings",
  billing: "Billing",
  monitor: "Member Monitor",
  download: "Desktop agent",
};

const notifIcon: Record<NotificationType, LucideIcon> = {
  report: FileBarChart,
  alert: AlertTriangle,
  mention: AtSign,
  member: UserPlus,
  system: SettingsIcon,
};

const notifTone: Record<NotificationType, string> = {
  report: "#0d9488",
  alert: "#dc2626",
  mention: "#0284c7",
  member: "#16a34a",
  system: "#64748b",
};

const iconBtn =
  "inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function openPalette() {
  window.dispatchEvent(new Event("open-command-palette"));
}

export function Topbar({ onMenuClick }: { onMenuClick: () => void }) {
  const pathname = usePathname();
  const { user: currentUser, logout, workspace, isLive, status } = useSession();
  // Load live notifications once the real identity is in (not for the placeholder session).
  const liveReady = isLive && status === "ready";
  const planName = planNameFor(workspace);
  const [menuOpen, setMenuOpen] = useState(false);
  const [notifOpen, setNotifOpen] = useState(false);
  // Demo: the seeded notifications. Live: the user's REAL notifications from
  // the backend — never the demo ones (which name demo people).
  const [notifs, setNotifs] = useState<AppNotification[]>(isLive ? [] : seedNotifications);
  const [notifLoading, setNotifLoading] = useState(false);
  const [notifError, setNotifError] = useState(false);

  const loadNotifications = useCallback(async () => {
    setNotifLoading(true);
    setNotifError(false);
    try {
      setNotifs(mapApiNotifications(await getApi("/api/app/notification/my-notifications")));
    } catch {
      setNotifs([]);
      setNotifError(true);
    } finally {
      setNotifLoading(false);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- resets the feed when the workspace/session changes
    setNotifOpen(false);
    if (liveReady) {
      void loadNotifications();
    } else if (isLive) {
      setNotifs([]);
    } else {
      setNotifs(seedNotifications);
    }
  }, [workspace.id, isLive, liveReady, loadNotifications]);

  /** Mark read locally at once; persist to the backend in a live session. */
  function markRead(ids: string[]) {
    if (ids.length === 0) return;
    setNotifs((prev) => prev.map((n) => (ids.includes(n.id) ? { ...n, read: true } : n)));
    if (isLive) {
      void Promise.allSettled(ids.map((id) => postApi(`/api/app/notification/mark-as-read/${encodeURIComponent(id)}`, {})));
    } else {
      seedNotifications.forEach((n) => {
        if (ids.includes(n.id)) n.read = true;
      });
    }
  }

  const segments = pathname.split("/").filter(Boolean);
  const section = segments[0] ?? "dashboard";
  const unread = notifs.filter((n) => !n.read).length;

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-3 border-b border-border/80 bg-background/85 px-4 backdrop-blur-xl lg:px-6">
      <button
        onClick={onMenuClick}
        className={cn(iconBtn, "border border-border lg:hidden")}
        aria-label="Open menu"
      >
        <Menu className="h-4 w-4" />
      </button>

      {/* Mobile page context only — desktop uses PageHeader */}
      <div className="min-w-0 flex-1 sm:hidden">
        <div className="truncate text-sm font-semibold">{titles[section] ?? "Dosi"}</div>
      </div>

      {/* Command search — grows, actions stay flush right */}
      <button
        onClick={openPalette}
        className="relative hidden h-9 min-w-0 max-w-md flex-1 items-center rounded-xl border border-border/80 bg-muted/40 pl-9 pr-14 text-left text-sm text-muted-foreground transition-colors hover:border-border hover:bg-muted/70 sm:flex"
      >
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        Search or jump to…
        <kbd className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded-md border border-border bg-background px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
          ⌘K
        </kbd>
      </button>

      <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
        <button
          onClick={openPalette}
          className={cn(iconBtn, "border border-border sm:hidden")}
          aria-label="Search"
        >
          <Search className="h-4 w-4" />
        </button>

        {/* Plan — quiet text chip, same height as icon tray */}
        {currentUser.role === "owner" ? (
          <Link
            href="/billing"
            className={cn(
              "hidden h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-medium transition-colors sm:inline-flex",
              workspace.status === "trialing"
                ? "bg-warning/10 text-warning hover:bg-warning/15"
                : "text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
            title="Billing & plan"
          >
            <Crown className="h-3.5 w-3.5 opacity-70" />
            <span>{planName}</span>
            {workspace.status === "trialing" && (
              <span className="text-[10px] opacity-80">· {statusLabel[workspace.status]}</span>
            )}
          </Link>
        ) : (
          <span className="hidden h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted-foreground sm:inline-flex">
            <Crown className="h-3.5 w-3.5 opacity-70" /> {planName}
          </span>
        )}

        {/* Unified icon tray — one surface, equal targets */}
        <div className="flex h-9 items-center gap-0.5 rounded-xl border border-border/80 bg-card px-0.5 shadow-[0_1px_0_rgba(12,21,36,0.04)]">
          <ThemeToggle className="h-8 w-8 rounded-lg border-0 bg-transparent shadow-none" />

          <div className="relative">
            <button
              onClick={() => setNotifOpen((o) => !o)}
              className={iconBtn}
              aria-label="Notifications"
            >
              <Bell className="h-4 w-4" />
              {unread > 0 && (
                <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-danger px-1 text-[9px] font-bold leading-none text-white ring-2 ring-card">
                  {unread > 9 ? "9+" : unread}
                </span>
              )}
            </button>
            {notifOpen && (
              <>
                <div className="fixed inset-0 z-10" onClick={() => setNotifOpen(false)} />
                <div className="absolute right-0 top-full z-20 mt-2 w-[calc(100vw-1rem)] max-w-sm overflow-hidden rounded-xl border border-border bg-card card-elev-lg animate-scale-in sm:w-80">
                  <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
                    <div className="text-sm font-semibold">Notifications</div>
                    {unread > 0 && (
                      <button
                        onClick={() => markRead(notifs.filter((n) => !n.read).map((n) => n.id))}
                        className="flex items-center gap-1 text-xs text-primary hover:underline"
                      >
                        <CheckCheck className="h-3.5 w-3.5" /> Mark all read
                      </button>
                    )}
                  </div>
                  <div className="max-h-96 overflow-y-auto">
                    {notifLoading && notifs.length === 0 && (
                      <p className="p-4 text-center text-sm text-muted-foreground">Loading notifications…</p>
                    )}
                    {notifError && (
                      <div className="flex flex-col items-center gap-2 p-4 text-center text-sm text-muted-foreground">
                        <span>Couldn&apos;t load notifications.</span>
                        <button onClick={() => void loadNotifications()} className="text-xs font-medium text-primary hover:underline">
                          Try again
                        </button>
                      </div>
                    )}
                    {!notifLoading && !notifError && notifs.length === 0 && (
                      <p className="p-4 text-center text-sm text-muted-foreground">You&apos;re all caught up — no notifications yet.</p>
                    )}
                    {notifs.map((n) => {
                      const Icon = notifIcon[n.type];
                      return (
                        <button
                          key={n.id}
                          onClick={() => {
                            if (!n.read) markRead([n.id]);
                          }}
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
                            {n.body && <span className="mt-0.5 block text-xs text-muted-foreground">{n.body}</span>}
                            <span className="mt-1 block text-[11px] text-muted-foreground">{timeAgo(new Date(n.at))}</span>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                  <Link
                    href="/dashboard"
                    onClick={() => setNotifOpen(false)}
                    className="block border-t border-border p-2.5 text-center text-sm text-primary hover:bg-muted"
                  >
                    Open dashboard
                  </Link>
                </div>
              </>
            )}
          </div>
        </div>

        {/* Avatar — same optical height as tray */}
        <div className="relative">
          <button
            onClick={() => setMenuOpen((o) => !o)}
            className="inline-flex h-9 w-9 items-center justify-center rounded-full transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Account menu"
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
                    onClick={() => {
                      setMenuOpen(false);
                      logout();
                    }}
                    className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-danger hover:bg-danger/10"
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
