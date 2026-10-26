import { ShieldCheck, Terminal } from "lucide-react";
import { Card } from "@/components/ui/card";
import { AssetDownloads, Code, CommandBlock, FactTable, GuideBreadcrumb, GuideSection } from "@/components/install/install-parts";
import { AGENT_PLATFORMS, LINUX_COMMANDS } from "@/lib/agent-downloads";
import { runtimeConfig } from "@/lib/runtime-config";

export const metadata = {
  title: "Linux Desktop Agent Guide | Dosi-Tracker",
  description:
    "Install the Dosi-Tracker agent on Linux from a .deb, .rpm or rootless tarball, sign in from the dosi-tracker command or the dosi-tracker-gui app, and see exactly what X11 and Wayland allow it to measure.",
};

export default function LinuxInstallPage() {
  const { apiBaseUrl, downloadBaseUrl } = runtimeConfig();
  const info = AGENT_PLATFORMS.linux;

  return (
    <div className="space-y-8">
      <div>
        <GuideBreadcrumb current="Linux" />
        <h1 className="flex items-center gap-2.5 font-display text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
          <Terminal className="h-7 w-7 text-primary-strong" aria-hidden="true" />
          Linux desktop agent guide
        </h1>
        <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">
          One <Code>dosi-tracker</Code> command runs the agent and controls it, and the optional <Code>dosi-tracker-gui</Code> app gives
          it a window; it shows itself in {info.home}. It always runs as you: never as root, never through input devices.
        </p>
        <p className="mt-2 text-xs text-muted-foreground">
          <span className="font-medium text-foreground">Requires:</span> {info.requirement}.
        </p>
      </div>

      <GuideSection title="Download">
        <AssetDownloads platform="linux" downloadBaseUrl={downloadBaseUrl} />
        <CommandBlock label="Check the download" commands={[LINUX_COMMANDS.verify]} />
      </GuideSection>

      <GuideSection title="Install">
        <p className="text-xs leading-relaxed text-muted-foreground">
          The packages install system-wide, so your package manager asks for your administrator password; the agent itself still runs as
          you. The tarball needs no root at all: it installs into <Code>~/.local</Code>.
        </p>
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <CommandBlock label="Ubuntu and Debian (.deb)" commands={[LINUX_COMMANDS.installDeb]} />
          <CommandBlock label="Fedora (.rpm)" commands={[LINUX_COMMANDS.installRpm]} />
        </div>
        <CommandBlock label="Any distribution (tarball, no root)" commands={LINUX_COMMANDS.installTarball(apiBaseUrl)} />
        <p className="text-xs text-muted-foreground">
          Installing never starts the agent or adds it to your login; you decide that in the next step.
        </p>
      </GuideSection>

      <GuideSection title="First run">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Start the agent (now and at every login), sign in, choose what it may record and your project. <Code>login</Code> asks for your
          email and password without echoing it, then lets you pick a workspace when you belong to more than one. Passwords are never
          accepted on the command line.
        </p>
        <CommandBlock
          label="Sign in and start tracking"
          commands={[
            LINUX_COMMANDS.enable,
            LINUX_COMMANDS.login(apiBaseUrl),
            LINUX_COMMANDS.consent,
            LINUX_COMMANDS.projectUse,
            LINUX_COMMANDS.status,
          ]}
        />
        <p className="text-xs leading-relaxed text-muted-foreground">
          Prefer a window? Open <span className="font-medium text-foreground">Dosi Tracker</span> from your applications menu, or run{" "}
          <Code>{LINUX_COMMANDS.gui}</Code>: it walks through the same sign-in, consent and project steps. Nothing is recorded until you
          grant consent; <Code>{LINUX_COMMANDS.consentTimeOnly}</Code> tracks time only.
        </p>
      </GuideSection>

      <GuideSection title="Everyday commands">
        <FactTable
          caption="Everyday dosi-tracker commands"
          headers={["Command", "What it does"]}
          rows={[
            [<Code key="status">{LINUX_COMMANDS.status}</Code>, "What is being tracked, today's totals, the upload queue and what this desktop can measure."],
            [<Code key="pause">{LINUX_COMMANDS.pause}</Code>, "Pause for a while (or until you resume)."],
            [<Code key="resume">{LINUX_COMMANDS.resume}</Code>, "Carry on tracking."],
            [<Code key="stop">{LINUX_COMMANDS.stop}</Code>, "Stop tracking; the agent keeps running."],
            [<Code key="sync">{LINUX_COMMANDS.sync}</Code>, "Upload waiting blocks now."],
            [<Code key="revoke">{LINUX_COMMANDS.revoke}</Code>, "Withdraw consent immediately (works offline)."],
            [<Code key="doctor">{LINUX_COMMANDS.doctor}</Code>, "Check the session, keyring, portal and tray without changing anything."],
            [<Code key="logout">{LINUX_COMMANDS.logout}</Code>, "Sign out; unsent blocks are kept for 14 days."],
          ]}
        />
      </GuideSection>

      <GuideSection title="What X11 and Wayland allow">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Wayland isolates applications from each other. The agent respects that instead of working around it, so what it cannot measure is
          reported as <span className="font-medium text-foreground">not measured (null)</span>, never as a fabricated zero.
        </p>
        <FactTable
          caption="Capture on X11 and Wayland"
          headers={["Capability", "X11", "Wayland (GNOME, KDE, wlroots)"]}
          rows={[
            ["Time blocks, lock, sleep and idle", "Yes", "Yes (idle from the desktop's idle monitor; manual mode where there is none)"],
            ["Keyboard and mouse counts, activity %", "Yes (counts only, never keys)", "Not measured (null)"],
            ["Active window and per-app time", "Yes", "Not measured (null)"],
            ["Running programs (names only, your own)", "Yes", "Yes"],
            ["Screenshots", "Yes", "Through the desktop's screenshot permission (xdg-desktop-portal)"],
            ["Webcam", "Not available yet", "Not available yet"],
          ]}
        />
      </GuideSection>

      <GuideSection title="Troubleshooting">
        <ul className="list-disc space-y-2 pl-5 text-xs leading-relaxed text-muted-foreground">
          <li>
            <span className="font-medium text-foreground">Start with</span> <Code>{LINUX_COMMANDS.doctor}</Code>: it reports the session
            type, the keyring, the screenshot portal, the tray and the service, and never changes anything.
          </li>
          <li>
            <span className="font-medium text-foreground">Sign-in storage</span> — the sign-in lives in your desktop keyring (Secret
            Service: GNOME Keyring, KWallet or KeePassXC). If the keyring is locked, unlock it when asked.
          </li>
          <li>
            <span className="font-medium text-foreground">No tray icon on GNOME</span> — install and enable the AppIndicator extension; the
            agent keeps a notification up instead so you always know it is tracking.
          </li>
          <li>
            <span className="font-medium text-foreground">Screenshots denied on Wayland</span> — allow Dosi Tracker when your desktop asks,
            or turn the permission back on in its privacy settings.
          </li>
        </ul>
      </GuideSection>

      <GuideSection title="Uninstall">
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
          <CommandBlock label="Tarball install" commands={[LINUX_COMMANDS.uninstallTarball]} />
          <CommandBlock label="Debian or Ubuntu" commands={[LINUX_COMMANDS.uninstallDeb]} />
          <CommandBlock label="Fedora" commands={[LINUX_COMMANDS.uninstallRpm]} />
        </div>
        <p className="text-xs text-muted-foreground">
          Uninstalling keeps your configuration and consent choices; <Code>dosi-tracker uninstall --purge</Code> also deletes them and any
          unsent blocks (it shows how many first).
        </p>
      </GuideSection>

      <Card variant="quiet" className="flex items-start gap-3 p-4 text-xs text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-success" aria-hidden="true" />
        <p>
          Never collected: what you type, which keys you press, the clipboard, pointer positions, background window titles, other users&apos;
          processes, the microphone or your files. The agent refuses to run as root and never reads input devices.
        </p>
      </Card>
    </div>
  );
}
