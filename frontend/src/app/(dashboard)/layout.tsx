"use client";

import { useState } from "react";
import { usePathname, useRouter } from "next/navigation";
import { Eye, LogOut } from "lucide-react";
import { Sidebar } from "@/components/layout/sidebar";
import { Topbar } from "@/components/layout/topbar";
import { CommandPalette } from "@/components/command-palette";
import { ToastViewport } from "@/components/toast";
import { RoleGuard } from "@/components/role-guard";
import { PageEnter } from "@/components/motion/reveal";
import { useSession } from "@/components/session-provider";
import { cn } from "@/lib/utils";

export default function DashboardLayout({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const { workspace, user, isImpersonating, stopImpersonating } = useSession();

  return (
    <div className="min-h-screen">
      {/* Keyboard users otherwise tab through ~16 nav stops on every page
          before reaching content. Visible only while focused. */}
      <a
        href="#main-content"
        className="sr-only rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-[100]"
      >
        Skip to main content
      </a>
      <Sidebar
        collapsed={collapsed}
        onToggle={() => setCollapsed((c) => !c)}
        mobileOpen={mobileOpen}
        onMobileClose={() => setMobileOpen(false)}
      />
      <div className={cn("transition-all duration-300", collapsed ? "lg:pl-[76px]" : "lg:pl-64")}>
        {isImpersonating && (
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 bg-primary px-4 py-2 text-sm text-primary-foreground lg:px-6">
            <Eye className="h-4 w-4 shrink-0" />
            <span className="min-w-0">
              Viewing <span className="font-semibold">{workspace.name}</span> as{" "}
              <span className="font-semibold">{user.name}</span> — host impersonation
            </span>
            <button
              onClick={() => {
                stopImpersonating();
                router.push("/host");
              }}
              className="ml-auto inline-flex items-center gap-1.5 rounded-md bg-white/20 px-2.5 py-1 text-xs font-medium hover:bg-white/30"
            >
              <LogOut className="h-3.5 w-3.5" /> Return to Host
            </button>
          </div>
        )}
        <Topbar onMenuClick={() => setMobileOpen(true)} />
        <main id="main-content" tabIndex={-1} key={workspace.id} className="app-canvas mx-auto max-w-[1400px] p-4 lg:p-6 outline-none">
          <PageEnter motionKey={`${workspace.id}:${pathname}`}>
            <RoleGuard>{children}</RoleGuard>
          </PageEnter>
        </main>
      </div>
      <CommandPalette />
      <ToastViewport />
    </div>
  );
}
