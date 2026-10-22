import Link from "next/link";
import {
  Monitor,
  AlertTriangle,
  Lock,
  ExternalLink,
  Cpu,
  Layers,
  Download,
  Terminal,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { AGENT_RELEASES_URL } from "@/components/dashboard/agent-setup";

export const metadata = {
  title: "Windows Desktop Agent Installation & Setup Guide | Dosi-Tracker",
  description:
    "Installation instructions, SmartScreen approval, silent enterprise deployment, and DPAPI credential protection for Dosi-Tracker on Windows 10 and 11.",
};

export default function WindowsInstallPage() {
  return (
    <div className="space-y-8">
      {/* Page Title */}
      <div>
        <div className="flex items-center gap-2 mb-2">
          <Link href="/install" className="text-xs text-muted-foreground hover:text-foreground">
            Guides
          </Link>
          <span className="text-xs text-muted-foreground/60" aria-hidden="true">/</span>
          <span className="text-xs font-semibold text-primary-strong">Windows</span>
        </div>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h1 className="font-display text-2xl sm:text-3xl font-bold tracking-tight text-foreground flex items-center gap-2.5">
            <Monitor className="h-7 w-7 text-primary-strong" aria-hidden="true" />
            Windows Desktop Agent Guide
          </h1>
          <Badge tone="primary">Pre-Built Installer (NSIS)</Badge>
        </div>
        <p className="mt-2 text-sm text-muted-foreground max-w-3xl leading-relaxed">
          High-performance Rust desktop agent for Windows 10 and Windows 11. Lives in the Notification Area (system tray),
          installs per-user without requiring administrator privileges, and protects stored credentials using Windows DPAPI.
        </p>
      </div>

      {/* Code Signing Notice */}
      <Card variant="quiet" className="border-blue-500/30 bg-blue-500/5 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-blue-500 shrink-0 mt-0.5" aria-hidden="true" />
          <div className="space-y-1.5 text-xs text-muted-foreground">
            <h2 className="font-semibold text-sm text-foreground">Authenticode Code Signing Status</h2>
            <p>
              The Windows NSIS installer is generated during release packaging. Production Authenticode code signing with an EV certificate is{" "}
              <strong className="text-foreground">pending enterprise release pipeline configuration</strong>.
            </p>
            <p>
              When running the installer on a modern Windows machine, Windows Defender SmartScreen may display an informational notice.
              Follow the verified SmartScreen instructions below.
            </p>
          </div>
        </div>
      </Card>

      {/* Supported Versions & System Requirements */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Supported Windows Versions &amp; Hardware</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Cpu className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Architecture &amp; Binary Target
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">x86_64:</strong> 64-bit Windows PC (AMD64 / Intel 64)</li>
              <li>Native Rust binary target: <code className="font-mono text-foreground">x86_64-pc-windows-msvc</code></li>
              <li>Lightweight execution: ~18 MB RAM consumption</li>
              <li>Installer package: <code className="font-mono text-foreground">dosi-tracker-setup-&lt;version&gt;.exe</code></li>
            </ul>
          </Card>
          <Card className="p-4">
            <h3 className="font-semibold text-foreground flex items-center gap-2 mb-2">
              <Layers className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Supported OS Editions
            </h3>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">Windows 11:</strong> Home, Pro, Enterprise, Education (all builds)</li>
              <li><strong className="text-foreground">Windows 10:</strong> Version 1809 or newer (64-bit)</li>
              <li>Installs for current user only: <code className="font-mono text-foreground">%LOCALAPPDATA%\Programs\DosiTracker</code></li>
              <li>Zero administrative or UAC elevation prompts required for standard use</li>
            </ul>
          </Card>
        </div>
      </div>

      {/* Installation Steps & SmartScreen Handling */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground flex items-center gap-2">
          <Download className="h-5 w-5 text-primary-strong" aria-hidden="true" />
          Standard User Installation Steps
        </h2>

        <Card className="p-5 border-border/80 space-y-4 text-xs">
          <ol className="list-decimal pl-5 space-y-3 text-muted-foreground leading-relaxed">
            <li>
              <strong className="text-foreground">Download the Installer:</strong> Download the latest setup executable from{" "}
              <a
                href={AGENT_RELEASES_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="text-primary-strong font-semibold hover:underline inline-flex items-center gap-1"
              >
                GitHub Releases <ExternalLink className="h-3 w-3" aria-hidden="true" />
              </a>.
            </li>
            <li>
              <strong className="text-foreground">Run the Executable:</strong> Launch <code className="font-mono text-foreground">dosi-tracker-setup-&lt;version&gt;.exe</code>.
              It installs to <code className="font-mono text-foreground">%LOCALAPPDATA%\Programs\DosiTracker</code>, creates a Start Menu shortcut,
              and configures auto-start when you log in to Windows.
            </li>
            <li>
              <strong className="text-foreground">Windows SmartScreen Approval:</strong> If Windows Defender SmartScreen displays{" "}
              <span className="font-semibold text-foreground">&ldquo;Windows protected your PC&rdquo;</span>:
              <div className="mt-2 pl-4 border-l-2 border-primary/40 space-y-1 text-foreground">
                <p>1. Click <span className="font-bold underline">More info</span>.</p>
                <p>2. Click the <span className="font-bold bg-muted px-2 py-0.5 rounded border border-border">Run anyway</span> button that appears.</p>
              </div>
            </li>
            <li>
              <strong className="text-foreground">Sign In:</strong> The application opens to the sign-in screen and places an icon in the Notification Area (system tray).
            </li>
          </ol>
        </Card>
      </div>

      {/* Silent Enterprise / Administrator Deployment */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground flex items-center gap-2">
          <Terminal className="h-5 w-5 text-primary-strong" aria-hidden="true" />
          Silent Enterprise &amp; Managed Deployment
        </h2>
        <p className="text-xs text-muted-foreground leading-relaxed">
          System administrators can deploy the agent across fleet workstations silently with pre-configured server addresses:
        </p>

        <Card className="p-4 bg-muted/40 font-mono text-xs text-foreground overflow-x-auto">
          <pre>{`# Silent installation with pre-configured API server URL and Dashboard:
dosi-tracker-setup-0.1.0.exe /S /SERVER=https://tracker-api.example.com /DASHBOARD=https://app.example.com`}</pre>
        </Card>

        <div className="overflow-x-auto rounded-xl border border-border/80 bg-card">
          <table className="w-full text-left text-xs">
            <thead className="bg-muted/50 border-b border-border/80 text-foreground font-semibold">
              <tr>
                <th className="p-3">Switch</th>
                <th className="p-3">Description</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/60 text-muted-foreground">
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-mono font-medium text-foreground">/S</td>
                <td className="p-3">Runs the NSIS installer in completely silent, unattended mode.</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-mono font-medium text-foreground">/SERVER=&lt;url&gt;</td>
                <td className="p-3">Sets the default API server URL in <code className="font-mono text-foreground">config.toml</code> (<code className="font-mono text-foreground">api_base_url</code>).</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-mono font-medium text-foreground">/DASHBOARD=&lt;url&gt;</td>
                <td className="p-3">Sets the default web dashboard address linked from the app (<code className="font-mono text-foreground">dashboard_url</code>).</td>
              </tr>
              <tr className="hover:bg-muted/20">
                <td className="p-3 font-mono font-medium text-foreground">/D=&lt;folder&gt;</td>
                <td className="p-3">Custom installation directory (must be the final argument on the command line).</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      {/* First-Run Configuration & DPAPI Security */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">First-Run Configuration &amp; DPAPI Protection</h2>
        <p className="text-xs text-muted-foreground leading-relaxed">
          When the sign-in window opens, enter your <strong className="text-foreground">Workspace</strong>, <strong className="text-foreground">Email</strong>,
          and <strong className="text-foreground">Password</strong>. If your administrator did not deploy with <code className="font-mono text-foreground">/SERVER</code>,
          click <strong className="text-foreground">Advanced</strong> and paste your server address from the web dashboard.
        </p>

        <Card variant="quiet" className="p-4 text-xs text-muted-foreground space-y-2">
          <div className="flex items-center gap-2 text-foreground font-semibold">
            <Lock className="h-4 w-4 text-primary-strong" aria-hidden="true" />
            <span>Windows Data Protection API (DPAPI) Keying</span>
          </div>
          <p>
            Your account credentials and authentication tokens are encrypted using <strong className="text-foreground">Windows DPAPI</strong> (<code className="font-mono text-foreground">CryptProtectData</code>).
            The encryption key is tied to your Windows user logon security identifier (SID). Credentials are never stored in cleartext files or environment variables.
          </p>
        </Card>
      </div>

      {/* Local Controls, Pause, Stop & Troubleshooting */}
      <div className="space-y-4">
        <h2 className="font-display text-lg font-bold text-foreground">Application Controls, Troubleshooting &amp; Uninstall</h2>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-xs">
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">System Tray &amp; Tabs</h3>
            <p className="text-muted-foreground">The main window provides three organized tabs:</p>
            <ul className="space-y-1 text-muted-foreground list-disc pl-4">
              <li><strong className="text-foreground">Timer:</strong> Project selector, task notes, Pause / Resume, and timed breaks (15, 30, 60m).</li>
              <li><strong className="text-foreground">Activity:</strong> Today&apos;s activity %, top apps, and upload queue status.</li>
              <li><strong className="text-foreground">Settings:</strong> Launch on Windows startup, privacy rules, and logs.</li>
            </ul>
          </Card>
          <Card className="p-4 space-y-2">
            <h3 className="font-semibold text-foreground">Graceful Termination &amp; Upgrades</h3>
            <p className="text-muted-foreground">To quit or prepare for an upgrade:</p>
            <p className="font-mono text-[11px] text-foreground bg-muted/60 p-2 rounded">
              dosi-tracker.exe --quit
            </p>
            <p className="text-muted-foreground">
              Finishes and uploads the in-progress time block before exiting cleanly.
            </p>
          </Card>
        </div>

        <Card variant="quiet" className="p-4 text-xs text-muted-foreground space-y-2">
          <h3 className="font-semibold text-foreground">Uninstallation &amp; Cache Cleanup</h3>
          <p>
            To uninstall Dosi Tracker:
          </p>
          <ol className="list-decimal pl-5 space-y-1 text-muted-foreground">
            <li>Open <strong className="text-foreground">Windows Settings → Apps → Installed apps</strong>.</li>
            <li>Locate <strong className="text-foreground">Dosi Tracker</strong>, click the three dots, and select <strong className="text-foreground">Uninstall</strong>.</li>
            <li>Alternatively, execute: <code className="font-mono text-foreground">%LOCALAPPDATA%\Programs\DosiTracker\uninstall.exe</code>.</li>
            <li>To delete unsent queued activity blocks: delete <code className="font-mono text-foreground">%LOCALAPPDATA%\DosiTracker\queue.db</code>.</li>
          </ol>
        </Card>
      </div>
    </div>
  );
}
