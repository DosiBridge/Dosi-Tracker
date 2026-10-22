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
  title: "Linux Desktop Agent Foundation & Specification | Dosi-Tracker",
  description:
    "Technical architecture specification, headless foundation crate, display server boundaries (X11 vs Wayland), and unprivileged security invariants for Dosi-Tracker Linux.",
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
            Linux Desktop Agent Foundation &amp; Specification
          </h1>
          <Badge tone="warning">Headless Source Foundation (No Native Collection / Sync Yet)</Badge>
        </div>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          Technical architecture, display server boundaries (X11 vs. Wayland), headless diagnostic CLI foundation,
          and privacy invariants for the Linux desktop agent.
        </p>
      </div>

      {/* Honest Implementation Status Notice */}
      <Card variant="quiet" className="border-amber-500/40 bg-amber-500/10 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-amber-500 shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-2 text-xs text-muted-foreground">
            <h2 className="font-semibold text-sm text-foreground">
              Status: Headless Source Foundation (Diagnostic CLI &amp; Consent Core)
            </h2>
            <p>
              The repository now contains the first finite headless Linux implementation slice in{" "}
              <code className="font-mono text-foreground font-semibold">clients/linux</code> (crate:{" "}
              <code className="font-mono text-foreground font-semibold">dosi-tracker-linux</code> v0.1.0).
            </p>
            <p>
              It provides a working diagnostic command (<code className="font-mono text-foreground font-semibold">dosi-tracker-linux doctor [--json]</code>),
              safe non-mutating XDG path resolution, and a default-inactive local consent and lifecycle core with ABA generation invalidation.
            </p>
            <p>
              <strong className="text-foreground">Critical Boundaries:</strong> All native collection adapters (screen capture, keyboard/mouse hooks,
              active window sampler, webcam) report <code className="font-mono text-foreground font-semibold">NOT_IMPLEMENTED</code>.
              Global input counting and window titles on Wayland are architecturally <code className="font-mono text-foreground font-semibold">UNSUPPORTED</code>.
              No native sensor is attached, no offline queue or network sync is active, and no pre-built binaries or installers are released.
              Tests have been verified on Windows host toolchain (27 passed); native Linux compilation and runtime remain unverified.
            </p>
          </div>
        </div>
      </Card>

      {/* Actual Implemented CLI Command Syntax */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Implemented Diagnostic Command Syntax</h2>
        <Card className="p-4 space-y-3 text-xs text-muted-foreground">
          <p>
            The headless client foundation in <code className="font-mono text-foreground">clients/linux</code> can be inspected
            using the real diagnostic command. It runs without <code className="font-mono text-foreground">DISPLAY</code>, D-Bus, credentials, or sensor access:
          </p>
          <div className="space-y-2 font-mono text-[11px] bg-muted/60 p-3 rounded-lg border border-border/80 text-foreground overflow-x-auto">
            <div># Run capability and session diagnostics (plain text)</div>
            <div className="text-primary-strong">cargo run --bin dosi-tracker-linux -- doctor</div>
            <div className="mt-2"># Run diagnostics with structured JSON output</div>
            <div className="text-primary-strong">cargo run --bin dosi-tracker-linux -- doctor --json</div>
            <div className="mt-2"># Inspect usage and privacy invariants</div>
            <div className="text-primary-strong">cargo run --bin dosi-tracker-linux -- --help</div>
          </div>
          <p className="text-[11px]">
            * Note: The <code className="font-mono text-foreground font-semibold">doctor</code> command performs pure non-mutating inspection:
            it checks existence of XDG configuration and data paths without creating any directories or files.
          </p>
        </Card>
      </div>

      {/* Rootless Source-Build Installation Lifecycle */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Rootless Source-Build Installation Lifecycle</h2>
        <Card className="p-4 space-y-3 text-xs text-muted-foreground">
          <p>
            An unprivileged, user-scoped installation lifecycle script is provided at{" "}
            <code className="font-mono text-foreground font-semibold">clients/linux/scripts/install.sh</code>. It compiles the locked crate locally
            and installs the diagnostic executable into an explicit user-controlled prefix (default: <code className="font-mono text-foreground">~/.local</code>):
          </p>
          <div className="space-y-2 font-mono text-[11px] bg-muted/60 p-3 rounded-lg border border-border/80 text-foreground overflow-x-auto">
            <div># 1. Fresh rootless install to ~/.local/bin (requires local Rust/Cargo)</div>
            <div className="text-primary-strong">bash clients/linux/scripts/install.sh install</div>
            <div className="mt-2"># Or install to an explicit user-controlled prefix:</div>
            <div className="text-primary-strong">bash clients/linux/scripts/install.sh install --prefix ~/.local</div>
            <div className="mt-2"># 2. Run installed diagnostic probe</div>
            <div className="text-primary-strong">~/.local/bin/dosi-tracker-linux doctor [--json]</div>
            <div className="mt-2"># 3. Update existing installation (with staging and prior backup)</div>
            <div className="text-primary-strong">bash clients/linux/scripts/install.sh update</div>
            <div className="mt-2"># 4. Rollback to retained previous installation (if needed)</div>
            <div className="text-primary-strong">bash clients/linux/scripts/install.sh rollback</div>
            <div className="mt-2"># 5. Clean rootless uninstallation (preserves user config and consent)</div>
            <div className="text-primary-strong">bash clients/linux/scripts/uninstall.sh</div>
          </div>
          <div className="space-y-1.5 text-[11px] pt-1">
            <p className="font-semibold text-foreground">Safety &amp; Ownership Invariants:</p>
            <ul className="list-disc pl-4 space-y-1 text-foreground/80">
              <li><strong className="text-foreground">Strictly Rootless:</strong> Refuses to run with <code className="font-mono text-foreground">sudo</code> or as root; system paths (<code className="font-mono text-foreground">/usr</code>, <code className="font-mono text-foreground">/etc</code>) and symlink escapes are rejected.</li>
              <li><strong className="text-foreground">Ownership Manifest:</strong> Writes an explicit manifest (<code className="font-mono text-foreground">~/.local/share/dosi-tracker/manifest.json</code>); refuses to overwrite unmanaged foreign files.</li>
              <li><strong className="text-foreground">Atomic Staging:</strong> Builds and validates binary in a temporary staging directory before activating.</li>
              <li><strong className="text-foreground">Consent Preservation:</strong> Uninstallation deletes only owned files; user configuration and consent at <code className="font-mono text-foreground">~/.config/dosi-tracker</code> are strictly preserved.</li>
              <li><strong className="text-foreground">Diagnostic Scope:</strong> Installs the sensor-free diagnostic foundation executable only; no autostart, systemd service, or background tracking daemons are registered.</li>
            </ul>
          </div>
        </Card>
      </div>

      {/* Target Platforms for Future Implementation */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Target Platforms &amp; Architecture</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Cpu className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Target Architectures &amp; Kernel
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">x86_64:</strong> 64-bit AMD / Intel processors (tier 1)</li>
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
            Display Server Architecture: X11 vs. Wayland
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            Linux display servers enforce fundamentally different security models. The architecture respects compositor
            boundaries and strictly forbids privileged keylogging or kernel device inspection.
          </p>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* Wayland Subsystem */}
          <Card className="border-border/80">
            <CardHeader className="pb-3 border-b border-border/60 bg-muted/20">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base font-semibold">Wayland Compositors</CardTitle>
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
                  Window hierarchies are private to the compositor on standard GNOME/KDE sessions. Active window titles must be
                  reported as <code className="font-mono text-foreground font-semibold">null</code>. User-owned process names can be read via
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

      {/* Planned Configuration & Credential Storage Specification */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Storage &amp; Credential Specification</h2>
        <p className="text-xs text-muted-foreground">
          Configuration and consent are stored per-user following the XDG Base Directory specification at{" "}
          <code className="font-mono text-foreground bg-muted px-1.5 py-0.5 rounded">~/.config/dosi-tracker/consent.json</code>.
          Future credentials will be stored in the Freedesktop Secret Service API via D-Bus (<code className="font-mono text-foreground">libsecret</code>, backed by GNOME Keyring or KWallet).
        </p>

        <Card variant="quiet" className="p-4 border border-border/80 text-xs text-muted-foreground space-y-2">
          <p className="font-semibold text-foreground">Note on Storage Non-Mutation:</p>
          <p>
            Running <code className="font-mono text-foreground font-semibold">dosi-tracker-linux doctor</code> inspects paths without creating
            directories or writing files. User consent persistence is explicitly managed and tested strictly within isolated directories.
          </p>
        </Card>
      </div>

      {/* Implementation Roadmap & Acceptance Gaps */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Linux Implementation Roadmap &amp; Finite Next Slices</h2>
        <Card className="p-4 space-y-3 text-xs text-muted-foreground">
          <div className="flex items-center gap-2 font-semibold text-foreground">
            <Clock className="h-4 w-4 text-primary-strong" aria-hidden="true" />
            <span>Structured Implementation Slices</span>
          </div>
          <ol className="list-decimal pl-5 space-y-2">
            <li>
              <strong className="text-foreground">Slice 1 (Delivered):</strong> Headless foundation crate (<code className="font-mono text-foreground">clients/linux</code>), non-mutating <code className="font-mono text-foreground font-semibold">doctor [--json]</code> capability diagnostics, safe XDG path resolution, default-deny consent core, and ABA lifecycle generation invalidation (27 tests passed on Windows toolchain).
            </li>
            <li>
              <strong className="text-foreground">Slice 2 (Next):</strong> Authentication, workspace enrollment, credential storage via Freedesktop Secret Service D-Bus, and offline-first SQLite queue schema.
            </li>
            <li>
              <strong className="text-foreground">Slice 3 (Future):</strong> Native consent-gated collection adapters (X11 XInput2 / EWMH, Wayland portal ScreenCast via PipeWire, V4L2 webcam).
            </li>
            <li>
              <strong className="text-foreground">Slice 4 (Future):</strong> Native Linux packaging (DEB, RPM), systemd user service integration, automated updates, and clean uninstallation.
            </li>
          </ol>
        </Card>
      </div>
    </div>
  );
}
