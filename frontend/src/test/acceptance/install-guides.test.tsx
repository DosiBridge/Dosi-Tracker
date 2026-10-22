import { describe, it, expect } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { renderWithProviders } from "@/test/harness";
import InstallLayout from "@/app/install/layout";
import InstallOverviewPage from "@/app/install/page";
import LinuxInstallPage from "@/app/install/linux/page";
import MacOsInstallPage from "@/app/install/macos/page";
import WindowsInstallPage from "@/app/install/windows/page";

describe("Public Installation & Onboarding Guides", () => {
  describe("InstallLayout", () => {
    it("renders navigation header, all OS links, accessible skip link, and security footer", () => {
      renderWithProviders(
        <InstallLayout>
          <div>Guide Content</div>
        </InstallLayout>
      );

      // Skip link
      const skipLink = screen.getByRole("link", { name: /skip to main content/i });
      expect(skipLink).toHaveAttribute("href", "#install-main-content");

      // Brand
      expect(screen.getByText("Dosi-Tracker")).toBeInTheDocument();
      expect(screen.getByText("Installation Guides")).toBeInTheDocument();

      // Navigation links
      const nav = screen.getByRole("navigation", { name: /operating system guides/i });
      expect(within(nav).getByRole("link", { name: /overview & matrix/i })).toHaveAttribute("href", "/install");
      expect(within(nav).getByRole("link", { name: /linux \(x11 & wayland\)/i })).toHaveAttribute("href", "/install/linux");
      expect(within(nav).getByRole("link", { name: /macos \(apple silicon & intel\)/i })).toHaveAttribute("href", "/install/macos");
      expect(within(nav).getByRole("link", { name: /windows desktop/i })).toHaveAttribute("href", "/install/windows");

      // Footer
      expect(screen.getByText(/Transparent Client Invariants: No root\/administrator requirements/i)).toBeInTheDocument();
      expect(screen.getByText(/Keystroke content is never recorded/i)).toBeInTheDocument();
    });
  });

  describe("InstallOverviewPage", () => {
    it("renders comparison cards for all 3 OS platforms with truthful release status", () => {
      render(<InstallOverviewPage />);

      expect(screen.getByRole("heading", { level: 1, name: /desktop agent installation & capability guides/i })).toBeInTheDocument();

      // Linux Card
      const linuxHeading = screen.getByRole("heading", { level: 2, name: /^linux$/i });
      const linuxCard = linuxHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(linuxCard as HTMLElement).getByText(/Source Preview/i)).toBeInTheDocument();
      expect(within(linuxCard as HTMLElement).getByText(/Ubuntu 22.04\+, Fedora 38\+, Arch/i)).toBeInTheDocument();
      expect(within(linuxCard as HTMLElement).getByRole("link", { name: /linux setup guide/i })).toHaveAttribute("href", "/install/linux");

      // macOS Card
      const macHeading = screen.getByRole("heading", { level: 2, name: /^macos$/i });
      const macCard = macHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(macCard as HTMLElement).getByText(/Source Build Only/i)).toBeInTheDocument();
      expect(within(macCard as HTMLElement).getByText(/Apple Silicon \(arm64\) and Intel \(x86_64\)/i)).toBeInTheDocument();
      expect(within(macCard as HTMLElement).getByRole("link", { name: /macos setup guide/i })).toHaveAttribute("href", "/install/macos");

      // Windows Card
      const winHeading = screen.getByRole("heading", { level: 2, name: /^windows$/i });
      const winCard = winHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(winCard as HTMLElement).getByText(/Pre-Built Installer/i)).toBeInTheDocument();
      expect(within(winCard as HTMLElement).getByText(/Windows 10 \(1809\+\) or Windows 11/i)).toBeInTheDocument();
      expect(within(winCard as HTMLElement).getByRole("link", { name: /windows setup guide/i })).toHaveAttribute("href", "/install/windows");
    });

    it("renders capability matrix with Wayland null reporting and zero admin privileges", () => {
      render(<InstallOverviewPage />);

      const table = screen.getByRole("table");
      expect(table).toBeInTheDocument();

      // Check Wayland row
      expect(within(table).getByText("Linux (Wayland)")).toBeInTheDocument();
      const unsupportedCells = within(table).getAllByText(/UNSUPPORTED \(Reported as null\)/i);
      expect(unsupportedCells.length).toBeGreaterThanOrEqual(2); // input and window title

      // Check zero admin privileges across all
      const noAdminCells = within(table).getAllByText(/No \(/i);
      expect(noAdminCells.length).toBe(4); // Linux X11, Linux Wayland, macOS, Windows
    });

    it("has no axe violations", async () => {
      const { container } = render(<InstallOverviewPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("LinuxInstallPage", () => {
    it("states unreleased status, supported distributions, and architectures", () => {
      render(<LinuxInstallPage />);

      expect(screen.getByRole("heading", { level: 1, name: /linux desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByText(/Pre-built Binaries Unreleased/i)).toBeInTheDocument();
      expect(screen.getByText(/currently unreleased/i)).toBeInTheDocument();
      expect(screen.getByText(/x86_64:/i)).toBeInTheDocument();
      expect(screen.getByText(/aarch64:/i)).toBeInTheDocument();
      expect(screen.getByText(/Ubuntu 22.04 LTS/i)).toBeInTheDocument();
      expect(screen.getByText(/Fedora Workstation/i)).toBeInTheDocument();
    });

    it("details Wayland portal consent and strictly prohibits root/sudo/evdev escalation", () => {
      render(<LinuxInstallPage />);

      // Wayland ScreenCast
      expect(screen.getByText(/org.freedesktop.portal.ScreenCast/i)).toBeInTheDocument();
      expect(screen.getByText(/interactive OS dialog/i)).toBeInTheDocument();
      expect(screen.getByText(/Global Input Activity: UNSUPPORTED \(null\)/i)).toBeInTheDocument();
      expect(screen.getByText(/Active Window Title: UNSUPPORTED \(null\)/i)).toBeInTheDocument();

      // Unprivileged security policy
      expect(screen.getByRole("heading", { name: /Unprivileged Execution Invariant \(Anti-Keylogger Policy\)/i })).toBeInTheDocument();
      expect(screen.getByText(/exclusively as an unprivileged user process/i)).toBeInTheDocument();
      expect(screen.getByText(/unrestricted hardware keylogger/i)).toBeInTheDocument();
      expect(screen.getByText(/refuses to implement or recommend evdev workarounds/i)).toBeInTheDocument();
    });

    it("provides first-run config.toml, Secret Service storage, and CLI controls", () => {
      render(<LinuxInstallPage />);

      expect(screen.getAllByText(/~\/\.config\/dosi-tracker\/config\.toml/i).length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText(/api_base_url = "https:\/\/tracker-api\.example\.com"/i)).toBeInTheDocument();
      expect(screen.getByText(/Freedesktop Secret Service D-Bus API/i)).toBeInTheDocument();
      expect(screen.getByText(/dosi-tracker pause --minutes 15/i)).toBeInTheDocument();
      expect(screen.getByText(/echo \$XDG_SESSION_TYPE/i)).toBeInTheDocument();
    });

    it("has no axe violations", async () => {
      const { container } = render(<LinuxInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("MacOsInstallPage", () => {
    it("states Sonoma/Sequoia requirements, Apple Silicon & Intel support, and source build status", () => {
      render(<MacOsInstallPage />);

      expect(screen.getByRole("heading", { level: 1, name: /macos desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByText(/Source Build Only \(Xcode 15\+\)/i)).toBeInTheDocument();
      expect(screen.getByText(/Apple Silicon:/i)).toBeInTheDocument();
      expect(screen.getByText(/Intel:/i)).toBeInTheDocument();
      expect(screen.getAllByText(/macOS 14 Sonoma/i).length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText(/Developer ID Notarization Status/i)).toBeInTheDocument();
    });

    it("provides step-by-step Gatekeeper approval and TCC privacy permissions", () => {
      render(<MacOsInstallPage />);

      // Gatekeeper steps
      expect(screen.getByText(/First Launch: Approving in macOS Gatekeeper/i)).toBeInTheDocument();
      expect(screen.getByText(/Open Anyway/i)).toBeInTheDocument();

      // TCC permissions table
      expect(screen.getByText("Screen Recording")).toBeInTheDocument();
      expect(screen.getByText("Input Monitoring")).toBeInTheDocument();
      expect(screen.getByText(/reported honestly as unmeasured \(null\)/i)).toBeInTheDocument();
      expect(screen.getByText("Accessibility")).toBeInTheDocument();

      // Keychain
      expect(screen.getByText(/macOS Login Keychain/i)).toBeInTheDocument();
      expect(screen.getAllByText(/com\.dosibridge\.tracker/i).length).toBeGreaterThanOrEqual(1);

      // Troubleshooting
      expect(screen.getByText(/tccutil reset ScreenCapture com\.dosibridge\.tracker/i)).toBeInTheDocument();
    });

    it("has no axe violations", async () => {
      const { container } = render(<MacOsInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("WindowsInstallPage", () => {
    it("states Windows 10/11 x86_64, NSIS package, SmartScreen handling, and DPAPI protection", () => {
      render(<WindowsInstallPage />);

      expect(screen.getByRole("heading", { level: 1, name: /windows desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByText(/Pre-Built Installer \(NSIS\)/i)).toBeInTheDocument();
      expect(screen.getByText(/Version 1809 or newer/i)).toBeInTheDocument();
      expect(screen.getAllByText(/Windows 11/i).length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText(/x86_64-pc-windows-msvc/i)).toBeInTheDocument();

      // SmartScreen
      expect(screen.getByText(/Windows SmartScreen Approval/i)).toBeInTheDocument();
      expect(screen.getByText(/Run anyway/i)).toBeInTheDocument();

      // Silent install
      expect(screen.getByText(/Silent Enterprise & Managed Deployment/i)).toBeInTheDocument();
      expect(screen.getAllByText(/\/S/).length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText(/\/SERVER=<url>/)).toBeInTheDocument();

      // DPAPI
      expect(screen.getByText(/Windows Data Protection API \(DPAPI\) Keying/i)).toBeInTheDocument();
      expect(screen.getByText(/CryptProtectData/i)).toBeInTheDocument();

      // Graceful quit & uninstall
      expect(screen.getByText(/dosi-tracker\.exe --quit/i)).toBeInTheDocument();
      expect(screen.getByText(/%LOCALAPPDATA%\\Programs\\DosiTracker\\uninstall\.exe/i)).toBeInTheDocument();
    });

    it("has no axe violations", async () => {
      const { container } = render(<WindowsInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });
});
