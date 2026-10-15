"use client";

import Link from "next/link";
import { Download, ExternalLink, Laptop, Monitor, ShieldCheck, Sparkles } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { linkButtonClass } from "@/components/dashboard/link-button";
import {
  AGENT_RELEASES_URL,
  MACOS_GUIDE_URL,
  SignInDetails,
  useVisitorOs,
  type VisitorOs,
} from "@/components/dashboard/agent-setup";
import { cn } from "@/lib/utils";

/**
 * Where people get the desktop agent. Tracking only starts once an agent runs
 * on the member's computer, so the onboarding checklist, the sidebar and the
 * invite result point here. The agent can't guess the server it belongs to,
 * so the page shows the address to paste into its sign-in screen.
 */
export default function DownloadPage() {
  const { user, isLive } = useSession();
  const os = useVisitorOs();

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
        workspace without it. Then sign in with the email and password you use here.
      </SignInDetails>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <AgentCard
          platform="windows"
          os={os}
          icon={Monitor}
          title="Windows"
          requirement="Windows 10 or 11"
          action={
            <a href={AGENT_RELEASES_URL} target="_blank" rel="noopener noreferrer" className={linkButtonClass("primary", "md")}>
              <Download aria-hidden className="h-4 w-4" /> Download for Windows <ExternalLink aria-hidden className="h-3.5 w-3.5 opacity-70" />
              <span className="sr-only"> (opens in a new tab)</span>
            </a>
          }
        >
          <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
            <li>
              Download <span className="font-medium text-foreground">dosi-tracker-setup-&lt;version&gt;.exe</span> from the latest release.
            </li>
            <li>
              Run it. It installs for your Windows user only — no administrator rights needed — and starts when you sign in to
              Windows. If SmartScreen says <span className="font-medium text-foreground">“Windows protected your PC”</span>, choose{" "}
              <span className="font-medium text-foreground">More info → Run anyway</span>.
            </li>
            <li>
              Sign in with the <span className="font-medium text-foreground">server address</span>, email and password above.
              The agent lives in the system tray.
            </li>
            <li>Choose the project you&apos;re working on (only when you&apos;re on more than one) and start tracking.</li>
          </ol>
          <p className="text-xs text-muted-foreground">
            Windows asks for no special permissions. The project decides what is captured.
          </p>
        </AgentCard>

        <AgentCard
          platform="macos"
          os={os}
          icon={Laptop}
          title="macOS"
          requirement="macOS 14 Sonoma or later"
          action={
            <a href={MACOS_GUIDE_URL} target="_blank" rel="noopener noreferrer" className={linkButtonClass("outline", "md")}>
              macOS setup guide <ExternalLink aria-hidden className="h-3.5 w-3.5 opacity-70" />
              <span className="sr-only"> (opens in a new tab)</span>
            </a>
          }
        >
          <p className="text-muted-foreground">
            The macOS agent is built from source for now (Xcode 15 or later); a signed installer is on the way. The setup guide walks
            through it.
          </p>
          <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
            <li>Build and start the agent by following the setup guide.</li>
            <li>
              Sign in with the <span className="font-medium text-foreground">server address</span>, email and password above.
              The agent lives in the menu bar.
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
              <span className="font-medium text-foreground">Accessibility</span>
              {" — "}to count keyboard and mouse activity (never what you type) and to read the active window&apos;s title.
            </li>
            <li>
              <span className="font-medium text-foreground">Camera</span>
              {" — "}only if your project turns on webcam photos.
            </li>
          </ul>
          <p className="text-xs text-muted-foreground">Skip a permission and only that part is left out — time is still tracked.</p>
        </AgentCard>
      </div>

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
  title,
  requirement,
  action,
  children,
}: {
  platform: Exclude<VisitorOs, "other" | "unknown">;
  os: VisitorOs;
  icon: LucideIcon;
  title: string;
  requirement: string;
  action: React.ReactNode;
  children: React.ReactNode;
}) {
  const recommended = os === platform;
  return (
    <Card
      className={cn("flex flex-col", recommended && "border-primary/50 ring-2 ring-primary/30")}
      aria-labelledby={`agent-${platform}-title`}
      role="region"
    >
      <CardHeader>
        <div className="min-w-0">
          <CardTitle id={`agent-${platform}-title`} className="flex items-center gap-2 text-base">
            <Icon aria-hidden className="h-5 w-5 text-muted-foreground" /> {title}
          </CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">{requirement}</p>
        </div>
        {recommended && (
          <Badge tone="primary" className="gap-1">
            <Sparkles aria-hidden className="h-3 w-3" /> For this computer
          </Badge>
        )}
      </CardHeader>
      <CardContent className="flex flex-1 flex-col gap-4 text-sm">
        {children}
        <div className="mt-auto">{action}</div>
      </CardContent>
    </Card>
  );
}
