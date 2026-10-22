import Link from "next/link";
import {
  Monitor,
  Laptop,
  Terminal,
  ArrowRight,
  ShieldCheck,
  Info,
  ExternalLink,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export const metadata = {
  title: "Desktop Agent Installation & Platform Matrix | Dosi-Tracker",
  description:
    "Cross-platform desktop agent installation guides, version support, packaging status, and truthful display-server capability matrix for Linux, macOS, and Windows.",
};

export default function InstallOverviewPage() {
  return (
    <div className="space-y-8">
      {/* Hero Header */}
      <div>
        <div className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-primary/10 text-primary-strong text-xs font-semibold mb-3">
          <ShieldCheck className="h-3.5 w-3.5" aria-hidden="true" />
          <span>Multi-Platform Desktop Onboarding</span>
        </div>
        <h1 className="font-display text-2xl sm:text-3xl font-bold tracking-tight text-foreground">
          Desktop Agent Installation &amp; Capability Guides
        </h1>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          Dosi-Tracker operates native background desktop agents that record time blocks, active project tasks, and
          workspace-configured telemetry. This documentation establishes truthful, source-proven installation steps,
          distinguishes available release packages from source-only builds, and details display-server boundaries
          across Linux, macOS, and Windows.
        </p>
      </div>

      {/* Platform Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {/* Linux Card */}
        <Card className="flex flex-col border-border/80 hover:border-primary/50 transition-colors">
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Terminal className="h-5 w-5 text-primary-strong" aria-hidden="true" />
                <CardTitle className="text-base font-semibold">Linux</CardTitle>
              </div>
              <Badge tone="warning" className="text-[10px]">Source Preview</Badge>
            </div>
            <p className="text-xs text-muted-foreground mt-1">Ubuntu 22.04+, Fedora 38+, Arch, Debian 12+</p>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-between gap-4 text-xs text-muted-foreground">
            <div className="space-y-2.5">
              <p>
                Targeted at modern X11 and Wayland environments. Operates strictly as an unprivileged user process —
                never requires <code className="font-mono text-foreground font-semibold">root</code> or <code className="font-mono text-foreground font-semibold">sudo</code>.
              </p>
              <ul className="space-y-1.5 list-disc pl-4 text-foreground/80">
                <li><strong className="text-foreground">Architectures:</strong> x86_64, aarch64</li>
                <li><strong className="text-foreground">Packaging:</strong> Pre-built packages unreleased; architecture preview</li>
                <li><strong className="text-foreground">Wayland:</strong> Portal dialog screen selection; input activity unsupported (null)</li>
              </ul>
            </div>
            <Link
              href="/install/linux"
              className="mt-2 inline-flex items-center justify-center gap-1.5 w-full rounded-lg bg-primary/10 hover:bg-primary/20 text-primary-strong font-medium py-2 transition-colors text-xs"
            >
              Linux Setup Guide <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
            </Link>
          </CardContent>
        </Card>

        {/* macOS Card */}
        <Card className="flex flex-col border-border/80 hover:border-primary/50 transition-colors">
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Laptop className="h-5 w-5 text-primary-strong" aria-hidden="true" />
                <CardTitle className="text-base font-semibold">macOS</CardTitle>
              </div>
              <Badge tone="warning" className="text-[10px]">Source Build Only</Badge>
            </div>
            <p className="text-xs text-muted-foreground mt-1">macOS 14 Sonoma or macOS 15 Sequoia</p>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-between gap-4 text-xs text-muted-foreground">
            <div className="space-y-2.5">
              <p>
                Native Swift/Cocoa menu-bar agent built against Apple ScreenCaptureKit. Pre-built notarized packages are unreleased;
                compiled from source using Xcode 15+.
              </p>
              <ul className="space-y-1.5 list-disc pl-4 text-foreground/80">
                <li><strong className="text-foreground">Architectures:</strong> Apple Silicon (arm64) and Intel (x86_64)</li>
                <li><strong className="text-foreground">Signing:</strong> Ad-hoc build requires Gatekeeper &ldquo;Open Anyway&rdquo; grant</li>
                <li><strong className="text-foreground">Permissions:</strong> Standard TCC Screen Recording, Input Monitoring &amp; Accessibility</li>
              </ul>
            </div>
            <Link
              href="/install/macos"
              className="mt-2 inline-flex items-center justify-center gap-1.5 w-full rounded-lg bg-primary/10 hover:bg-primary/20 text-primary-strong font-medium py-2 transition-colors text-xs"
            >
              macOS Setup Guide <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
            </Link>
          </CardContent>
        </Card>

        {/* Windows Card */}
        <Card className="flex flex-col border-border/80 hover:border-primary/50 transition-colors">
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Monitor className="h-5 w-5 text-primary-strong" aria-hidden="true" />
                <CardTitle className="text-base font-semibold">Windows</CardTitle>
              </div>
              <Badge tone="primary" className="text-[10px]">Pre-Built Installer</Badge>
            </div>
            <p className="text-xs text-muted-foreground mt-1">Windows 10 (1809+) or Windows 11</p>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-between gap-4 text-xs text-muted-foreground">
            <div className="space-y-2.5">
              <p>
                High-performance Rust/Win32 tray agent. Installs per-user without administrator privileges.
                Encrypted with Windows DPAPI.
              </p>
              <ul className="space-y-1.5 list-disc pl-4 text-foreground/80">
                <li><strong className="text-foreground">Architectures:</strong> x86_64 (64-bit)</li>
                <li><strong className="text-foreground">Packaging:</strong> <code className="font-mono text-[11px]">dosi-tracker-setup-&lt;version&gt;.exe</code> via NSIS</li>
                <li><strong className="text-foreground">Signing:</strong> Authenticode signing pending; SmartScreen &ldquo;Run anyway&rdquo;</li>
              </ul>
            </div>
            <Link
              href="/install/windows"
              className="mt-2 inline-flex items-center justify-center gap-1.5 w-full rounded-lg bg-primary/10 hover:bg-primary/20 text-primary-strong font-medium py-2 transition-colors text-xs"
            >
              Windows Setup Guide <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
            </Link>
          </CardContent>
        </Card>
      </div>

      {/* Comparison & Evidence Matrix */}
      <div className="space-y-4">
        <div>
          <h2 className="font-display text-lg font-bold text-foreground">
            Truthful Platform Capabilities &amp; Evidence Matrix
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            This table reflects the actual codebase capabilities, storage mechanisms, and release boundaries verified in the repository.
          </p>
        </div>

        <div className="overflow-x-auto rounded-xl border border-border/80 bg-card">
          <table className="w-full text-left text-xs">
            <thead className="bg-muted/50 border-b border-border/80 text-foreground font-semibold">
              <tr>
                <th className="p-3">Platform</th>
                <th className="p-3">Supported Versions</th>
                <th className="p-3">Packaging &amp; Binary Status</th>
                <th className="p-3">Screen Capture</th>
                <th className="p-3">Input Activity</th>
                <th className="p-3">Active Window Title</th>
                <th className="p-3">Credential Vault</th>
                <th className="p-3">Admin / Root Needed</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/60 text-muted-foreground">
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground">Linux (X11)</td>
                <td className="p-3">Ubuntu 22+, Fedora 38+, Arch</td>
                <td className="p-3"><Badge tone="warning" className="text-[10px]">Source Preview / Unreleased</Badge></td>
                <td className="p-3">Supported (XShmGetImage / xcap)</td>
                <td className="p-3">Supported (XInput2 raw counters)</td>
                <td className="p-3">Supported (_NET_ACTIVE_WINDOW)</td>
                <td className="p-3">Secret Service API (libsecret)</td>
                <td className="p-3 font-medium text-success">No (Unprivileged)</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground">Linux (Wayland)</td>
                <td className="p-3">GNOME 42+, KDE Plasma 5.27+, Sway</td>
                <td className="p-3"><Badge tone="warning" className="text-[10px]">Source Preview / Unreleased</Badge></td>
                <td className="p-3">Supported (xdg-desktop-portal ScreenCast)</td>
                <td className="p-3 font-medium text-amber-600 dark:text-amber-400">UNSUPPORTED (Reported as null)</td>
                <td className="p-3 font-medium text-amber-600 dark:text-amber-400">UNSUPPORTED (Reported as null)</td>
                <td className="p-3">Secret Service API (libsecret)</td>
                <td className="p-3 font-medium text-success">No (Strictly Prohibited)</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground">macOS (Apple Silicon &amp; Intel)</td>
                <td className="p-3">macOS 14 Sonoma, macOS 15 Sequoia</td>
                <td className="p-3"><Badge tone="warning" className="text-[10px]">Source Build Only (Xcode 15+)</Badge></td>
                <td className="p-3">Supported (ScreenCaptureKit)</td>
                <td className="p-3">Supported (Input Monitoring counter)</td>
                <td className="p-3">Supported (Accessibility API)</td>
                <td className="p-3">macOS Login Keychain</td>
                <td className="p-3 font-medium text-success">No (Standard User)</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground">Windows Desktop</td>
                <td className="p-3">Windows 10 (1809+) &amp; 11 (x86_64)</td>
                <td className="p-3"><Badge tone="primary" className="text-[10px]">Pre-Built Setup EXE (NSIS)</Badge></td>
                <td className="p-3">Supported (xcap / Win32 GDI)</td>
                <td className="p-3">Supported (Low-level hooks counter)</td>
                <td className="p-3">Supported (GetForegroundWindow)</td>
                <td className="p-3">Windows DPAPI (CryptProtectData)</td>
                <td className="p-3 font-medium text-success">No (Per-User Install)</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      {/* Onboarding & Connection Guidance */}
      <Card variant="quiet" className="p-4 sm:p-5 border border-border/80">
        <div className="flex items-start gap-3">
          <Info className="h-5 w-5 text-primary-strong shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-2 text-xs text-muted-foreground">
            <h3 className="font-semibold text-sm text-foreground">First-Run Server Configuration</h3>
            <p>
              Desktop agents require three credentials to establish synchronization: the <strong className="text-foreground">API Server URL</strong> (e.g.{" "}
              <code className="font-mono bg-muted px-1.5 py-0.5 rounded text-foreground">https://tracker-api.example.com</code>), your{" "}
              <strong className="text-foreground">Workspace Name</strong>, and your <strong className="text-foreground">Email and Password</strong>.
            </p>
            <p>
              If you are already signed in to your workspace, navigate to the{" "}
              <Link href="/download" className="text-primary-strong font-medium hover:underline inline-flex items-center gap-1">
                Dashboard Download Page <ExternalLink className="h-3 w-3" aria-hidden="true" />
              </Link>{" "}
              to copy your server address and view personalized onboarding credentials.
            </p>
          </div>
        </div>
      </Card>
    </div>
  );
}
