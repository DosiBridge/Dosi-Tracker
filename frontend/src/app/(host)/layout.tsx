"use client";

import { useState } from "react";
import { usePathname } from "next/navigation";
import Link from "next/link";
import { Lock } from "lucide-react";
import { HostSidebar } from "@/components/layout/host-sidebar";
import { HostTopbar } from "@/components/layout/host-topbar";
import { ToastViewport } from "@/components/toast";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { PageEnter } from "@/components/motion/reveal";
import { useSession } from "@/components/session-provider";
import { cn } from "@/lib/utils";

export default function HostLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const { isHost } = useSession();

  if (!isHost) {
    return (
      <div className="flex min-h-screen items-center justify-center p-4">
        <Card className="page-enter flex max-w-md flex-col items-center justify-center gap-4 py-16 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <Lock className="h-7 w-7" />
          </div>
          <div>
            <h2 className="text-lg font-semibold">Host area</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              This console is only for the platform operator. Sign in with the Platform Admin account.
            </p>
          </div>
          <div className="flex gap-2">
            <Link href="/login"><Button>Go to login</Button></Link>
            <Link href="/dashboard"><Button variant="outline">Dashboard</Button></Link>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="min-h-screen">
      {/* See the tenant layout: skips the nav stops on every page. */}
      <a
        href="#main-content"
        className="sr-only rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-[100]"
      >
        Skip to main content
      </a>
      <HostSidebar
        collapsed={collapsed}
        onToggle={() => setCollapsed((c) => !c)}
        mobileOpen={mobileOpen}
        onMobileClose={() => setMobileOpen(false)}
      />
      <div className={cn("transition-all duration-300", collapsed ? "lg:pl-[76px]" : "lg:pl-64")}>
        <HostTopbar onMenuClick={() => setMobileOpen(true)} />
        <main id="main-content" tabIndex={-1} className="app-canvas mx-auto max-w-[1400px] p-4 lg:p-6 outline-none">
          <PageEnter motionKey={pathname}>{children}</PageEnter>
        </main>
      </div>
      <ToastViewport />
    </div>
  );
}
