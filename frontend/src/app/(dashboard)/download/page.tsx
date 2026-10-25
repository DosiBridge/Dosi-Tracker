"use client";

import Link from "next/link";
import { ArrowRight, BookOpen, Download, Laptop, Monitor, ShieldCheck, Sparkles, Terminal } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { SignInDetails, useVisitorOs, type VisitorOs } from "@/components/dashboard/agent-setup";
import {
  AGENT_PLATFORMS,
  INSTALL_GUIDES_PATH,
  LINUX_COMMANDS,
  checksumsUrl,
  releaseAssetUrl,
  releasesPageUrl,
  type AgentPlatform,
} from "@/lib/agent-downloads";
import { runtimeConfig } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";

/**
 * Where people get the desktop agent. Tracking only starts once an agent runs
 * on the member's computer, so the onboarding checklist, the sidebar and the
 * invite result point here. The agent can't guess the server it belongs to,
 * so the page shows the address to paste into its sign-in screen. Every file
 * name and link comes from lib/agent-downloads (shared with the /install
 * guides), and downloads come from the deployment's configured location.
 */
export default function DownloadPage() {
  const { user, isLive } = useSession();
  const os = useVisitorOs();
  const { downloadBaseUrl } = runtimeConfig();
  const releases = releasesPageUrl(downloadBaseUrl);

  return (
    <PageStack>
      <PageHeader
        eyebrow="Get started"
        title="Desktop agent"
        description="Install the agent on each computer you want to track. Time and activity appear here after its first upload."
      />

      <SignInDetails
        heading="What the agent asks for when you sign in"
        headingLevel={2}
        agentLink={false}
        email={isLive ? user.email || null : null}
      >
        Paste the <span className="font-medium text-foreground">server address</span> exactly as shown — the agent can&apos;t reach your
        workspace without it. Then sign in with the email and password you use here, and pick your workspace if you belong to more
        than one.
      </SignInDetails>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <AgentCard platform="windows" os={os} icon={Monitor} downloadBaseUrl={downloadBaseUrl}>
          <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
            <li>
              Run <span className="font-medium text-foreground">{AGENT_PLATFORMS.windows.assets[0].file}</span>. It installs for your
              Windows user only — no administrator rights needed — and starts when you sign in to Windows. If SmartScreen says{" "}
              <span className="font-medium text-foreground">“Windows protected your PC”</span>, choose{" "}
              <span className="font-medium text-foreground">More info → Run anyway</span>.
            </li>
            <li>
              Sign in with the <span className="font-medium text-foreground">server address</span>, email and password above. The
              agent lives in the system tray.
            </li>
            <li>Choose the project you&apos;re working on (only when you&apos;re on more than one) and start tracking.</li>
          </ol>
          <p className="text-xs text-muted-foreground">Windows asks for no special permissions. The project decides what is captured.</p>
        </AgentCard>

        <AgentCard platform="macos" os={os} icon={Laptop} downloadBaseUrl={downloadBaseUrl}>
          <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
            <li>
              Open the zip and move <span className="font-medium text-foreground">Dosi Tracker</span> to Applications. If macOS blocks
              the first launch, choose <span className="font-medium text-foreground">Open Anyway</span> in System Settings → Privacy
              &amp; Security.
            </li>
            <li>
              Sign in with the <span className="font-medium text-foreground">server address</span>, email and password above. The
              agent lives in the menu bar.
            </li>
            <li>
              Allow what macOS asks for, in <span className="font-medium text-foreground">System Settings → Privacy &amp; Security</span>:
            </li>
          </ol>
          <ul className="space-y-2 rounded-xl border border-border/70 bg-muted/40 p-3 text-xs text-muted-foreground">
            <li>
              <span className="font-medium text-foreground">Screen Recording</span>
              {" — "}for screenshots, when your project takes them.
            </li>
            <li>
              <span className="font-medium text-foreground">Input Monitoring</span>
              {" — "}to count keyboard and mouse activity (never what you type).
            </li>
            <li>
              <span className="font-medium text-foreground">Accessibility</span>
              {" — "}to read the active window&apos;s title, when your project records it.
            </li>
            <li>
              <span className="font-medium text-foreground">Camera</span>
              {" — "}only if your project turns on webcam photos.
            </li>
          </ul>
          <p className="text-xs text-muted-foreground">Skip a permission and only that part is left out — time is still tracked.</p>
        </AgentCard>

        <AgentCard platform="linux" os={os} icon={Terminal} downloadBaseUrl={downloadBaseUrl}>
          <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
            <li>
              Install the package for your distribution (<code className="font-mono text-xs text-foreground">{LINUX_COMMANDS.installDeb}</code>{" "}
              or the .rpm with dnf), or unpack the tarball and run its{" "}
              <code className="font-mono text-xs text-foreground">install.sh</code>, which needs no root.
            </li>
            <li>
              Start it with <code className="font-mono text-xs text-foreground">{LINUX_COMMANDS.enable}</code>, then sign in with{" "}
              <code className="font-mono text-xs text-foreground">dosi-tracker login</code> or the{" "}
              <span className="font-medium text-foreground">Dosi Tracker</span> app (
              <code className="font-mono text-xs text-foreground">{LINUX_COMMANDS.gui}</code>).
            </li>
            <li>Choose what you allow it to record, then your project, and start tracking.</li>
          </ol>
          <p className="text-xs text-muted-foreground">
            Runs as you, never as root. On Wayland, keyboard and mouse counts and window titles are not available and are reported as not
            measured; screenshots go through the desktop&apos;s screenshot permission.
          </p>
        </AgentCard>
      </div>

      <Card variant="quiet" className="flex flex-col gap-3 p-4 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
        <p>
          Check a download against the published <span className="font-medium text-foreground">SHA-256</span> checksums before you
          install it.
        </p>
        <div className="flex flex-wrap gap-3">
          <a href={checksumsUrl(downloadBaseUrl)} className="font-medium text-primary-strong hover:underline">
            Checksums (SHA256SUMS)
          </a>
          {releases && (
            <a href={releases} target="_blank" rel="noopener noreferrer" className="font-medium text-primary-strong hover:underline">
              Release notes and older versions<span className="sr-only"> (opens in a new tab)</span>
            </a>
          )}
        </div>
      </Card>

      <Card
        variant="quiet"
        className="flex flex-col items-start justify-between gap-4 border border-border/80 p-4 text-sm text-muted-foreground sm:flex-row sm:items-center"
      >
        <div className="flex items-start gap-3">
          <BookOpen aria-hidden className="mt-0.5 h-5 w-5 shrink-0 text-primary-strong" />
          <div>
            <p className="font-medium text-foreground">Rolling it out to a team?</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              The installation guides cover silent installs, setting the server for everyone, permissions and troubleshooting.
            </p>
          </div>
        </div>
        <Link
          href={INSTALL_GUIDES_PATH}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-primary/10 px-3 py-2 text-xs font-semibold text-primary-strong transition-colors hover:bg-primary/20"
        >
          Installation guides <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
        </Link>
      </Card>

      <Card variant="quiet" className="flex items-start gap-3 p-4 text-sm text-muted-foreground">
        <ShieldCheck aria-hidden className="mt-0.5 h-5 w-5 shrink-0" />
        <p>
          What the agent captures is decided per project by the workspace owner — see{" "}
          <Link href="/settings" className="text-primary-strong hover:underline">
            Settings → Privacy
          </Link>
          . Keystroke content is never recorded.
        </p>
      </Card>
    </PageStack>
  );
}

