import Link from "next/link";
import { Radar, ExternalLink, ShieldCheck } from "lucide-react";
import { ThemeToggle } from "@/components/ui/theme-toggle";

export const metadata = {
  title: "Desktop Agent Installation & Capability Guides | Dosi-Tracker",
  description:
    "Truthful installation guides, architecture details, and privacy permission matrices for Linux, macOS, and Windows desktop agents.",
};

export default function InstallLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-background text-foreground flex flex-col">
      {/* Accessible skip link */}
      <a
        href="#install-main-content"
        className="sr-only rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-[100]"
      >
        Skip to main content
      </a>

      {/* Public Header */}
      <header className="border-b border-border/80 bg-card/60 backdrop-blur sticky top-0 z-40">
        <div className="mx-auto max-w-5xl px-4 py-3 flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <Link href="/" className="flex items-center gap-2.5 group">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl brand-gradient text-white shadow-sm transition-transform group-hover:scale-105">
                <Radar className="h-5 w-5" aria-hidden="true" />
              </div>
              <span className="font-display text-base font-bold tracking-tight">Dosi-Tracker</span>
            </Link>
            <span className="text-muted-foreground/60 text-sm hidden sm:inline" aria-hidden="true">/</span>
            <span className="text-sm font-medium text-muted-foreground hidden sm:inline">Installation Guides</span>
          </div>

          <div className="flex items-center gap-2 sm:gap-3 text-sm">
            <Link
              href="/download"
              className="hidden md:inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground font-medium px-2.5 py-1.5 rounded-lg hover:bg-muted transition-colors"
            >
              Dashboard Download <ExternalLink className="h-3 w-3 opacity-70" aria-hidden="true" />
            </Link>
            <Link
              href="/login"
              className="inline-flex items-center justify-center rounded-lg bg-primary/10 hover:bg-primary/20 text-primary-strong text-xs font-semibold px-3 py-1.5 transition-colors"
            >
              Sign In
            </Link>
            <ThemeToggle />
          </div>
        </div>

        {/* OS Sub-Navigation Bar */}
        <nav aria-label="Operating System Guides" className="border-t border-border/50 bg-muted/20">
          <div className="mx-auto max-w-5xl px-4 flex items-center gap-1 sm:gap-2 overflow-x-auto py-1.5 text-xs">
            <Link
              href="/install"
              className="px-2.5 py-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted font-medium transition-colors shrink-0"
            >
              Overview & Matrix
            </Link>
            <span className="text-muted-foreground/40" aria-hidden="true">|</span>
            <Link
              href="/install/linux"
              className="px-2.5 py-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted font-medium transition-colors shrink-0"
            >
              Linux (X11 & Wayland)
            </Link>
            <Link
              href="/install/macos"
              className="px-2.5 py-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted font-medium transition-colors shrink-0"
            >
              macOS (Apple Silicon & Intel)
            </Link>
            <Link
              href="/install/windows"
              className="px-2.5 py-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted font-medium transition-colors shrink-0"
            >
              Windows Desktop
            </Link>
          </div>
        </nav>
      </header>

      {/* Main Content */}
      <main id="install-main-content" tabIndex={-1} className="flex-1 mx-auto max-w-5xl w-full px-4 py-8 outline-none">
        {children}
      </main>

      {/* Public Footer */}
      <footer className="border-t border-border/80 bg-muted/30 py-6 text-xs text-muted-foreground">
        <div className="mx-auto max-w-5xl px-4 flex flex-col sm:flex-row items-center justify-between gap-4 text-center sm:text-left">
          <div className="flex items-center gap-2">
            <ShieldCheck className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
            <span>
              Transparent Client Invariants: No root/administrator requirements. Keystroke content is never recorded.
            </span>
          </div>
          <div className="flex items-center gap-3">
            <Link href="/install" className="hover:underline">All Guides</Link>
            <span>•</span>
            <Link href="/download" className="hover:underline">Agent Downloads</Link>
            <span>•</span>
            <Link href="/login" className="hover:underline">Web App</Link>
          </div>
        </div>
      </footer>
    </div>
  );
}
