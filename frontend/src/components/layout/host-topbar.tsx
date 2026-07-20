"use client";

import { useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Menu, LogOut, ShieldCheck } from "lucide-react";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { Avatar } from "@/components/ui/avatar";
import { useSession } from "@/components/session-provider";
import { cn } from "@/lib/utils";

const titles: Record<string, string> = {
  host: "Overview",
  tenants: "Tenants",
  plans: "Plans & Editions",
  users: "Global Users",
  billing: "Platform Billing",
};

const iconBtn =
  "inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function HostTopbar({ onMenuClick }: { onMenuClick: () => void }) {
  const pathname = usePathname();
  const { user, logout } = useSession();
  const [menuOpen, setMenuOpen] = useState(false);
  const segments = pathname.split("/").filter(Boolean);
  const section = segments[1] ?? "host";
  const label = titles[segments.length > 1 ? section : "host"] ?? "Overview";

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-3 border-b border-border/80 bg-background/85 px-4 backdrop-blur-xl lg:px-6">
      <button
        onClick={onMenuClick}
        className={cn(iconBtn, "border border-border lg:hidden")}
        aria-label="Open menu"
      >
        <Menu className="h-4 w-4" />
      </button>

      <div className="min-w-0 flex-1 sm:hidden">
        <div className="truncate text-sm font-semibold">{label}</div>
      </div>

      <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
        <span className="hidden h-8 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted-foreground sm:inline-flex">
          <ShieldCheck className="h-3.5 w-3.5 opacity-70" /> Platform
        </span>

        <div className="flex h-9 items-center gap-0.5 rounded-xl border border-border/80 bg-card px-0.5 shadow-[0_1px_0_rgba(12,21,36,0.04)]">
          <ThemeToggle className="h-8 w-8 rounded-lg border-0 bg-transparent shadow-none" />
        </div>

        <div className="relative">
          <button
            onClick={() => setMenuOpen((o) => !o)}
            className="inline-flex h-9 w-9 items-center justify-center rounded-full transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Account menu"
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
