import Link from "next/link";
import { Lock, Monitor, ShieldCheck } from "lucide-react";
import { Card } from "@/components/ui/card";
import { AssetDownloads, Code, CommandBlock, FactTable, GuideBreadcrumb, GuideSection } from "@/components/install/install-parts";
import { AGENT_PLATFORMS, RELEASE_ASSET_FILES, WINDOWS_COMMANDS } from "@/lib/agent-downloads";
import { runtimeConfig } from "@/lib/runtime-config";

export const metadata = {
  title: "Windows Desktop Agent Guide | Dosi-Tracker",
  description:
    "Download and install the Dosi-Tracker agent for Windows 10 and 11: per-user install without administrator rights, sign-in, silent deployment, data locations and uninstall.",
};

export default function WindowsInstallPage() {
  const { apiBaseUrl, downloadBaseUrl } = runtimeConfig();
  const info = AGENT_PLATFORMS.windows;

  return (
    <div className="space-y-8">
      <div>
        <GuideBreadcrumb current="Windows" />
        <h1 className="flex items-center gap-2.5 font-display text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
          <Monitor className="h-7 w-7 text-primary-strong" aria-hidden="true" />
          Windows desktop agent guide
        </h1>
        <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">
          A small Rust agent that lives in {info.home}, records work in time blocks on the project you choose and uploads them to your
          workspace. It installs for your Windows user only and never needs administrator rights.
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          <span className="font-medium text-foreground">Requires:</span> {info.requirement}.
        </p>
      </div>

      <GuideSection title="Download">
        <AssetDownloads platform="windows" downloadBaseUrl={downloadBaseUrl} />
        <CommandBlock label="Check the download (PowerShell)" commands={[WINDOWS_COMMANDS.verify]} />
      </GuideSection>

      <GuideSection title="Install and sign in">
        <Card className="p-5 text-xs">
          <ol className="list-decimal space-y-3 pl-5 leading-relaxed text-muted-foreground">
            <li>
              Run <Code>{RELEASE_ASSET_FILES.windowsInstaller}</Code>. It installs to <Code>%LOCALAPPDATA%\Programs\DosiTracker</Code>,
              adds a Start menu shortcut and starts the agent when you sign in to Windows.
            </li>
            <li>
              If Windows SmartScreen says <span className="font-semibold text-foreground">&ldquo;Windows protected your PC&rdquo;</span>,
              choose <span className="font-semibold text-foreground">More info</span>, then{" "}
              <span className="font-semibold text-foreground">Run anyway</span>.
            </li>
            <li>
              Sign in with the <span className="font-semibold text-foreground">server address</span>, your{" "}
              <span className="font-semibold text-foreground">email</span> and <span className="font-semibold text-foreground">password</span>.
              The server address is under <span className="font-semibold text-foreground">Advanced</span>; signed-in members find it on
              the dashboard&apos;s <Link href="/download" className="font-medium text-primary-strong hover:underline">Download page</Link>
              {apiBaseUrl ? (
                <>
                  {" "}
                  (for this site: <Code>{apiBaseUrl}</Code>)
                </>
              ) : null}
              .
            </li>
            <li>
              If you belong to more than one workspace, pick the one to track for; then choose your project (only asked when you are on
              more than one) and start tracking.
            </li>
          </ol>
        </Card>
      </GuideSection>

      <GuideSection title="Silent and managed deployment">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Deploy the installer to many computers without prompts and point every agent at your server, so nobody types the address:
        </p>
        <CommandBlock label="Silent install" commands={[WINDOWS_COMMANDS.silentInstall(apiBaseUrl)]} />
        <FactTable
          caption="Installer switches"
          headers={["Switch", "Effect"]}
          rows={[
            [<Code key="s">/S</Code>, "Silent install."],
            [
              <Code key="server">/SERVER=&lt;url&gt;</Code>,
              <>
                The server address the agent signs in to, written to <Code>config.toml</Code> next to the program. An invalid address stops
                the installer (exit code 2).
              </>,
            ],
            [<Code key="dash">/DASHBOARD=&lt;url&gt;</Code>, "The web dashboard the agent links to."],
            [<Code key="d">/D=&lt;folder&gt;</Code>, "Another install folder (must be the last switch)."],
          ]}
        />
      </GuideSection>

      <GuideSection title="Where things are kept">
        <FactTable
          caption="Agent files on Windows"
          headers={["Where", "What"]}
          rows={[
            [<Code key="p">%LOCALAPPDATA%\Programs\DosiTracker\</Code>, "The program, its config.toml (when the installer wrote one) and uninstall.exe."],
            [
              <Code key="c">%APPDATA%\DosiTracker\credentials.bin</Code>,
              "Your sign-in, encrypted with Windows DPAPI (CryptProtectData) for your Windows user only.",
            ],
            [<Code key="q">%LOCALAPPDATA%\DosiTracker\dosi-tracker.db</Code>, "The offline queue: blocks and screenshots waiting to upload."],
            [<Code key="l">%LOCALAPPDATA%\DosiTracker\logs\</Code>, "Daily logs, kept 7 days."],
          ]}
        />
        <Card variant="quiet" className="space-y-1.5 p-4 text-xs text-muted-foreground">
          <p className="flex items-center gap-2 font-semibold text-foreground">
            <Lock className="h-4 w-4 text-primary-strong" aria-hidden="true" /> Your password is never stored in clear text
          </p>
          <p>The sign-in is protected by Windows DPAPI and tied to your Windows account; it is never written to environment variables.</p>
        </Card>
      </GuideSection>

      <GuideSection title="Quit, update and uninstall">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Quitting saves and uploads the block in progress. Installing a new version asks the running agent to quit the same way first.
        </p>
        <CommandBlock label="Quit gracefully" commands={[WINDOWS_COMMANDS.quit]} />
        <p className="text-xs leading-relaxed text-muted-foreground">
          Uninstall from <span className="font-medium text-foreground">Settings → Apps → Installed apps → Dosi Tracker</span>, or run{" "}
          <Code>%LOCALAPPDATA%\Programs\DosiTracker\uninstall.exe</Code>. It asks whether to delete your data (sign-in, preferences, logs
          and time not uploaded yet). A silent uninstall deletes it; keep it with:
        </p>
        <CommandBlock label="Silent uninstall, keeping data" commands={[WINDOWS_COMMANDS.silentUninstallKeepData]} />
      </GuideSection>

      <Card variant="quiet" className="flex items-start gap-3 p-4 text-xs text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-success" aria-hidden="true" />
        <p>
          Windows asks desktop apps for no special permissions. The agent counts key presses and clicks — never which keys or what you
          type — and captures only what your workspace&apos;s project allows. Camera photos need{" "}
          <span className="font-medium text-foreground">Settings → Privacy &amp; security → Camera</span> and are off unless a project
          turns them on.
        </p>
      </Card>
    </div>
  );
}
