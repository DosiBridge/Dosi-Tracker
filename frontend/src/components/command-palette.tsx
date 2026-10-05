"use client";

import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Search,
  LayoutDashboard,
  FolderKanban,
  Users,
  Activity,
  CalendarClock,
  MonitorDot,
  BarChart3,
  CreditCard,
  Settings,
  Moon,
  Sun,
  CornerDownLeft,
  ArrowUp,
  ArrowDown,
  Download,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { users } from "@/lib/tenant-data";
import { reportCatalog } from "@/lib/reports-data";
import { useTheme } from "@/components/theme-provider";
import { useSession } from "@/components/session-provider";
import { useDialogFocus } from "@/components/ui/use-dialog-focus";
import { reportAccessFor } from "@/lib/report-math";
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

/**
 * ⌘K / Ctrl+K: jump to a page, report, project or member. A modal dialog
 * holding a combobox: the search field keeps focus while ↑ ↓ move through the
 * results (announced through aria-activedescendant), Enter opens one and
 * Escape closes — focus then returns to where it was.
 */
export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const router = useRouter();
  const { theme, setTheme } = useTheme();
  const { user, workspace, isLive, livePolicies } = useSession();
  // Who may see pay: the permissions the session read at sign-in (the role's defaults otherwise).
  const manageTeam = reportAccessFor(user.role, isLive ? livePolicies : null).manageTeam;
  const dialogRef = useDialogFocus<HTMLDivElement>(open);
  const baseId = useId();
  const listId = `${baseId}-results`;

  /** Open with a clean search (or close). */
  const toggle = useCallback((next: boolean) => {
    if (next) {
      setQuery("");
      setActive(0);
    }
    setOpen(next);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        // Whether it opens or closes, the next opening starts with a clean search.
        setQuery("");
        setActive(0);
        setOpen((o) => !o);
      }
      if (e.key === "Escape") setOpen(false);
    };
    const onOpen = () => toggle(true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("open-command-palette", onOpen);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("open-command-palette", onOpen);
    };
  }, [toggle]);

  const go = useCallback(
    (href: string) => () => {
      router.push(href);
      setOpen(false);
    },
    [router],
  );

  const items = useMemo<Item[]>(() => {
    const role = user.role;
    const allPages: (Item & { href: string })[] = [
      { id: "p-dash", label: "Dashboard", group: "Pages", icon: LayoutDashboard, href: "/dashboard", action: go("/dashboard") },
      { id: "p-proj", label: "Projects", group: "Pages", icon: FolderKanban, href: "/projects", action: go("/projects") },
      { id: "p-team", label: "Team", group: "Pages", icon: Users, href: "/team", action: go("/team") },
      { id: "p-monitor", label: "Member Monitor", group: "Pages", icon: MonitorDot, href: "/monitor", action: go("/monitor") },
      { id: "p-act", label: "Activity", group: "Pages", icon: Activity, href: "/activities", action: go("/activities") },
      { id: "p-screens", label: "Activity · screens", group: "Pages", icon: Activity, href: "/activities", action: go("/activities?view=screens") },
      { id: "p-time", label: "Timesheet", group: "Pages", icon: CalendarClock, href: "/timesheet", action: go("/timesheet") },
      { id: "p-reports", label: "Reports", group: "Pages", icon: BarChart3, href: "/reports", action: go("/reports") },
      { id: "p-billing", label: "Billing & Plan", group: "Pages", icon: CreditCard, href: "/billing", action: go("/billing") },
      { id: "p-agent", label: "Desktop agent", hint: "Download", group: "Pages", icon: Download, href: "/download", action: go("/download") },
      { id: "p-settings", label: "Settings", group: "Pages", icon: Settings, href: "/settings", action: go("/settings") },
    ];
    const pages: Item[] = allPages.filter((p) => canAccess(role, p.href));

    // Each report is offered only where its own route is open to the role (a client sees just theirs),
    // and pay only to whoever may see rates (Tracker.Team.Manage) — the hub lists the same set.
    const reports: Item[] = reportCatalog
      .filter((r) => canAccess(role, `/reports/${r.slug}`) && (r.requires !== "manageTeam" || manageTeam))
      .map((r) => ({
        id: `r-${r.slug}`,
        label: r.title,
        hint: "Report",
        group: "Reports",
        icon: BarChart3,
        action: go(`/reports/${r.slug}`),
      }));

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
      ? users
          .filter((u) => u.role !== "client")
          .map((u) => ({
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
  }, [theme, user, workspace.id, go, setTheme, manageTeam]); // eslint-disable-line react-hooks/exhaustive-deps

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((i) => i.label.toLowerCase().includes(q) || i.hint?.toLowerCase().includes(q) || i.group.toLowerCase().includes(q));
  }, [items, query]);

  const groups = useMemo(() => {
    const map = new Map<string, Item[]>();
    filtered.forEach((i) => {
      if (!map.has(i.group)) map.set(i.group, []);
      map.get(i.group)!.push(i);
    });
    return [...map.entries()];
  }, [filtered]);

  const optionId = (index: number) => `${baseId}-option-${index}`;
  const current = filtered.length > 0 ? Math.min(active, filtered.length - 1) : -1;

  // Keep the highlighted result in view as the arrows move it.
  useEffect(() => {
    if (!open || current < 0) return;
    document.getElementById(optionId(current))?.scrollIntoView({ block: "nearest" });
  }, [open, current]); // eslint-disable-line react-hooks/exhaustive-deps

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Home") {
      e.preventDefault();
      setActive(0);
    } else if (e.key === "End") {
      e.preventDefault();
      setActive(Math.max(0, filtered.length - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (current >= 0) filtered[current]?.action();
    }
  }

  if (!open) return null;

  let flatIndex = -1;

  return (
    <div className="fixed inset-0 z-[100] flex items-start justify-center p-4 pt-[12vh]">
      <div aria-hidden className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in" onClick={() => setOpen(false)} />
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label="Search and jump"
        className="relative w-full max-w-xl overflow-hidden rounded-2xl border border-border bg-card outline-none card-elev-lg animate-scale-in"
      >
        <div className="flex items-center gap-3 border-b border-border px-4">
          <Search aria-hidden className="h-5 w-5 text-muted-foreground" />
          <input
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            placeholder="Search pages, reports, projects, members…"
            aria-label="Search pages, reports, projects and members"
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={current >= 0 ? optionId(current) : undefined}
            className="h-14 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <kbd className="rounded border border-border bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">ESC</kbd>
        </div>

        {filtered.length === 0 && (
          <p role="status" className="px-3 py-10 text-center text-sm text-muted-foreground">
            No results for “{query}”.
          </p>
        )}
        <div id={listId} role="listbox" aria-label="Results" className={cn("max-h-[52vh] overflow-y-auto p-2", filtered.length === 0 && "hidden")}>
          {groups.map(([group, list]) => {
            const headingId = `${baseId}-group-${group.replace(/\s+/g, "-")}`;
            return (
              <div key={group} role="group" aria-labelledby={headingId} className="mb-1">
                <div id={headingId} role="presentation" className="px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                  {group}
                </div>
                {list.map((item) => {
                  flatIndex++;
                  const idx = flatIndex;
                  const Icon = item.icon;
                  const selected = idx === current;
                  return (
                    <div
                      key={item.id}
                      id={optionId(idx)}
                      role="option"
                      aria-selected={selected}
                      onClick={item.action}
                      onMouseMove={() => setActive(idx)}
                      className={cn(
                        "flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm",
                        selected ? "bg-accent text-accent-foreground" : "hover:bg-muted",
                      )}
                    >
                      <Icon aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
                      <span className="flex-1 truncate">{item.label}</span>
                      {item.hint && <span className="truncate text-xs text-muted-foreground">{item.hint}</span>}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>

        <div aria-hidden className="flex items-center gap-4 border-t border-border px-4 py-2 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <ArrowUp className="h-3 w-3" />
            <ArrowDown className="h-3 w-3" /> navigate
          </span>
          <span className="flex items-center gap-1">
            <CornerDownLeft className="h-3 w-3" /> open
          </span>
          <span className="ml-auto">Dosi-Tracker</span>
        </div>
      </div>
    </div>
  );
}
