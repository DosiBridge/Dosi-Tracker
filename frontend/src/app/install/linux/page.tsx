import Link from "next/link";
import {
  Terminal,
  AlertTriangle,
  ShieldCheck,
  XCircle,
  Layers,
  Cpu,
  Clock,
  CheckCircle2,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export const metadata = {
  title: "Linux Desktop Agent Technical Specification & Architecture | Dosi-Tracker",
  description:
    "Technical architecture specification, display server boundaries (X11 vs Wayland), and unprivileged security invariants for future Linux agent implementation.",
};

export default function LinuxInstallPage() {
  return (
    <div className="space-y-8">
      {/* Page Title */}
      <div>
        <div className="flex items-center gap-2 mb-2">
          <Link href="/install" className="text-xs text-muted-foreground hover:text-foreground">
            Guides
          </Link>
          <span className="text-xs text-muted-foreground/60" aria-hidden="true">/</span>
          <span className="text-xs font-semibold text-primary-strong">Linux</span>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h1 className="font-display text-2xl sm:text-3xl font-bold tracking-tight text-foreground flex items-center gap-2.5">
            <Terminal className="h-7 w-7 text-primary-strong" aria-hidden="true" />
            Linux Desktop Agent Specification
          </h1>
          <Badge tone="warning">Client Not Implemented (Architecture Planned)</Badge>
        </div>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          Technical architecture, display server boundaries (X11 vs. Wayland), and privacy invariants for the planned Linux desktop agent.
          There is currently no installable client binary or source crate in the repository.
        </p>
      </div>

      {/* Honest Implementation Status Notice */}
      <Card variant="quiet" className="border-amber-500/40 bg-amber-500/10 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-amber-500 shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-2 text-xs text-muted-foreground">
            <h2 className="font-semibold text-sm text-foreground">
              Status: Client Not Implemented (Planned Architecture)
            </h2>
            <p>
              There is currently <strong className="text-foreground">no Linux client implementation in the repository</strong> (git tracked files confirm no crate under <code className="font-mono text-foreground font-semibold">clients/linux</code>, no installable binaries, and no CLI executable).
            </p>
            <p>
              Do not attempt to execute terminal commands or configure local config files; no client process exists to read them.
              The technical specifications below document the required protocol boundaries, display server privacy constraints, and anti-keylogger invariants for future implementation.
            </p>
          </div>
        </div>
      </Card>

      {/* Target Platforms for Future Implementation */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Target Platforms for Future Implementation</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Cpu className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Target Architectures &amp; Kernel
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">x86_64:</strong> 64-bit AMD / Intel processors (planned tier 1)</li>
              <li><strong className="text-foreground">aarch64:</strong> 64-bit ARM (e.g. Raspberry Pi 5, Asahi Linux)</li>
              <li><strong className="text-foreground">Kernel:</strong> Linux kernel 5.15 LTS or newer</li>
              <li><strong className="text-foreground">C Library:</strong> glibc 2.35+ or musl 1.2+</li>
            </ul>
          </Card>
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Layers className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Target Distributions (Planned)
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li>Ubuntu 22.04 LTS (Jammy Jellyfish) and 24.04 LTS (Noble Numbat)</li>
              <li>Debian 12 (Bookworm) and newer</li>
              <li>Fedora Workstation 38, 39, 40+</li>
              <li>Arch Linux</li>
              <li>Prerequisites: systemd user session instance and D-Bus</li>
            </ul>
          </Card>
        </div>
      </div>

      {/* Planned X11 vs Wayland Display Server Architecture */}
      <div className="space-y-4">
        <div>
          <h2 className="font-display text-lg font-bold text-foreground">
            Planned Display Server Architecture: X11 vs. Wayland
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            Linux display servers enforce fundamentally different security models. The planned architecture respects compositor
            boundaries and strictly forbids privileged keylogging or kernel device inspection.
          </p>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* Wayland Subsystem (Planned) */}
          <Card className="border-border/80">
            <CardHeader className="pb-3 border-b border-border/60 bg-muted/20">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base font-semibold">Wayland Compositors (Planned)</CardTitle>
                <Badge tone="muted" className="text-[10px]">GNOME, KDE Plasma, Sway</Badge>
              </div>
              <p className="text-xs text-muted-foreground">Default on modern Ubuntu, Fedora, Debian</p>
            </CardHeader>
            <CardContent className="p-4 space-y-3 text-xs text-muted-foreground">
              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
                  <span>Screen Capture via Portal (Specification)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Required specification: Must be implemented via <code className="font-mono text-foreground font-semibold">org.freedesktop.portal.ScreenCast</code> and PipeWire.
                  When capture is requested, the desktop compositor shows an <strong className="text-foreground">interactive OS dialog</strong> requiring
                  the user to select the display. If cancelled, the status must record as <code className="font-mono text-foreground">denied</code> without infinite retries.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <XCircle className="h-4 w-4 text-amber-500 shrink-0" aria-hidden="true" />
                  <span>Global Input Activity: UNSUPPORTED (null)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Wayland strictly isolates input events to the focused application surface. Background input monitoring is
                  <strong className="text-foreground"> architecturally unsupported</strong>. Keyboard hits and mouse clicks will be reported
                  truthfully as <code className="font-mono text-foreground font-semibold">null</code> (unsupported), never fabricated zeros.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <XCircle className="h-4 w-4 text-amber-500 shrink-0" aria-hidden="true" />
                  <span>Active Window Title: UNSUPPORTED (null)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Window hierarchies are private to the compositor on standard GNOME/KDE sessions. Active window titles must be
                  reported as <code className="font-mono text-foreground font-semibold">null</code>. User-owned process names can be read via
                  <code className="font-mono text-foreground font-semibold">/proc/[pid]/cmdline</code>.
                </p>
              </div>
            </CardContent>
          </Card>

          {/* X11 Subsystem (Planned) */}
          <Card className="border-border/80">
            <CardHeader className="pb-3 border-b border-border/60 bg-muted/20">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base font-semibold">Legacy X11 Server (Planned)</CardTitle>
                <Badge tone="muted" className="text-[10px]">Xorg / X.Org</Badge>
              </div>
              <p className="text-xs text-muted-foreground">Traditional unconfined X11 desktop sessions</p>
            </CardHeader>
            <CardContent className="p-4 space-y-3 text-xs text-muted-foreground">
              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
                  <span>Screen Capture (Specification)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Planned display capture via standard X11 protocol (<code className="font-mono text-foreground">XShmGetImage</code> / xcap).
                  Unconfined user processes capture screen buffers without modal prompts.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
                  <span>Input Activity Counters (Specification)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Planned via <code className="font-mono text-foreground font-semibold">XInput2</code> raw event masks (<code className="font-mono text-foreground">XI_RawKeyPress</code>,{" "}
                  <code className="font-mono text-foreground">XI_RawButtonPress</code>). Counts aggregate ticks per minute; keystroke content is never read or stored.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
                  <span>Active Window Title (Specification)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Planned by querying the root window property <code className="font-mono text-foreground">_NET_ACTIVE_WINDOW</code> and reading
                  the window name property <code className="font-mono text-foreground">_NET_WM_NAME</code>.
                </p>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>

      {/* Unprivileged Security Policy */}
      <Card className="border-border/80">
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-semibold flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-success" aria-hidden="true" />
            Unprivileged Execution Invariant (Anti-Keylogger Policy)
          </CardTitle>
        </CardHeader>
        <CardContent className="text-xs text-muted-foreground space-y-2 leading-relaxed">
          <p>
            When built, the Linux client will operate <strong className="text-foreground">exclusively as an unprivileged user process</strong>.
            The architecture strictly prohibits:
          </p>
          <ul className="list-disc pl-5 space-y-1 text-foreground/80">
            <li>Running with <code className="font-mono font-bold text-danger">sudo</code> or as <code className="font-mono font-bold text-danger">root</code>.</li>
            <li>Installing SUID root executable binaries.</li>
            <li>Adding desktop user accounts to the <code className="font-mono font-semibold text-foreground">input</code> group to read <code className="font-mono font-semibold text-foreground">/dev/input/event*</code> raw kernel devices (evdev).</li>
          </ul>
          <p>
            Bypassing Wayland input isolation via evdev turns an application into an unrestricted hardware keylogger capable of intercepting root
            passwords across all user sessions. Dosi-Tracker refuses to implement or recommend evdev workarounds.
          </p>
        </CardContent>
      </Card>

      {/* Planned Configuration & Credential Storage Specification */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Planned Configuration &amp; Credential Storage Specification</h2>
        <p className="text-xs text-muted-foreground">
          Future implementation specification: Configuration must be stored per-user following the XDG Base Directory specification at{" "}
          <code className="font-mono text-foreground bg-muted px-1.5 py-0.5 rounded">~/.config/dosi-tracker/config.toml</code>, and credentials
          must be stored in the Freedesktop Secret Service API via D-Bus (<code className="font-mono text-foreground">libsecret</code>, backed by GNOME Keyring or KWallet).
        </p>

        <Card variant="quiet" className="p-4 border border-border/80 text-xs text-muted-foreground space-y-2">
          <p className="font-semibold text-foreground">Note on Client State:</p>
          <p>
            Because the Linux client is currently not implemented in the repository, <strong className="text-foreground">no configuration file is created or read</strong>.
            Do not create manual configuration files or attempt to run client commands.
          </p>
        </Card>
      </div>

      {/* Implementation Roadmap & Acceptance Gaps */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Linux Implementation Roadmap &amp; Acceptance Gaps</h2>
        <Card className="p-4 space-y-3 text-xs text-muted-foreground">
          <div className="flex items-center gap-2 font-semibold text-foreground">
            <Clock className="h-4 w-4 text-primary-strong" aria-hidden="true" />
            <span>Required Backlog Deliverables Prior to Verification</span>
          </div>
          <ol className="list-decimal pl-5 space-y-2">
            <li>
              <strong className="text-foreground">Greenfield Rust Client Crate:</strong> Create <code className="font-mono text-foreground">clients/linux</code> with a decoupled sensor driver (<code className="font-mono text-foreground">SensorDriver</code> trait) matching the Windows decoupled architecture.
            </li>
            <li>
              <strong className="text-foreground">X11 Protocol Engine:</strong> Implement root window query (<code className="font-mono text-foreground">_NET_ACTIVE_WINDOW</code>) and XInput2 aggregate tick counters with sensor-free unit test fixtures.
            </li>
            <li>
              <strong className="text-foreground">Wayland Desktop Portal Integration:</strong> Implement D-Bus portal calls for ScreenCast dialog handling and verify honest null reporting for unsupported window titles and input counts.
            </li>
            <li>
              <strong className="text-foreground">Packaging &amp; CI Pipeline:</strong> Define RPM/DEB packaging manifests and automated build validation on Linux build agents.
            </li>
          </ol>
        </Card>
      </div>
    </div>
  );
}
