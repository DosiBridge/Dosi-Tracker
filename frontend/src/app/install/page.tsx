import Link from "next/link";
import { ArrowRight, Download, Info, Laptop, Monitor, ShieldCheck, Terminal } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Code, FactTable } from "@/components/install/install-parts";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { AGENT_PLATFORMS, AGENT_PLATFORM_ORDER, releaseAssetUrl, type AgentPlatform } from "@/lib/agent-downloads";
import { runtimeConfig } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";

export const metadata = {
  title: "Desktop Agent Installation Guides | Dosi-Tracker",
  description:
    "Download and install the Dosi-Tracker desktop agent for Windows, macOS and Linux, and see what each platform lets it measure.",
};

const ICONS: Record<AgentPlatform, LucideIcon> = { windows: Monitor, macos: Laptop, linux: Terminal };

export default function InstallOverviewPage() {
  const { apiBaseUrl, downloadBaseUrl } = runtimeConfig();

  return (
    <div className="space-y-8">
      <div>
        <p className="mb-2 text-xs font-semibold uppercase tracking-wider text-primary-strong">Documentation &amp; setup</p>
        <h1 className="font-display text-3xl font-bold tracking-tight text-foreground sm:text-4xl">Desktop agent installation</h1>
        <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">
          Time and activity are recorded by a small agent on each computer. Pick your platform to download it and follow its guide.
        </p>
      </div>

      <Card variant="quiet" className="border-border/80 bg-card/60 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-success" aria-hidden="true" />
          <div className="space-y-1 text-xs text-muted-foreground">
            <h2 className="text-sm font-semibold text-foreground">Privacy by design</h2>
            <p>
              Agents never run as root or administrator and never record what you type: keyboard and mouse activity are counts only. Each
              workspace project decides what is captured, the agent can only capture less, and anything a platform can&apos;t measure is
              reported as not measured (<Code>null</Code>), never as a fabricated zero.
            </p>
          </div>
        </div>
      </Card>

      <div className="grid grid-cols-1 gap-6 md:grid-cols-3">
        {AGENT_PLATFORM_ORDER.map((platform) => {
          const info = AGENT_PLATFORMS[platform];
          const Icon = ICONS[platform];
          const primary = info.assets[0];
          const headingId = `install-${platform}-title`;
          return (
            <Card key={platform} role="region" aria-labelledby={headingId} className="flex flex-col border-border/80">
              <CardHeader className="pb-3">
                <CardTitle id={headingId} className="flex items-center gap-2 text-base font-semibold">
                  <Icon className="h-5 w-5 text-primary-strong" aria-hidden="true" /> {info.name}
                </CardTitle>
                <p className="mt-1 text-xs text-muted-foreground">{info.requirement}</p>
              </CardHeader>
              <CardContent className="flex flex-1 flex-col justify-between gap-4 text-xs text-muted-foreground">
                <ul className="list-disc space-y-1 pl-4">
                  {info.assets.map((asset) => (
                    <li key={asset.file}>
                      <span className="font-medium text-foreground">{asset.label}</span> — {asset.note}
                    </li>
                  ))}
                </ul>
                <div className="flex flex-col gap-2">
                  <a href={releaseAssetUrl(downloadBaseUrl, primary.file)} className={cn(linkButtonClass("primary", "md"), "w-full")}>
                    <Download className="h-4 w-4" aria-hidden="true" /> Download for {info.name}
                  </a>
                  <Link
                    href={info.guidePath}
                    className="inline-flex w-full items-center justify-center gap-1.5 rounded-lg bg-primary/10 py-2 text-xs font-medium text-primary-strong transition-colors hover:bg-primary/20"
                  >
                    {info.name} setup guide <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
                  </Link>
                </div>
              </CardContent>
            </Card>
          );
        })}
      </div>

      <section aria-labelledby="capabilities-title" className="space-y-4">
        <div>
          <h2 id="capabilities-title" className="font-display text-lg font-bold text-foreground">
            What each platform can measure
          </h2>
          <p className="mt-1 text-xs text-muted-foreground">Always subject to what the project records and what the person consents to.</p>
        </div>
        <FactTable
          caption="Agent capabilities by platform"
          headers={["Platform", "Activity and apps", "Screenshots and sign-in storage"]}
          rows={[
            [
              "Windows 10 and 11",
              "Key press and click counts, activity %, active window and apps, running programs",
              "Screenshots of the monitor you work on; sign-in encrypted with DPAPI",
            ],
            [
              "macOS 14+",
              "Counts with Input Monitoring, window titles with Accessibility, running apps",
              "Screenshots with Screen Recording; sign-in in the login Keychain",
            ],
            [
              "Linux on X11",
              "Key press and click counts, activity %, active window and apps, running programs",
              "Screenshots of the active monitor; sign-in in the desktop keyring",
            ],
            [
              "Linux on Wayland",
              "Time, idle and running programs; counts and window titles not measured (null)",
              "Screenshots through the desktop's screenshot permission; sign-in in the desktop keyring",
            ],
          ]}
        />
        <p className="text-xs text-muted-foreground">No agent needs root or administrator rights to run.</p>
      </section>

      <Card variant="quiet" className="border border-border/80 p-4 sm:p-5">
        <div className="flex items-start gap-3">
          <Info className="mt-0.5 h-5 w-5 shrink-0 text-primary-strong" aria-hidden="true" />
          <div className="space-y-2 text-xs text-muted-foreground">
            <h2 className="text-sm font-semibold text-foreground">Signing in to an agent</h2>
            <p>
              Agents ask for the <strong className="text-foreground">server address</strong>
              {apiBaseUrl ? (
                <>
                  {" "}
                  (for this site: <Code>{apiBaseUrl}</Code>)
                </>
              ) : null}
              , then your <strong className="text-foreground">email and password</strong> — the same ones you use on the web. If you belong
              to several workspaces, the agent lets you pick one after signing in.
            </p>
            <p>
              Signed-in members find everything to copy on the dashboard&apos;s{" "}
              <Link href="/download" className="font-medium text-primary-strong hover:underline">
                Download page
              </Link>
              .
            </p>
          </div>
        </div>
      </Card>
    </div>
  );
}
