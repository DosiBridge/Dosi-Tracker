"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Menu, ChevronRight, LogOut, ShieldCheck } from "lucide-react";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { useSession } from "@/components/session-provider";

const titles: Record<string, string> = {
  host: "Overview",
  tenants: "Tenants",
  plans: "Plans & Editions",
  users: "Global Users",
  billing: "Platform Billing",
};

export function HostTopbar({ onMenuClick }: { onMenuClick: () => void }) {
  const pathname = usePathname();
  const { user, logout } = useSession();
  const [menuOpen, setMenuOpen] = useState(false);
  const segments = pathname.split("/").filter(Boolean);
  const section = segments[1] ?? "host";

  return (
    <header className="sticky top-0 z-30 flex h-16 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur-xl lg:px-6">
      <button
        onClick={onMenuClick}
        className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted lg:hidden"
        aria-label="Open menu"
      >
        <Menu className="h-5 w-5" />
      </button>

      <div className="hidden items-center gap-1.5 text-sm sm:flex">
        <span className="font-semibold">Host</span>
        <ChevronRight className="h-4 w-4 text-muted-foreground" />
        <span className="text-muted-foreground">{titles[segments.length > 1 ? section : "host"] ?? "Overview"}</span>
      </div>

      <div className="ml-auto flex items-center gap-2">
        <Badge tone="primary" className="hidden gap-1 sm:inline-flex">
          <ShieldCheck className="h-3.5 w-3.5" /> Platform Operator
        </Badge>

        <ThemeToggle />

        <div className="relative">
          <button
            onClick={() => setMenuOpen((o) => !o)}
            className="flex items-center gap-2 rounded-lg p-1 pr-2 transition-colors hover:bg-muted"
          >
            <Avatar name={user.name} size="sm" status="active" />
          </button>
          {menuOpen && (
            <>
              <div className="fixed inset-0 z-10" onClick={() => setMenuOpen(false)} />
              <div className="absolute right-0 top-full z-20 mt-2 w-60 overflow-hidden rounded-xl border border-border bg-card card-elev-lg animate-scale-in">
                <div className="flex items-center gap-3 border-b border-border p-3">
                  <Avatar name={user.name} size="md" />
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium">{user.name}</div>
                    <div className="truncate text-xs text-muted-foreground">{user.email}</div>
                  </div>
                </div>
                <div className="border-t border-border p-1">
                  <Link
                    href="/login"
                    onClick={() => { setMenuOpen(false); logout(); }}
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
