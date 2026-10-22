import Link from "next/link";
import {
  Laptop,
  AlertTriangle,
  ShieldCheck,
  Lock,
  Cpu,
  Layers,
  Eye,
  Camera,
  Activity,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export const metadata = {
  title: "macOS Desktop Agent Installation & Permission Guide | Dosi-Tracker",
  description:
    "Step-by-step installation, Gatekeeper approval, Apple Silicon & Intel requirements, and TCC privacy permissions for Dosi-Tracker on macOS.",
};

export default function MacOsInstallPage() {
  return (
    <div className="space-y-8">
      {/* Page Title */}
      <div>
        <div className="flex items-center gap-2 mb-2">
          <Link href="/install" className="text-xs text-muted-foreground hover:text-foreground">
            Guides
          </Link>
          <span className="text-xs text-muted-foreground/60" aria-hidden="true">/</span>
          <span className="text-xs font-semibold text-primary-strong">macOS</span>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h1 className="font-display text-2xl sm:text-3xl font-bold tracking-tight text-foreground flex items-center gap-2.5">
            <Laptop className="h-7 w-7 text-primary-strong" aria-hidden="true" />
            macOS Desktop Agent Guide
          </h1>
          <Badge tone="warning">Source Build Only (Xcode 15+)</Badge>
        </div>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          Native Swift menu-bar agent for macOS 14 Sonoma and macOS 15 Sequoia. Works seamlessly on both Apple Silicon (M1/M2/M3/M4)
          and Intel Macs. Pre-built notarized packages are unreleased; this guide details compiling from source and granting required TCC privacy permissions.
        </p>
      </div>

      {/* Release & Code Signing Notice */}
      <Card variant="quiet" className="border-amber-500/30 bg-amber-500/5 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-amber-500 shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-1.5 text-xs text-muted-foreground">
            <h2 className="font-semibold text-sm text-foreground">Packaging &amp; Developer ID Notarization Status</h2>
            <p>
              An official signed and notarized <code className="font-mono text-foreground font-semibold">.dmg</code> installer is{" "}
              <strong className="text-foreground">currently pending Apple Developer ID certificate deployment</strong>.
            </p>
            <p>
              The current client is built from source. Ad-hoc signed local builds trigger macOS Gatekeeper on first launch.
              Follow the instructions below to approve the app in macOS Security settings.
            </p>
          </div>
        </div>
      </Card>

      {/* Supported Versions & Architectures */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Supported OS Versions &amp; Hardware</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Cpu className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Hardware Architecture
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">Apple Silicon:</strong> Native <code className="font-mono text-foreground">arm64</code> (M1, M2, M3, M4 series)</li>
              <li><strong className="text-foreground">Intel:</strong> Native <code className="font-mono text-foreground">x86_64</code> processors</li>
              <li>Universal binary support supported via Xcode build scripts</li>
              <li>Memory footprint: ~25 MB; optimized for background menu bar operation</li>
            </ul>
          </Card>
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Layers className="h-4 w-4 text-primary-strong" aria-hidden="true" /> OS Requirements &amp; Tools
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">macOS 14 Sonoma</strong> or <strong className="text-foreground">macOS 15 Sequoia</strong></li>
              <li>Xcode 15 or later (or Command Line Tools: <code className="font-mono text-foreground">xcode-select --install</code>)</li>
              <li>Swift 5.9+ compiler toolchain</li>
              <li>Git command-line client</li>
            </ul>
          </Card>
        </div>
      </div>

      {/* Building from Source Steps */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Build &amp; Installation Steps</h2>
        <Card className="p-4 bg-muted/40 font-mono text-xs text-foreground overflow-x-auto">
          <pre>{`# 1. Clone the repository
git clone https://github.com/DosiBridge/Dosi-Tracker.git
cd Dosi-Tracker/clients/macos

# 2. Run the verified application build script
./scripts/build-app.sh

# 3. Move the compiled bundle into Applications
cp -R .build/artifacts/DosiTracker.app /Applications/"Dosi Tracker.app"`}</pre>
        </Card>
        <p className="text-xs text-muted-foreground">
          The app will now be installed in <code className="font-mono text-foreground bg-muted px-1.5 py-0.5 rounded">/Applications/Dosi Tracker.app</code>.
          Moving it to Applications ensures macOS preserves permission grants and login item registrations.
        </p>
      </div>

      {/* Gatekeeper & Ad-Hoc Signing Workaround */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground flex items-center gap-2">
          <ShieldCheck className="h-5 w-5 text-primary-strong" aria-hidden="true" />
          First Launch: Approving in macOS Gatekeeper
        </h2>
        <p className="text-xs text-muted-foreground leading-relaxed">
          Because local builds use ad-hoc code signatures rather than an Apple Developer ID certificate, macOS blocks the app
          the first time with a message stating: <em>&ldquo;Dosi Tracker&rdquo; was blocked because it is not from an identified developer</em>.
        </p>

        <Card className="p-4 border-border/80 space-y-3 text-xs">
          <h3 className="font-semibold text-foreground">Step-by-Step Security Approval:</h3>
          <ol className="list-decimal pl-5 space-y-2 text-muted-foreground">
            <li>
              Open <strong className="text-foreground">System Settings → Privacy &amp; Security</strong>.
            </li>
            <li>
              Scroll down to the <strong className="text-foreground">Security</strong> section.
            </li>
            <li>
              Locate the prompt: <em className="text-foreground">&ldquo;Dosi Tracker&rdquo; was blocked from use because it is not from an identified developer</em>.
            </li>
            <li>
              Click <strong className="text-foreground bg-muted px-2 py-0.5 rounded border border-border">Open Anyway</strong> and enter your Mac administrator password.
            </li>
            <li>
              <em>Alternative shortcut:</em> In Finder, open <code className="font-mono text-foreground">/Applications</code>, <strong>Control-click</strong> (or right-click) <strong className="text-foreground">Dosi Tracker</strong>, and select <strong className="text-foreground">Open</strong>.
            </li>
          </ol>
        </Card>
      </div>

      {/* TCC Privacy Permissions Breakdown */}
      <div className="space-y-4">
        <div>
          <h2 className="font-display text-lg font-bold text-foreground">
            Privacy Permissions (TCC) &amp; Telemetry Breakdown
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            macOS requires explicit user authorization before an application may access screen buffers, input monitors, or active window metadata.
            Configure these in <strong className="text-foreground">System Settings → Privacy &amp; Security</strong>:
          </p>
        </div>

        <div className="overflow-x-auto rounded-xl border border-border/80 bg-card">
          <table className="w-full text-left text-xs">
            <thead className="bg-muted/50 border-b border-border/80 text-foreground font-semibold">
              <tr>
                <th className="p-3">Permission Pane</th>
                <th className="p-3">Used For</th>
                <th className="p-3">Behavior if Denied / Skipped</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/60 text-muted-foreground">
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground flex items-center gap-2">
                  <Laptop className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Screen Recording
                </td>
                <td className="p-3">Periodic screen captures via ScreenCaptureKit (display where active work occurs).</td>
                <td className="p-3">No screenshots uploaded. Time is still tracked normally.</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground flex items-center gap-2">
                  <Activity className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Input Monitoring
                </td>
                <td className="p-3">Counts keyboard strokes and mouse clicks per minute slice. Never records keys typed.</td>
                <td className="p-3 font-medium text-amber-600 dark:text-amber-400">
                  Activity is reported honestly as unmeasured (null), never as a misleading 0%.
                </td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground flex items-center gap-2">
                  <Eye className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Accessibility
                </td>
                <td className="p-3">Reads the window title of the focused application.</td>
                <td className="p-3">Applications recorded by name only (without window title).</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-medium text-foreground flex items-center gap-2">
                  <Camera className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Camera
                </td>
                <td className="p-3">One webcam frame per block (only if explicitly enabled by project owner).</td>
                <td className="p-3">No webcam photos captured.</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p className="text-[11px] text-muted-foreground italic">
          Note: macOS ties TCC permission grants to the binary&apos;s code signature. If you rebuild an ad-hoc binary from source,
          macOS may require re-approving permissions in System Settings.
        </p>
      </div>

      {/* First-Run Configuration & Keychain */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">First-Run Configuration &amp; Keychain Storage</h2>
        <p className="text-xs text-muted-foreground leading-relaxed">
          When Dosi Tracker opens, it resides in the macOS menu bar. Clicking the menu bar icon and selecting <strong className="text-foreground">Sign In</strong> opens the configuration window:
        </p>

        <Card className="p-4 border-border/80 space-y-2 text-xs">
          <ul className="list-disc pl-5 space-y-1.5 text-muted-foreground">
            <li><strong className="text-foreground">Workspace:</strong> Your workspace slug (e.g. <code className="font-mono text-foreground">acme</code>).</li>
            <li><strong className="text-foreground">Email &amp; Password:</strong> The account credentials created on the web dashboard.</li>
            <li><strong className="text-foreground">Advanced → Server Address:</strong> The API server address (e.g. <code className="font-mono text-foreground">https://tracker-api.example.com</code>). Copyable from your web dashboard Download page.</li>
          </ul>
        </Card>

        <p className="text-xs text-muted-foreground flex items-center gap-1.5">
          <Lock className="h-4 w-4 text-primary-strong shrink-0" aria-hidden="true" />
          <span>
            Credentials are encrypted and stored in your <strong className="text-foreground">macOS Login Keychain</strong> (Service:{" "}
            <code className="font-mono text-foreground">com.dosibridge.tracker</code>). Passwords are never written to disk or logs.
          </span>
        </p>
      </div>

      {/* Local Controls & Troubleshooting */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Menu Bar Controls, Troubleshooting &amp; Uninstall</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">Menu Bar Controls</h3>
            <p className="text-muted-foreground">The icon indicates live status:</p>
            <ul className="space-y-1 text-muted-foreground">
              <li><strong className="text-foreground">● Solid dot:</strong> Actively recording time block</li>
              <li><strong className="text-foreground">❚❚ Paused:</strong> Tracking paused by user</li>
              <li><strong className="text-foreground">○ Hollow circle:</strong> Away / system idle / display locked</li>
            </ul>
            <p className="text-muted-foreground pt-1">
              Menu offers: <strong>Pause / Resume</strong>, <strong>Timed breaks</strong> (15/30/60m), <strong>Switch project</strong>, and <strong>Quit</strong>.
            </p>
          </Card>
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">Permission Reset Command</h3>
            <p className="text-muted-foreground">If screen capture permissions become desynchronized after a rebuild, reset TCC grants:</p>
            <p className="font-mono text-[11px] text-foreground bg-muted/60 p-2 rounded">
              tccutil reset ScreenCapture com.dosibridge.tracker
            </p>
          </Card>
        </div>

        <Card variant="quiet" className="p-4 text-xs text-muted-foreground space-y-1.5">
          <h3 className="font-semibold text-foreground">Complete Uninstallation</h3>
          <p>
            To completely remove Dosi Tracker from your Mac:
          </p>
          <p className="font-mono text-[11px] text-foreground bg-muted/60 p-2 rounded">
            rm -rf &quot;/Applications/Dosi Tracker.app&quot; ~/Library/Application\ Support/DosiTracker
          </p>
          <p>Open <strong className="text-foreground">Keychain Access</strong> and delete the item named <code className="font-mono text-foreground">Dosi Tracker sign-in</code>.</p>
        </Card>
      </div>
    </div>
  );
}