function AgentCard({
  platform,
  os,
  icon: Icon,
  downloadBaseUrl,
  children,
}: {
  platform: AgentPlatform;
  os: VisitorOs;
  icon: LucideIcon;
  downloadBaseUrl: string;
  children: React.ReactNode;
}) {
  const info = AGENT_PLATFORMS[platform];
  const recommended = os === platform;
  const [primary, ...others] = info.assets;
  return (
    <Card
      className={cn("flex flex-col", recommended && "border-primary/50 ring-2 ring-primary/30")}
      aria-labelledby={`agent-${platform}-title`}
      role="region"
    >
      <CardHeader>
        <div className="min-w-0">
          <CardTitle id={`agent-${platform}-title`} className="flex items-center gap-2 text-base">
            <Icon aria-hidden className="h-5 w-5 text-muted-foreground" /> {info.name}
          </CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">{info.requirement}</p>
        </div>
        {recommended && (
          <Badge tone="primary" className="gap-1">
            <Sparkles aria-hidden className="h-3 w-3" /> For this computer
          </Badge>
        )}
      </CardHeader>
      <CardContent className="flex flex-1 flex-col gap-4 text-sm">
        {children}
        <div className="mt-auto flex flex-col gap-2">
          <a href={releaseAssetUrl(downloadBaseUrl, primary.file)} className={linkButtonClass("primary", "md")}>
            <Download aria-hidden className="h-4 w-4" /> Download for {info.name}
            <span className="sr-only">
              {" "}
              ({primary.label}, {primary.file})
            </span>
          </a>
          <p className="text-center text-[11px] text-muted-foreground">
            <span className="font-mono">{primary.file}</span> · {primary.note}
          </p>
          {others.length > 0 && (
            <ul className="flex flex-col gap-1.5" aria-label={`Other ${info.name} downloads`}>
              {others.map((asset) => (
                <li key={asset.file}>
                  <a href={releaseAssetUrl(downloadBaseUrl, asset.file)} className={cn(linkButtonClass("outline", "sm"), "w-full")}>
                    <Download aria-hidden className="h-3.5 w-3.5" /> {asset.label}
                    <span className="sr-only"> ({asset.file})</span>
                  </a>
                  <p className="mt-0.5 text-center text-[11px] text-muted-foreground">{asset.note}</p>
                </li>
              ))}
            </ul>
          )}
          <Link
            href={info.guidePath}
            className="inline-flex items-center justify-center gap-1 pt-1 text-xs text-primary-strong hover:underline"
          >
            {info.name} setup guide <ArrowRight className="h-3 w-3" aria-hidden="true" />
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
