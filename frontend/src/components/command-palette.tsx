"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Search,
  LayoutDashboard,
  FolderKanban,
  Users,
  Activity,
  Camera,
  CalendarClock,
  MonitorDot,
  BarChart3,
  Sparkles,
  CreditCard,
  Settings,
  Moon,
  Sun,
  CornerDownLeft,
  ArrowUp,
  ArrowDown,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { users } from "@/lib/tenant-data";
import { reportCatalog } from "@/lib/reports-data";
import { useTheme } from "@/components/theme-provider";
import { useSession } from "@/components/session-provider";
import { canAccess } from "@/lib/roles";
import { scopeProjects } from "@/lib/scope";

interface Item {
  id: string;
  label: string;
  hint?: string;
  group: string;
  icon: LucideIcon;
  action: () => void;
}

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const router = useRouter();
  const { theme, setTheme } = useTheme();
  const { user, workspace } = useSession();
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
      if (e.key === "Escape") setOpen(false);
    };
    const onOpen = () => setOpen(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("open-command-palette", onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("open-command-palette", onOpen);
    };
  }, []);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setTimeout(() => inputRef.current?.focus(), 20);
    }
  }, [open]);

  const go = (href: string) => () => {
    router.push(href);
    setOpen(false);
  };

  const items = useMemo<Item[]>(() => {
    const role = user.role;
    const allPages: (Item & { href: string })[] = [
      { id: "p-dash", label: "Dashboard", group: "Pages", icon: LayoutDashboard, href: "/dashboard", action: go("/dashboard") },
      { id: "p-proj", label: "Projects", group: "Pages", icon: FolderKanban, href: "/projects", action: go("/projects") },
      { id: "p-team", label: "Team", group: "Pages", icon: Users, href: "/team", action: go("/team") },
      { id: "p-monitor", label: "Member Monitor", group: "Pages", icon: MonitorDot, href: "/monitor", action: go("/monitor") },
      { id: "p-act", label: "Activities", group: "Pages", icon: Activity, href: "/activities", action: go("/activities") },
      { id: "p-shot", label: "Screenshots", group: "Pages", icon: Camera, href: "/screenshots", action: go("/screenshots") },
      { id: "p-time", label: "Timesheet", group: "Pages", icon: CalendarClock, href: "/timesheet", action: go("/timesheet") },
      { id: "p-reports", label: "Reports", group: "Pages", icon: BarChart3, href: "/reports", action: go("/reports") },
      { id: "p-insights", label: "Insights", group: "Pages", icon: Sparkles, href: "/insights", action: go("/insights") },
      { id: "p-billing", label: "Billing & Plan", group: "Pages", icon: CreditCard, href: "/billing", action: go("/billing") },
      { id: "p-settings", label: "Settings", group: "Pages", icon: Settings, href: "/settings", action: go("/settings") },
    ];
    const pages: Item[] = allPages.filter((p) => canAccess(role, p.href));

    const reports: Item[] = canAccess(role, "/reports")
      ? reportCatalog.map((r) => ({
          id: `r-${r.slug}`,
          label: r.title,
          hint: "Report",
          group: "Reports",
          icon: BarChart3,
          action: go(`/reports/${r.slug}`),
        }))
      : [];

    const projItems: Item[] = canAccess(role, "/projects")
      ? scopeProjects(user).map((p) => ({
          id: `proj-${p.id}`,
          label: p.title,
          hint: "Project",
          group: "Projects",
          icon: FolderKanban,
          action: go(`/projects/${p.id}`),
        }))
      : [];

    const memberItems: Item[] = canAccess(role, "/team")
      ? users.filter((u) => u.role !== "client").map((u) => ({
          id: `mem-${u.id}`,
          label: u.name,
          hint: u.designation,
          group: "Members",
          icon: Users,
          action: go("/team"),
        }))
      : [];

    const actions: Item[] = [
      {
        id: "a-theme",
        label: theme === "dark" ? "Switch to light mode" : "Switch to dark mode",
        group: "Actions",
        icon: theme === "dark" ? Sun : Moon,
        action: () => {
          setTheme(theme === "dark" ? "light" : "dark");
          setOpen(false);
        },
      },
    ];
    return [...pages, ...reports, ...projItems, ...memberItems, ...actions];
  }, [theme, user, workspace.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((i) => i.label.toLowerCase().includes(q) || i.hint?.toLowerCase().includes(q) || i.group.toLowerCase().includes(q));
  }, [items, query]);

  useEffect(() => setActive(0), [query]);

  const groups = useMemo(() => {
    const map = new Map<string, Item[]>();
    filtered.forEach((i) => {
      if (!map.has(i.group)) map.set(i.group, []);
      map.get(i.group)!.push(i);
    });
    return [...map.entries()];
  }, [filtered]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      filtered[active]?.action();
    }
  }

  if (!open) return null;

  let flatIndex = -1;

  return (
    <div className="fixed inset-0 z-[100] flex items-start justify-center p-4 pt-[12vh]">
      <div className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in" onClick={() => setOpen(false)} />
      <div className="relative w-full max-w-xl overflow-hidden rounded-2xl border border-border bg-card card-elev-lg animate-scale-in">
        <div className="flex items-center gap-3 border-b border-border px-4">
          <Search className="h-5 w-5 text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search pages, reports, projects, members…"
            className="h-14 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <kbd className="rounded border border-border bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">ESC</kbd>
        </div>

        <div className="max-h-[52vh] overflow-y-auto p-2">
          {filtered.length === 0 && (
            <div className="px-3 py-10 text-center text-sm text-muted-foreground">No results for “{query}”.</div>
          )}
          {groups.map(([group, list]) => (
            <div key={group} className="mb-1">
              <div className="px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{group}</div>
              {list.map((item) => {
                flatIndex++;
                const idx = flatIndex;
                const Icon = item.icon;
                return (
                  <button
                    key={item.id}
                    onClick={item.action}
                    onMouseEnter={() => setActive(idx)}
                    className={cn(
                      "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm",
                      idx === active ? "bg-accent text-accent-foreground" : "hover:bg-muted"
                    )}
                  >
                    <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="flex-1 truncate">{item.label}</span>
                    {item.hint && <span className="truncate text-xs text-muted-foreground">{item.hint}</span>}
                  </button>
                );
              })}
            </div>
          ))}
        </div>

        <div className="flex items-center gap-4 border-t border-border px-4 py-2 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1"><ArrowUp className="h-3 w-3" /><ArrowDown className="h-3 w-3" /> navigate</span>
          <span className="flex items-center gap-1"><CornerDownLeft className="h-3 w-3" /> open</span>
          <span className="ml-auto">Dosi-Tracker</span>
        </div>
      </div>
    </div>
  );
}
