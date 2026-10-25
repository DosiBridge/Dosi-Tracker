import Link from "next/link";
import { KeyRound, Laptop, ShieldCheck } from "lucide-react";
import { Card } from "@/components/ui/card";
import { AssetDownloads, Code, CommandBlock, FactTable, GuideBreadcrumb, GuideSection } from "@/components/install/install-parts";
import { AGENT_PLATFORMS, MACOS_COMMANDS, RELEASE_ASSET_FILES } from "@/lib/agent-downloads";
import { runtimeConfig } from "@/lib/runtime-config";

export const metadata = {
  title: "macOS Desktop Agent Guide | Dosi-Tracker",
  description:
    "Download and install the Dosi-Tracker menu-bar agent for macOS 14 and later: Gatekeeper, the privacy permissions and what each is for, Keychain, managed setup and troubleshooting.",
};

export default function MacOsInstallPage() {
  const { apiBaseUrl, downloadBaseUrl } = runtimeConfig();
  const info = AGENT_PLATFORMS.macos;

  return (
    <div className="space-y-8">
      <div>
        <GuideBreadcrumb current="macOS" />
        <h1 className="flex items-center gap-2.5 font-display text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
          <Laptop className="h-7 w-7 text-primary-strong" aria-hidden="true" />
          macOS desktop agent guide
        </h1>
        <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">
          A native Swift agent that lives in {info.home} (no Dock icon). One universal app runs on Apple silicon and Intel Macs; it never
          needs administrator rights.
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          <span className="font-medium text-foreground">Requires:</span> {info.requirement}.
        </p>
      </div>

      <GuideSection title="Download">
        <AssetDownloads platform="macos" downloadBaseUrl={downloadBaseUrl} />
        <CommandBlock label="Check the download (Terminal)" commands={[MACOS_COMMANDS.verify]} />
      </GuideSection>

      <GuideSection title="Install and sign in">
        <Card className="p-5 text-xs">
          <ol className="list-decimal space-y-3 pl-5 leading-relaxed text-muted-foreground">
            <li>
              Open <Code>{RELEASE_ASSET_FILES.macosZip}</Code> and move <span className="font-semibold text-foreground">Dosi Tracker</span>{" "}
              to <span className="font-semibold text-foreground">Applications</span> — macOS keeps your permission grants for an app that
              stays in one place.
            </li>
            <li>
              Open it. A build notarized with a Developer ID opens normally. If macOS says it was blocked, open{" "}
              <span className="font-semibold text-foreground">System Settings → Privacy &amp; Security</span>, find{" "}
              <em>&ldquo;Dosi Tracker&rdquo; was blocked</em> and choose <span className="font-semibold text-foreground">Open Anyway</span>{" "}
              (or Control-click the app in Finder and choose Open).
            </li>
            <li>
              Sign in with your <span className="font-semibold text-foreground">email</span> and{" "}
              <span className="font-semibold text-foreground">password</span>; the{" "}
              <span className="font-semibold text-foreground">server address</span> is under Advanced (signed-in members find it on the{" "}
              <Link href="/download" className="font-medium text-primary-strong hover:underline">
                Download page
              </Link>
              {apiBaseUrl ? (
                <>
                  ; for this site: <Code>{apiBaseUrl}</Code>
                </>
              ) : null}
              ).
            </li>
            <li>Pick your workspace if you belong to more than one, then your project, and tracking starts.</li>
          </ol>
        </Card>
      </GuideSection>

      <GuideSection title="Permissions, and why">
        <p className="text-xs leading-relaxed text-muted-foreground">
          The agent asks only for what your project records, the first time it needs it. Settings → Permissions in the app shows each one
          and opens the right pane of System Settings → Privacy &amp; Security.
        </p>
        <FactTable
          caption="macOS permissions"
          headers={["Permission", "Used for", "Without it"]}
          rows={[
            ["Screen Recording", "The screenshot of each block, when the project takes them.", "No screenshots; time is still tracked."],
            [
              "Input Monitoring",
              "Counting keyboard and mouse activity: when you typed or clicked, never what.",
              "Activity is reported honestly as unmeasured (null), never as a misleading 0%.",
            ],
            ["Accessibility", "The title of the window you work in, when the project records it.", "Apps are recorded by name only."],
            ["Camera", "One webcam photo per block, only if the project turns it on (off by default).", "No webcam photos."],
            ["Notifications", "Telling you tracking started, a project needs choosing or the sign-in expired.", "The menu bar still says it."],
          ]}
        />
      </GuideSection>

      <GuideSection title="Sign-in storage">
        <Card variant="quiet" className="space-y-1.5 p-4 text-xs text-muted-foreground">
          <p className="flex items-center gap-2 font-semibold text-foreground">
            <KeyRound className="h-4 w-4 text-primary-strong" aria-hidden="true" /> macOS Login Keychain
          </p>
          <p>
            The sign-in is kept in your login Keychain (item <em>Dosi Tracker sign-in</em>, service <Code>com.dosibridge.tracker</Code>,
            this Mac only); the password is never written anywhere else.
          </p>
        </Card>
      </GuideSection>

      <GuideSection title="Managed setup">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Set the server address for the signed-in macOS user, or push the same <Code>DosiApiBaseURL</Code> key to every Mac in an MDM
          configuration profile, so nobody has to type it:
        </p>
        <CommandBlock label="Set the server address" commands={[MACOS_COMMANDS.setServer(apiBaseUrl)]} />
      </GuideSection>

      <GuideSection title="Troubleshooting">
        <ul className="list-disc space-y-2 pl-5 text-xs leading-relaxed text-muted-foreground">
          <li>
            <span className="font-medium text-foreground">No screenshots</span> — allow Screen Recording and restart the app. If it is
            listed but still not working, reset the grant and allow it again:
          </li>
        </ul>
        <CommandBlock label="Reset the Screen Recording grant" commands={[MACOS_COMMANDS.resetScreenRecording]} />
        <ul className="list-disc space-y-2 pl-5 text-xs leading-relaxed text-muted-foreground">
          <li>
            <span className="font-medium text-foreground">Keyboard and mouse activity not measured</span> — allow Dosi Tracker in Input
            Monitoring; if it is on but still not measured, remove it with −, quit and reopen the app.
          </li>
          <li>
            <span className="font-medium text-foreground">Permissions reset after an update</span> — builds signed only ad hoc count as a
            new app each time; a Developer ID build keeps them.
          </li>
          <li>
            <span className="font-medium text-foreground">Where things are</span> — the offline queue is{" "}
            <Code>~/Library/Application Support/DosiTracker/dosi-tracker.db</Code>, logs are in <Code>~/Library/Logs/DosiTracker/</Code>.
            Uninstall by quitting the app and moving it to the Bin.
          </li>
        </ul>
      </GuideSection>

      <GuideSection title="Build from source">
        <p className="text-xs leading-relaxed text-muted-foreground">
          With macOS 14 and Xcode 15 or later, from a checkout of the repository (the script builds a universal app and its zip in{" "}
          <Code>.build/dist</Code>):
        </p>
        <CommandBlock label="Build and install the app" commands={MACOS_COMMANDS.build} />
      </GuideSection>

      <Card variant="quiet" className="flex items-start gap-3 p-4 text-xs text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-success" aria-hidden="true" />
        <p>Keystroke content is never recorded: only counts. What is captured is your workspace&apos;s choice, per project.</p>
      </Card>
    </div>
  );
}
