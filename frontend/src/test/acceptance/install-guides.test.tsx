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
    it("renders comparison cards with truthful status: Linux Headless Foundation, Windows Source & NSIS", () => {
      render(<InstallOverviewPage />);

      expect(screen.getByRole("heading", { level: 1, name: /desktop agent installation & capabilities/i })).toBeInTheDocument();

      // Linux Card
      const linuxHeading = screen.getByRole("heading", { level: 2, name: /^linux$/i });
      const linuxCard = linuxHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(linuxCard as HTMLElement).getByText("Headless Foundation")).toBeInTheDocument();
      expect(within(linuxCard as HTMLElement).getByText(/Headless foundation crate in clients\/linux/i)).toBeInTheDocument();
      expect(within(linuxCard as HTMLElement).getByRole("link", { name: /linux foundation & specification/i })).toHaveAttribute("href", "/install/linux");
      // Negative oracle: must NOT claim "Source Preview" or pre-built binaries
      expect(within(linuxCard as HTMLElement).queryByText(/^Source Preview$/i)).not.toBeInTheDocument();

      // macOS Card
      const macHeading = screen.getByRole("heading", { level: 2, name: /^macos$/i });
      const macCard = macHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(macCard as HTMLElement).getByText(/Source Build Only/i)).toBeInTheDocument();
      expect(within(macCard as HTMLElement).getByText(/Apple Silicon \(arm64\) and Intel \(x86_64\)/i)).toBeInTheDocument();
      expect(within(macCard as HTMLElement).getByRole("link", { name: /macos setup guide/i })).toHaveAttribute("href", "/install/macos");

      // Windows Card
      const winHeading = screen.getByRole("heading", { level: 2, name: /^windows$/i });
      const winCard = winHeading.closest(".card-elev, [class*='rounded']")!;
      expect(within(winCard as HTMLElement).getByText(/Source Crate & NSIS Script/i)).toBeInTheDocument();
      expect(within(winCard as HTMLElement).getByText(/pre-built binaries are unreleased/i)).toBeInTheDocument();
      expect(within(winCard as HTMLElement).getByRole("link", { name: /windows setup guide/i })).toHaveAttribute("href", "/install/windows");
      // Negative oracle: must NOT claim pre-built installer
      expect(within(winCard as HTMLElement).queryByText(/^Pre-Built Installer$/i)).not.toBeInTheDocument();
    });

    it("renders capability matrix with truthful Linux Foundation status and Windows compilation status", () => {
      render(<InstallOverviewPage />);

      const table = screen.getByRole("table");
      expect(table).toBeInTheDocument();

      // Linux rows must explicitly say Source Foundation and FOUNDATION ONLY
      expect(within(table).getAllByText(/Source Foundation \(clients\/linux\)/i).length).toBe(2);
      expect(within(table).getAllByText(/FOUNDATION ONLY \(Sensor-Free Crate; 101 Tests Passed on Host; Native Runtime Unverified\)/i).length).toBe(2);

      // Windows row must show Compilation Only
      expect(within(table).getByText(/COMPILATION ONLY/i)).toBeInTheDocument();

      // Negative oracle: table must NOT label pre-built NSIS executable
      expect(within(table).queryByText("Pre-Built Setup EXE (NSIS)")).not.toBeInTheDocument();

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
    it("clearly discloses client is headless foundation with doctor command and consent core", () => {
      render(<LinuxInstallPage />);

      expect(screen.getByRole("heading", { level: 1, name: /linux desktop agent foundation & specification/i })).toBeInTheDocument();
      expect(screen.getByText("Headless Source Foundation (No Native Collection / Sync Yet)")).toBeInTheDocument();
      expect(screen.getByText("Status: Headless Source Foundation (Diagnostic CLI & Consent Core)")).toBeInTheDocument();
      expect(screen.getByText(/first finite headless Linux implementation slice in/i)).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: /Implemented Diagnostic Command Syntax/i })).toBeInTheDocument();
      expect(screen.getByText("cargo run --bin dosi-tracker-linux -- doctor")).toBeInTheDocument();
      expect(screen.getByText("cargo run --bin dosi-tracker-linux -- doctor --json")).toBeInTheDocument();

      // Rootless installer lifecycle commands
      expect(screen.getByRole("heading", { name: /Rootless Source-Build Installation Lifecycle/i })).toBeInTheDocument();
      expect(screen.getByText("bash clients/linux/scripts/install.sh install")).toBeInTheDocument();
      expect(screen.getByText("bash clients/linux/scripts/install.sh update")).toBeInTheDocument();
      expect(screen.getByText("bash clients/linux/scripts/install.sh rollback")).toBeInTheDocument();
      expect(screen.getByText("bash clients/linux/scripts/uninstall.sh")).toBeInTheDocument();
      expect(screen.getByText("~/.local/bin/dosi-tracker-linux doctor [--json]")).toBeInTheDocument();
    });

    it("details planned Wayland portal consent and anti-keylogger invariants", () => {
      render(<LinuxInstallPage />);

      // Wayland ScreenCast planned spec
      expect(screen.getByText(/org.freedesktop.portal.ScreenCast/i)).toBeInTheDocument();
      expect(screen.getByText(/interactive OS dialog/i)).toBeInTheDocument();
      expect(screen.getByText(/Global Input Activity: UNSUPPORTED \(null\)/i)).toBeInTheDocument();
      expect(screen.getByText(/Active Window Title: UNSUPPORTED \(null\)/i)).toBeInTheDocument();

      // Unprivileged security policy
      expect(screen.getByRole("heading", { name: /Unprivileged Execution Invariant \(Anti-Keylogger Policy\)/i })).toBeInTheDocument();
      expect(screen.getByText(/exclusively as an unprivileged user process/i)).toBeInTheDocument();
      expect(screen.getByText(/unrestricted hardware keylogger/i)).toBeInTheDocument();
      expect(screen.getByText(/refuses to implement or recommend evdev workarounds/i)).toBeInTheDocument();

      // Roadmap
      expect(screen.getByRole("heading", { name: /Linux Implementation Roadmap & Finite Next Slices/i })).toBeInTheDocument();
      expect(screen.getByText(/Slice 1 \(Delivered\):/i)).toBeInTheDocument();
      expect(screen.getByText(/Slice 2 \(Next\):/i)).toBeInTheDocument();
    });

    it("does not present fake CLI commands, operational setup, or uninstallation scripts (Negative Oracles)", () => {
      render(<LinuxInstallPage />);

      // Negative Oracles: Asserting absence of fabricated capabilities
      expect(screen.queryByText(/dosi-tracker pause/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/dosi-tracker resume/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/dosi-tracker stop/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/dosi-tracker status/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/Implemented through org\.freedesktop\.portal/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/rm -rf ~\/\\.config\/dosi-tracker/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/api_base_url = "https:\/\/tracker-api\.example\.com"/i)).not.toBeInTheDocument();
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
    it("states Source Crate & NSIS script status, unreleased pre-built binaries, SmartScreen, and DPAPI", () => {
      render(<WindowsInstallPage />);

      expect(screen.getByRole("heading", { level: 1, name: /windows desktop agent guide/i })).toBeInTheDocument();
      expect(screen.getByText(/Source Crate & NSIS Build Script/i)).toBeInTheDocument();
      expect(screen.getByText(/Pre-built release binary installers are/i)).toBeInTheDocument();
      expect(screen.getByText(/cargo check --bin dosi-tracker/i)).toBeInTheDocument();
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

      // Negative oracle: must NOT claim pre-built installer
      expect(screen.queryByText(/^Pre-Built Installer \(NSIS\)$/i)).not.toBeInTheDocument();
    });

    it("has no axe violations", async () => {
      const { container } = render(<WindowsInstallPage />);
      expect(await axe(container)).toHaveNoViolations();
    });
  });
});
