"use client";

import Link from "next/link";
import { Download, ExternalLink, Laptop, Monitor, ShieldCheck } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";

const REPO_URL = "https://github.com/DosiBridge/Dosi-Tracker";
const RELEASES_URL = `${REPO_URL}/releases`;
const MACOS_README_URL = `${REPO_URL}/tree/main/clients/macos#readme`;

/**
 * Where people get the desktop agent. Tracking only starts once an agent runs
 * on the member's computer, so the onboarding checklist points here.
 */
export default function DownloadPage() {
  const { workspace, isLive } = useSession();

  return (
    <PageStack>
      <PageHeader
        eyebrow="Get started"
        title="Desktop agent"
        description="Install the agent on each computer you want to track. Time and activity appear here after the first upload."
      />

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Monitor className="h-5 w-5 text-muted-foreground" /> Windows
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
              <li>
                Download <span className="font-medium text-foreground">dosi-tracker-setup.exe</span> from the latest release.
              </li>
              <li>Run it. It installs for your Windows user only — no administrator rights needed — and starts at sign-in.</li>
              <li>
                Sign in with your email and password
                {isLive ? (
                  <>
                    {" "}and the workspace name <span className="font-medium text-foreground">{workspace.name}</span>
                  </>
                ) : null}
                . The agent lives in the system tray.
              </li>
            </ol>
            <a href={RELEASES_URL} target="_blank" rel="noopener noreferrer" className="inline-block">
              <Button>
                <Download className="h-4 w-4" /> Download for Windows <ExternalLink className="h-3.5 w-3.5 opacity-70" />
              </Button>
            </a>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Laptop className="h-5 w-5 text-muted-foreground" /> macOS
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <p className="text-muted-foreground">
              The macOS agent is currently built from source (macOS 14+, Xcode 15+). A signed installer is on the way.
            </p>
            <ol className="list-decimal space-y-2 pl-5 text-muted-foreground">
              <li>Follow the build steps in the macOS agent&apos;s README.</li>
              <li>
                On first run, allow <span className="font-medium text-foreground">Screen Recording</span> and{" "}
                <span className="font-medium text-foreground">Accessibility</span> in System Settings → Privacy &amp; Security
                (and Camera only if a project enables webcam capture).
              </li>
            </ol>
            <a href={MACOS_README_URL} target="_blank" rel="noopener noreferrer" className="inline-block">
              <Button variant="outline">
                macOS setup guide <ExternalLink className="h-3.5 w-3.5 opacity-70" />
              </Button>
            </a>
          </CardContent>
        </Card>
      </div>

      <Card variant="quiet" className="flex items-start gap-3 p-4 text-sm text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0" />
        <p>
          What the agent captures is decided per project by the workspace owner — see{" "}
          <Link href="/settings" className="text-primary hover:underline">Settings → Privacy</Link>. Keystroke content is never recorded.
        </p>
      </Card>
    </PageStack>
  );
}
