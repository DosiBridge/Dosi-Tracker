import Link from "next/link";
import {
  Terminal,
  AlertTriangle,
  ShieldCheck,
  CheckCircle2,
  XCircle,
  Layers,
  Cpu,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export const metadata = {
  title: "Linux Desktop Agent Installation & Architecture Guide | Dosi-Tracker",
  description:
    "Technical installation guide, display server boundaries (X11 vs Wayland), and unprivileged security invariants for Dosi-Tracker on Linux.",
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
            Linux Desktop Agent Guide
          </h1>
          <Badge tone="warning">Pre-built Binaries Unreleased</Badge>
        </div>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          Comprehensive installation, display server boundaries (X11 vs. Wayland), and privacy invariants for Linux.
          Dosi-Tracker operates strictly as an unprivileged user process and enforces honest null reporting for Wayland-isolated telemetry.
        </p>
      </div>

      {/* Distribution Status Notice */}
      <Card variant="quiet" className="border-amber-500/30 bg-amber-500/5 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-amber-500 shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-1.5 text-xs text-muted-foreground">
            <h2 className="font-semibold text-sm text-foreground">Packaging &amp; Distribution Status</h2>
            <p>
              Pre-built packages (<code className="font-mono text-foreground font-semibold">.deb</code>, <code className="font-mono text-foreground font-semibold">.rpm</code>,{" "}
              <code className="font-mono text-foreground font-semibold">AppImage</code>, <code className="font-mono text-foreground font-semibold">Flatpak</code>) are{" "}
              <strong className="text-foreground">currently unreleased</strong>. The Linux agent architecture is specified in{" "}
              <code className="font-mono text-foreground">docs/CONSENT_CAPABILITY_AUDIT.md</code>.
            </p>
            <p>
              Do not attempt to download unofficial binary packages. When published, official packages will be hosted exclusively on our verified GitHub Releases.
            </p>
          </div>
        </div>
      </Card>

      {/* Prerequisites & Supported Distributions */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Supported Distributions &amp; Architectures</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Cpu className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Architectures &amp; Kernel
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">x86_64:</strong> 64-bit AMD / Intel processors</li>
              <li><strong className="text-foreground">aarch64:</strong> 64-bit ARM (e.g. Raspberry Pi 5, Asahi Linux)</li>
              <li><strong className="text-foreground">Kernel:</strong> Linux kernel 5.15 LTS or newer</li>
              <li><strong className="text-foreground">C Library:</strong> glibc 2.35+ or musl 1.2+</li>
            </ul>
          </Card>
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Layers className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Target Distributions
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li>Ubuntu 22.04 LTS (Jammy Jellyfish) and 24.04 LTS (Noble Numbat)</li>
              <li>Debian 12 (Bookworm) and newer</li>
              <li>Fedora Workstation 38, 39, 40+</li>
              <li>Arch Linux / Manjaro</li>
              <li>Desktop session running systemd user instance and D-Bus</li>
            </ul>
          </Card>
        </div>
      </div>

      {/* X11 vs Wayland Display Server Breakdown */}
      <div className="space-y-4">
        <div>
          <h2 className="font-display text-lg font-bold text-foreground">
            Display Server Architecture: X11 vs. Wayland
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            Linux display servers enforce fundamentally different security models. Dosi-Tracker respects compositor
            boundaries and never attempts privileged keylogging or bypasses.
          </p>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* Wayland Subsystem */}
          <Card className="border-border/80">
            <CardHeader className="pb-3 border-b border-border/60 bg-muted/20">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base font-semibold">Wayland Compositors</CardTitle>
                <Badge tone="primary" className="text-[10px]">GNOME, KDE Plasma, Sway</Badge>
              </div>
              <p className="text-xs text-muted-foreground">Default on Ubuntu 22.04+, Fedora, Debian 12</p>
            </CardHeader>
            <CardContent className="p-4 space-y-3 text-xs text-muted-foreground">
              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-success shrink-0" aria-hidden="true" />
                  <span>Screen Capture via Portal</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Implemented through <code className="font-mono text-foreground font-semibold">org.freedesktop.portal.ScreenCast</code> and PipeWire.
                  When the agent requests a capture, the desktop compositor shows an <strong className="text-foreground">interactive OS dialog</strong> requiring
                  the user to select the display. If cancelled, the status is recorded as <code className="font-mono text-foreground">denied</code> without infinite retries.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <XCircle className="h-4 w-4 text-amber-500 shrink-0" aria-hidden="true" />
                  <span>Global Input Activity: UNSUPPORTED (null)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Wayland strictly isolates input events to the focused application surface. Background input monitoring is
                  <strong className="text-foreground"> architecturally unsupported</strong>. Keyboard hits and mouse clicks are reported
                  truthfully as <code className="font-mono text-foreground font-semibold">null</code> (unsupported), never fabricated zeros.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <XCircle className="h-4 w-4 text-amber-500 shrink-0" aria-hidden="true" />
                  <span>Active Window Title: UNSUPPORTED (null)</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Window hierarchies are private to the compositor on standard GNOME/KDE sessions. Active window titles are
                  reported as <code className="font-mono text-foreground font-semibold">null</code>. User-owned process names are read via
                  <code className="font-mono text-foreground font-semibold">/proc/[pid]/cmdline</code>.
                </p>
              </div>
            </CardContent>
          </Card>

          {/* X11 Subsystem */}
          <Card className="border-border/80">
            <CardHeader className="pb-3 border-b border-border/60 bg-muted/20">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base font-semibold">Legacy X11 Server</CardTitle>
                <Badge tone="muted" className="text-[10px]">Xorg / X.Org</Badge>
              </div>
              <p className="text-xs text-muted-foreground">Traditional unconfined X11 desktop sessions</p>
            </CardHeader>
            <CardContent className="p-4 space-y-3 text-xs text-muted-foreground">
              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-success shrink-0" aria-hidden="true" />
                  <span>Screen Capture</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Direct display capture via standard X11 protocol (<code className="font-mono text-foreground">XShmGetImage</code> / xcap).
                  Unconfined user processes capture screen buffers without modal prompts.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-success shrink-0" aria-hidden="true" />
                  <span>Input Activity Counters</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Counted via <code className="font-mono text-foreground font-semibold">XInput2</code> raw event masks (<code className="font-mono text-foreground">XI_RawKeyPress</code>,{" "}
                  <code className="font-mono text-foreground">XI_RawButtonPress</code>). Counts aggregate ticks per minute; keystroke content is never read or stored.
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center gap-1.5 font-semibold text-foreground">
                  <CheckCircle2 className="h-4 w-4 text-success shrink-0" aria-hidden="true" />
                  <span>Active Window Title</span>
                </div>
                <p className="pl-5 leading-relaxed">
                  Determined by querying the root window property <code className="font-mono text-foreground">_NET_ACTIVE_WINDOW</code> and reading
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
            The Linux client operates <strong className="text-foreground">exclusively as an unprivileged user process</strong>.
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

      {/* First-Run Configuration */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">First-Run Configuration &amp; Credentials</h2>
        <p className="text-xs text-muted-foreground">
          Configuration is stored per-user following the XDG Base Directory specification at{" "}
          <code className="font-mono text-foreground bg-muted px-1.5 py-0.5 rounded">~/.config/dosi-tracker/config.toml</code>.
        </p>

        <Card className="p-4 bg-muted/40 font-mono text-xs text-foreground overflow-x-auto">
          <pre>{`# ~/.config/dosi-tracker/config.toml
api_base_url = "https://tracker-api.example.com"
workspace = "acme"
email = "worker@example.com"
# Optional overrides:
# log_level = "info"
# sync_interval_seconds = 30`}</pre>
        </Card>

        <p className="text-xs text-muted-foreground">
          <strong className="text-foreground">Credential Storage:</strong> Passwords and API bearer tokens are managed securely via the
          Freedesktop Secret Service D-Bus API (<code className="font-mono text-foreground">libsecret</code>, backed by GNOME Keyring or KWallet).
          Passwords are never stored in plaintext configuration files.
        </p>
      </div>

      {/* Local Controls & Troubleshooting */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Local Controls, Troubleshooting &amp; Removal</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">CLI Service Controls</h3>
            <p className="text-muted-foreground">Manage tracking and status directly from the terminal:</p>
            <ul className="space-y-1 font-mono text-[11px] text-foreground bg-muted/50 p-2.5 rounded">
              <li>dosi-tracker status</li>
              <li>dosi-tracker pause --minutes 15</li>
              <li>dosi-tracker resume</li>
              <li>dosi-tracker stop</li>
            </ul>
          </Card>
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">Session Verification</h3>
            <p className="text-muted-foreground">Check your active display server session and portal service:</p>
            <ul className="space-y-1 font-mono text-[11px] text-foreground bg-muted/50 p-2.5 rounded">
              <li>echo $XDG_SESSION_TYPE</li>
              <li>systemctl --user status xdg-desktop-portal</li>
            </ul>
          </Card>
        </div>

        <Card variant="quiet" className="p-4 text-xs text-muted-foreground space-y-1.5">
          <h3 className="font-semibold text-foreground">Complete Uninstallation</h3>
          <p>
            To completely remove the agent and all local state, terminate the process and remove configuration and queued cache directories:
          </p>
          <p className="font-mono text-[11px] text-foreground bg-muted/60 p-2 rounded">
            rm -rf ~/.config/dosi-tracker ~/.local/share/dosi-tracker
          </p>
        </Card>
      </div>
    </div>
  );
}
