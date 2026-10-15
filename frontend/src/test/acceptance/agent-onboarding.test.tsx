// Acceptance spec — Feature: Connecting a desktop agent.
// Gherkin documentation: src/test/acceptance/features/agent-onboarding.feature
//
// Business rule under test: a member can connect the desktop agent with only
// what the web app shows them — the server address (agents can't guess it),
// and their email — and is pointed at the agent for their computer.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import DownloadPage from "@/app/(dashboard)/download/page";
import { Sidebar } from "@/components/layout/sidebar";
import { CommandPalette } from "@/components/command-palette";
import { AGENT_SERVER_URL } from "@/components/dashboard/agent-setup";
import { renderAsRole, renderWithProviders, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;
let platformSpy: ReturnType<typeof vi.spyOn> | null = null;

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: "OK", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  platformSpy?.mockRestore();
  platformSpy = null;
  vi.unstubAllGlobals();
});

function browseFrom(platform: string) {
  platformSpy = vi.spyOn(window.navigator, "platform", "get").mockReturnValue(platform);
}

describe("Feature: Connecting a desktop agent", () => {
  describe("Scenario: The Download page gives the sign-in details", () => {
    it("Given a live member When they open Download Then the server address and email are shown (no workspace), each copyable", async () => {
      localStorage.setItem("dosi-token", "jwt-live");
      fetchMock.mockImplementation(async (input) => {
        const url = String(input).replace(API, "");
        if (url.startsWith("/api/abp/application-configuration")) {
          return json({
            currentUser: { isAuthenticated: true, id: "u-1", tenantId: "t-1", userName: "sam", name: "Sam", surName: "Lee", email: "sam@realco.test", roles: [] },
            currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
            auth: { grantedPolicies: {} },
          });
        }
        if (url.startsWith("/api/app/workspace/")) return json({}, 404);
        if (url.startsWith("/api/app/")) return json({ items: [], totalCount: 0 });
        throw new Error(`unexpected fetch: ${url}`);
      });
      const user = userEvent.setup();
      renderWithProviders(<DownloadPage />, { route: "/download" });

      const details = (await screen.findByRole("heading", { name: /what the agent asks for/i })).closest("section")!;
      expect(within(details).getByText(AGENT_SERVER_URL)).toBeInTheDocument();
      expect(within(details).queryByText("RealCo")).not.toBeInTheDocument();
      expect(await within(details).findByText("sam@realco.test")).toBeInTheDocument();

      await user.click(within(details).getByRole("button", { name: /copy server address/i }));
      expect(await navigator.clipboard.readText()).toBe(AGENT_SERVER_URL);
    });
  });

  describe("Scenario: The agent for this computer is recommended", () => {
    it("Given a member on a Mac When they open Download Then the macOS card is marked for this computer", () => {
      browseFrom("MacIntel");
      renderAsRole(<DownloadPage />, "worker", { route: "/download" });

      const mac = screen.getByRole("region", { name: /macos/i });
      expect(within(mac).getByText("For this computer")).toBeInTheDocument();
      expect(within(screen.getByRole("region", { name: /windows/i })).queryByText("For this computer")).not.toBeInTheDocument();
    });

    it("Given a member on Windows When they open Download Then the Windows card is marked for this computer", () => {
      browseFrom("Win32");
      renderAsRole(<DownloadPage />, "worker", { route: "/download" });

      expect(within(screen.getByRole("region", { name: /windows/i })).getByText("For this computer")).toBeInTheDocument();
    });
  });

  describe("Scenario: Every permission says what it is for", () => {
    it("Given the Download page Then macOS names each permission with its purpose, and the words are spaced correctly", () => {
      renderAsRole(<DownloadPage />, "worker", { route: "/download" });

      const mac = screen.getByRole("region", { name: /macos/i });
      const text = mac.textContent ?? "";
      expect(text).toContain("Screen Recording — for screenshots");
      expect(text).toContain("Accessibility — to count keyboard and mouse activity");
      expect(text).toContain("Camera — only if your project turns on webcam photos");
      expect(text).not.toMatch(/Accessibilityin/);
      expect(text).toContain("System Settings → Privacy & Security");
    });

    it("Given the Download page Then Windows names the installer and needs no special permissions", () => {
      renderAsRole(<DownloadPage />, "worker", { route: "/download" });

      const windows = screen.getByRole("region", { name: /windows/i });
      expect(within(windows).getByText("dosi-tracker-setup-<version>.exe")).toBeInTheDocument();
      expect(within(windows).getByText(/asks for no special permissions/i)).toBeInTheDocument();
      expect(within(windows).getByRole("link", { name: /download for windows/i })).toHaveAttribute("rel", expect.stringContaining("noopener"));
    });

    it("a11y: the Download page has no axe violations", async () => {
      const { container } = renderAsRole(<DownloadPage />, "worker", { route: "/download" });
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("Scenario: Members can always find the agent", () => {
    it("Given a worker Then the sidebar links the Desktop agent page", () => {
      renderAsRole(<Sidebar collapsed={false} onToggle={() => {}} mobileOpen={false} onMobileClose={() => {}} />, "worker");
      expect(screen.getByRole("link", { name: "Desktop agent" })).toHaveAttribute("href", "/download");
    });

    it("Given a worker Then the command palette offers the Desktop agent page", () => {
      renderAsRole(<CommandPalette />, "worker");
      fireEvent.keyDown(window, { key: "k", ctrlKey: true });
      expect(screen.getByRole("option", { name: /desktop agent/i })).toBeInTheDocument();
    });
  });
});
