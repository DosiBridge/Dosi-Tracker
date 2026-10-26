import { afterEach, describe, it, expect, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { renderWithProviders } from "@/test/harness";
import InstallLayout from "@/app/install/layout";
import InstallOverviewPage from "@/app/install/page";
import LinuxInstallPage from "@/app/install/linux/page";
import MacOsInstallPage from "@/app/install/macos/page";
import WindowsInstallPage from "@/app/install/windows/page";
import { DEFAULT_DOWNLOAD_BASE_URL, DEV_API_BASE_URL } from "@/lib/runtime-config";

const DOWNLOADS = DEFAULT_DOWNLOAD_BASE_URL;

/** Every line of every command block on the page. */
const commandLines = () => Array.from(document.querySelectorAll("pre code")).map((c) => c.textContent ?? "");

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("Public Installation & Onboarding Guides", () => {
  describe("InstallLayout", () => {
    it("renders navigation header, all OS links, accessible skip link, and security footer", () => {
      renderWithProviders(
        <InstallLayout>
          <div>Guide Content</div>
        </InstallLayout>,
      );

      const skipLink = screen.getByRole("link", { name: /skip to main content/i });
      expect(skipLink).toHaveAttribute("href", "#install-main-content");

      expect(screen.getByText("Dosi-Tracker")).toBeInTheDocument();
      expect(screen.getByText("Installation Guides")).toBeInTheDocument();

      const nav = screen.getByRole("navigation", { name: /operating system guides/i });
      expect(within(nav).getByRole("link", { name: /overview & matrix/i })).toHaveAttribute("href", "/install");
      expect(within(nav).getByRole("link", { name: /linux \(x11 & wayland\)/i })).toHaveAttribute("href", "/install/linux");
      expect(within(nav).getByRole("link", { name: /macos \(apple silicon & intel\)/i })).toHaveAttribute("href", "/install/macos");
      expect(within(nav).getByRole("link", { name: /windows desktop/i })).toHaveAttribute("href", "/install/windows");

      expect(screen.getByText(/Transparent Client Invariants: No root\/administrator requirements/i)).toBeInTheDocument();
      expect(screen.getByText(/Keystroke content is never recorded/i)).toBeInTheDocument();
    });
  });

  describe("InstallOverviewPage", () => {
    it("offers a download and a guide for every platform, from the released asset names", () => {
      render(<InstallOverviewPage />);
      expect(screen.getByRole("heading", { level: 1, name: /desktop agent installation/i })).toBeInTheDocument();

      const expected = {
        windows: ["Windows", "dosi-tracker-setup.exe"],
        macos: ["macOS", "DosiTracker-macos.zip"],
        linux: ["Linux", "dosi-tracker_amd64.deb"],
      } as const;
      for (const [platform, [name, file]] of Object.entries(expected)) {
        const card = screen.getByRole("region", { name: new RegExp(`^${name}$`, "i") });
        expect(within(card).getByRole("link", { name: new RegExp(`download for ${name}`, "i") })).toHaveAttribute("href", `${DOWNLOADS}/${file}`);
        expect(within(card).getByRole("link", { name: new RegExp(`${name} setup guide`, "i") })).toHaveAttribute("href", `/install/${platform}`);
      }
      // The Linux card lists every Linux package.
      const linux = screen.getByRole("region", { name: /^linux$/i });
      expect(within(linux).getByText(/Debian package/)).toBeInTheDocument();
      expect(within(linux).getByText(/RPM package/)).toBeInTheDocument();
      expect(within(linux).getByText(/Tarball/)).toBeInTheDocument();
    });

    it("states what each platform can measure, with Wayland's limits as not measured, and no internal test names", () => {
      render(<InstallOverviewPage />);
      const table = screen.getByRole("table", { name: /agent capabilities by platform/i });
      expect(within(table).getByText(/counts and window titles not measured \(null\)/i)).toBeInTheDocument();
      expect(within(table).getByText(/DPAPI/)).toBeInTheDocument();
      expect(within(table).getByText(/login Keychain/)).toBeInTheDocument();
      expect(table.textContent).not.toMatch(/FakeDriver|NOT_IMPLEMENTED|Planned/);
    });

    it("tells agents' users what to sign in with: no workspace name", () => {
      render(<InstallOverviewPage />);
      const box = screen.getByRole("heading", { name: /signing in to an agent/i }).parentElement!;
      expect(box.textContent).toContain(DEV_API_BASE_URL);
      expect(box.textContent).not.toMatch(/workspace name/i);
    });

    it("follows the deployment's download location", () => {
      vi.stubEnv("DOSI_DOWNLOAD_BASE_URL", "https://files.example.com/agents");
      render(<InstallOverviewPage />);
      expect(screen.getByRole("link", { name: /download for windows/i })).toHaveAttribute(
        "href",
        "https://files.example.com/agents/dosi-tracker-setup.exe",
      );
    });

    it("has no axe violations", async () => {
      const { container } = render(<InstallOverviewPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("LinuxInstallPage", () => {
    it("installs from the .deb, the .rpm or the rootless tarball, by their released names", () => {
      render(<LinuxInstallPage />);
      expect(screen.getByRole("heading", { level: 1, name: /linux desktop agent guide/i })).toBeInTheDocument();

      const downloads = screen.getByRole("list", { name: /linux downloads/i });
      expect(within(downloads).getByRole("link", { name: /debian package/i })).toHaveAttribute("href", `${DOWNLOADS}/dosi-tracker_amd64.deb`);
      expect(within(downloads).getByRole("link", { name: /rpm package/i })).toHaveAttribute("href", `${DOWNLOADS}/dosi-tracker.x86_64.rpm`);
      expect(within(downloads).getByRole("link", { name: /tarball/i })).toHaveAttribute("href", `${DOWNLOADS}/dosi-tracker-linux-x86_64.tar.gz`);
      expect(screen.getByRole("link", { name: "SHA256SUMS" })).toHaveAttribute("href", `${DOWNLOADS}/SHA256SUMS`);

      const lines = commandLines();
      expect(lines).toContain("sudo apt install ./dosi-tracker_amd64.deb");
      expect(lines).toContain("sudo dnf install ./dosi-tracker.x86_64.rpm");
      expect(lines).toContain("tar -xzf dosi-tracker-linux-x86_64.tar.gz");
      expect(lines).toContain(`./install.sh --server ${DEV_API_BASE_URL}`);
      expect(lines).toContain("sha256sum -c --ignore-missing SHA256SUMS");
    });

    it("walks through the real first-run CLI flow and names the GUI", () => {
      render(<LinuxInstallPage />);
      const lines = commandLines();
      for (const command of [
        "dosi-tracker service enable --now",
        `dosi-tracker login --server ${DEV_API_BASE_URL}`,
        "dosi-tracker consent grant --keyboard --mouse --active-window --running-programs --screenshots",
        'dosi-tracker project use "<project>"',
        "dosi-tracker status",
        "dosi-tracker uninstall",
        "sudo apt remove dosi-tracker",
        "sudo dnf remove dosi-tracker",
      ]) {
        expect(lines, command).toContain(command);
      }
      const everyday = screen.getByRole("table", { name: /everyday dosi-tracker commands/i });
      for (const command of ["dosi-tracker pause --for 30m", "dosi-tracker resume", "dosi-tracker stop", "dosi-tracker sync", "dosi-tracker doctor"]) {
        expect(within(everyday).getByText(command)).toBeInTheDocument();
      }
      expect(screen.getAllByText("dosi-tracker-gui").length).toBeGreaterThan(0);
      expect(screen.getByText(/Passwords are never accepted on the command line/i)).toBeInTheDocument();
    });

    it("is honest about Wayland: not measured (null), never zero, and no root or input-device workarounds", () => {
      render(<LinuxInstallPage />);
      const matrix = screen.getByRole("table", { name: /capture on x11 and wayland/i });
      const wayland = within(matrix).getByRole("rowheader", { name: /keyboard and mouse counts/i }).closest("tr")!;
      expect(within(wayland).getByText("Not measured (null)")).toBeInTheDocument();
      expect(within(matrix).getAllByText("Not available yet")).toHaveLength(2); // webcam
      expect(screen.getByText(/never as a fabricated zero/i)).toBeInTheDocument();
      expect(screen.getByText(/refuses to run as root and never reads input devices/i)).toBeInTheDocument();
      // Negative oracles: no stale "foundation" claims, no password on argv.
      expect(document.body.textContent).not.toMatch(/NOT_IMPLEMENTED|Headless Source Foundation|dosi-tracker-linux(?!-x86_64\.tar\.gz)/);
      expect(commandLines().join("\n")).not.toMatch(/--password(?!-stdin|-file)/);
    });

    it("has no axe violations", async () => {
      const { container } = render(<LinuxInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("MacOsInstallPage", () => {
    it("downloads the released zip and states the requirements", () => {
      render(<MacOsInstallPage />);
      expect(screen.getByRole("heading", { level: 1, name: /macos desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: /macos app \(zip\)/i })).toHaveAttribute("href", `${DOWNLOADS}/DosiTracker-macos.zip`);
      expect(screen.getAllByText(/macOS 14 Sonoma or later, Apple silicon or Intel/i).length).toBeGreaterThan(0);
      expect(commandLines()).toContain("shasum -a 256 DosiTracker-macos.zip");
    });

    it("explains Gatekeeper, every permission and the Keychain", () => {
      render(<MacOsInstallPage />);
      expect(screen.getByText(/Open Anyway/)).toBeInTheDocument();
      const permissions = screen.getByRole("table", { name: /macos permissions/i });
      for (const name of ["Screen Recording", "Input Monitoring", "Accessibility", "Camera", "Notifications"]) {
        expect(within(permissions).getByRole("rowheader", { name })).toBeInTheDocument();
      }
      expect(within(permissions).getByText(/reported honestly as unmeasured \(null\)/i)).toBeInTheDocument();
      expect(screen.getByText(/macOS Login Keychain/i)).toBeInTheDocument();
      expect(screen.getAllByText(/com\.dosibridge\.tracker/).length).toBeGreaterThan(0);
      expect(commandLines()).toContain("tccutil reset ScreenCapture com.dosibridge.tracker");
      expect(commandLines()).toContain(`defaults write com.dosibridge.tracker DosiApiBaseURL "${DEV_API_BASE_URL}"`);
    });

    it("builds from source with the steps that really work", () => {
      render(<MacOsInstallPage />);
      const lines = commandLines();
      expect(lines).toContain("bash scripts/build-app.sh");
      expect(lines).toContain('cp -R ".build/dist/Dosi Tracker.app" /Applications/');
      // The old, broken steps are gone.
      expect(lines.join("\n")).not.toMatch(/\.\/scripts\/build-app\.sh|\.build\/artifacts/);
    });

    it("has no axe violations", async () => {
      const { container } = render(<MacOsInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("WindowsInstallPage", () => {
    it("downloads the released installer and explains SmartScreen and sign-in", () => {
      render(<WindowsInstallPage />);
      expect(screen.getByRole("heading", { level: 1, name: /windows desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: /windows installer/i })).toHaveAttribute("href", `${DOWNLOADS}/dosi-tracker-setup.exe`);
      expect(screen.getAllByText(/Windows 10 \(1809 or newer\) or Windows 11/).length).toBeGreaterThan(0);
      expect(screen.getByText("Run anyway")).toBeInTheDocument();
      expect(screen.getAllByText(DEV_API_BASE_URL).length).toBeGreaterThan(0);
    });

    it("documents silent deployment with this site's server address", () => {
      render(<WindowsInstallPage />);
      expect(commandLines()).toContain(`dosi-tracker-setup.exe /S /SERVER=${DEV_API_BASE_URL}`);
      const switches = screen.getByRole("table", { name: /installer switches/i });
      for (const name of ["/S", "/SERVER=<url>", "/DASHBOARD=<url>", "/D=<folder>"]) {
        expect(within(switches).getByRole("rowheader", { name })).toBeInTheDocument();
      }
    });

    it("names the real data locations, DPAPI, quitting and uninstalling", () => {
      render(<WindowsInstallPage />);
      const files = screen.getByRole("table", { name: /agent files on windows/i });
      expect(within(files).getByText(String.raw`%LOCALAPPDATA%\DosiTracker\dosi-tracker.db`)).toBeInTheDocument();
      expect(within(files).getByText(/CryptProtectData/)).toBeInTheDocument();
      expect(document.body.textContent).not.toContain("queue.db");
      const lines = commandLines();
      expect(lines).toContain(String.raw`"%LOCALAPPDATA%\Programs\DosiTracker\dosi-tracker.exe" --quit`);
      expect(lines).toContain(String.raw`"%LOCALAPPDATA%\Programs\DosiTracker\uninstall.exe" /S /KEEPDATA`);
    });

    it("says when this site has no server address configured instead of inventing one", () => {
      vi.stubEnv("NODE_ENV", "production");
      vi.stubEnv("DOSI_API_URL", "");
      vi.stubEnv("NEXT_PUBLIC_API_URL", "");
      vi.stubEnv("DOSI_DEMO_MODE", "");
      render(<WindowsInstallPage />);
      expect(commandLines()).toContain("dosi-tracker-setup.exe /S /SERVER=<server address>");
      expect(screen.queryByText(DEV_API_BASE_URL)).not.toBeInTheDocument();
    });

    it("has no axe violations", async () => {
      const { container } = render(<WindowsInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });
});
