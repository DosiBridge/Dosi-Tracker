"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Activity,
  Camera,
  CalendarClock,
  LayoutDashboard,
  FolderKanban,
  Users,
  Settings,
  ChevronLeft,
  BarChart3,
  Sparkles,
  MonitorDot,
  CreditCard,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/ui/avatar";
import { WorkspaceSwitcher } from "@/components/layout/workspace-switcher";
import { useSession } from "@/components/session-provider";
import { canAccess, roleLabels } from "@/lib/roles";

const nav = [
  { href: "/dashboard", label: "Dashboard", icon: LayoutDashboard, group: "Overview" },
  { href: "/projects", label: "Projects", icon: FolderKanban, group: "Overview" },
  { href: "/team", label: "Team", icon: Users, group: "Overview" },
  { href: "/monitor", label: "Member Monitor", icon: MonitorDot, group: "Monitor" },
  { href: "/activities", label: "Activities", icon: Activity, group: "Monitor" },
  { href: "/screenshots", label: "Screenshots", icon: Camera, group: "Monitor" },
  { href: "/timesheet", label: "Timesheet", icon: CalendarClock, group: "Monitor" },
  { href: "/reports", label: "Reports", icon: BarChart3, group: "Analytics" },
  { href: "/insights", label: "Insights", icon: Sparkles, group: "Analytics" },
  { href: "/billing", label: "Billing & Plan", icon: CreditCard, group: "Workspace" },
  { href: "/settings", label: "Settings", icon: Settings, group: "Workspace" },
];

export function Sidebar({
  collapsed,
  onToggle,
  mobileOpen,
  onMobileClose,
}: {
  collapsed: boolean;
  onToggle: () => void;
  mobileOpen: boolean;
  onMobileClose: () => void;
}) {
  const pathname = usePathname();
  const { user: currentUser } = useSession();
  const visibleNav = nav.filter((item) => canAccess(currentUser.role, item.href));

  return (
    <>
      {mobileOpen && (
        <div className="fixed inset-0 z-40 bg-black/50 backdrop-blur-sm lg:hidden" onClick={onMobileClose} />
      )}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-50 flex flex-col border-r border-border bg-sidebar text-sidebar-foreground transition-all duration-300",
          collapsed ? "w-[76px]" : "w-64",
          "lg:translate-x-0",
          mobileOpen ? "translate-x-0" : "-translate-x-full lg:translate-x-0"
        )}
      >
        {/* Workspace switcher (multi-tenant) */}
        <div className="p-3">
          <WorkspaceSwitcher collapsed={collapsed} />
        </div>

        {/* Nav */}
        <nav className="flex-1 space-y-1 overflow-y-auto px-3 py-4">
          {visibleNav.map((item, i) => {
            const active = pathname === item.href || pathname.startsWith(item.href + "/");
            const Icon = item.icon;
            const showGroup = i === 0 || visibleNav[i - 1].group !== item.group;
            return (
              <div key={item.href}>
                {showGroup && !collapsed && (
                  <div className={cn("px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground", i === 0 && "pt-0")}>
                    {item.group}
                  </div>
                )}
                {showGroup && collapsed && i !== 0 && <div className="my-2 h-px bg-border" />}
                <Link
                  href={item.href}
                  onClick={onMobileClose}
                  title={collapsed ? item.label : undefined}
                  className={cn(
                    "group relative flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium transition-colors",
                    active
                      ? "bg-accent text-accent-foreground"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground",
                    collapsed && "justify-center px-0"
                  )}
                >
                  {active && (
                    <span className="absolute left-0 top-1/2 h-5 w-1 -translate-y-1/2 rounded-r-full bg-primary" />
                  )}
                  <Icon className="h-[18px] w-[18px] shrink-0" />
                  {!collapsed && <span>{item.label}</span>}
                </Link>
              </div>
            );
          })}
        </nav>

        {/* User + collapse */}
        <div className="border-t border-border p-3">
          <div className={cn("flex items-center gap-3 rounded-lg p-2", collapsed && "justify-center")}>
            <Avatar name={currentUser.name} size="sm" status={currentUser.status} />
            {!collapsed && (
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">{currentUser.name}</div>
                <div className="truncate text-[11px] text-muted-foreground">{roleLabels[currentUser.role]}</div>
              </div>
            )}
          </div>
          <button
            onClick={onToggle}
            className={cn(
              "mt-2 hidden w-full items-center justify-center gap-2 rounded-lg py-2 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground lg:flex"
            )}
          >
            <ChevronLeft className={cn("h-4 w-4 transition-transform", collapsed && "rotate-180")} />
            {!collapsed && "Collapse"}
          </button>
        </div>
      </aside>
    </>
  );
}
