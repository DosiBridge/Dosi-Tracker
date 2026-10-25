import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { SignInDetails, agentServerUrl, visitorOs } from "./agent-setup";
import { DEV_API_BASE_URL, RUNTIME_CONFIG_ELEMENT_ID, resetRuntimeConfigCache } from "@/lib/runtime-config";

/** What the root layout renders: this deployment's runtime configuration. */
function serveConfig(apiBaseUrl: string | null) {
  const el = document.createElement("script");
  el.type = "application/json";
  el.id = RUNTIME_CONFIG_ELEMENT_ID;
  el.textContent = JSON.stringify({ apiBaseUrl, downloadBaseUrl: "https://dl.example.com", demoMode: true, problems: [] });
  document.head.appendChild(el);
  resetRuntimeConfigCache();
}

afterEach(() => {
  document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.remove();
  resetRuntimeConfigCache();
});

const AGENT_SERVER_URL = DEV_API_BASE_URL;

describe("visitorOs", () => {
  it("recognises Windows, macOS and Linux desktops", () => {
    expect(visitorOs({ platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" })).toBe("windows");
    expect(visitorOs({ platform: "Windows", userAgent: "" })).toBe("windows");
    expect(visitorOs({ platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)", maxTouchPoints: 0 })).toBe("macos");
    expect(visitorOs({ platform: "macOS", userAgent: "" })).toBe("macos");
    expect(visitorOs({ platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; Linux x86_64)" })).toBe("linux");
    expect(visitorOs({ platform: "linux", userAgent: "" })).toBe("linux");
  });

  it("falls back to the user agent when the platform is empty", () => {
    expect(visitorOs({ platform: "", userAgent: "Mozilla/5.0 (Windows NT 10.0)" })).toBe("windows");
    expect(visitorOs({ platform: "", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15)" })).toBe("macos");
    expect(visitorOs({ platform: "", userAgent: "Mozilla/5.0 (X11; Ubuntu; Linux x86_64)" })).toBe("linux");
  });

  it("treats phones, tablets, iPads (a Mac platform that accepts touch) and Android as other", () => {
    expect(visitorOs({ platform: "iPhone", userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)" })).toBe("other");
    expect(visitorOs({ platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15)", maxTouchPoints: 5 })).toBe("other");
    expect(visitorOs({ platform: "Linux armv8l", userAgent: "Mozilla/5.0 (Linux; Android 14)" })).toBe("other");
    expect(visitorOs({})).toBe("unknown");
  });
});

describe("SignInDetails", () => {
  it("lists what the agent asks for, each copyable, and Copy all copies every labelled line", async () => {
    const user = userEvent.setup();
    render(
      <SignInDetails email="sam@realco.test" secret={{ label: <>One-time password</>, value: "Dt1!abc" }}>
        Share it securely.
      </SignInDetails>,
    );

    expect(screen.getByText(AGENT_SERVER_URL)).toBeInTheDocument();
    expect(screen.getByText("Dt1!abc")).toBeInTheDocument();
    expect(screen.getByText(/\/download$/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /copy server address/i }));
    expect(await navigator.clipboard.readText()).toBe(AGENT_SERVER_URL);

    await user.click(screen.getByRole("button", { name: /copy sign-in details/i }));
    const text = await navigator.clipboard.readText();
    expect(text.split("\n")).toEqual([
      `Server address: ${AGENT_SERVER_URL}`,
      "Email: sam@realco.test",
      "One-time password: Dt1!abc",
      `Desktop agent: ${window.location.origin}/download`,
    ]);
    expect(await screen.findAllByText("Copied")).not.toHaveLength(0);
  });

  it("leaves out what it doesn't have", () => {
    render(<SignInDetails agentLink={false} />);
    expect(screen.queryByText("Email")).not.toBeInTheDocument();
    expect(screen.queryByText("Desktop agent")).not.toBeInTheDocument();
  });

  it("the server address carries no trailing slash", () => {
    expect(agentServerUrl()?.endsWith("/")).toBe(false);
  });

  it("shows the address this deployment serves at runtime, not a build-time default", () => {
    serveConfig("https://tracker-api.example.com");
    render(<SignInDetails agentLink={false} />);
    expect(screen.getByText("https://tracker-api.example.com")).toBeInTheDocument();
  });

  it("says the address isn't set up (and offers nothing to copy) when the deployment names none", () => {
    serveConfig(null);
    render(<SignInDetails agentLink={false} />);
    expect(screen.getByText(/Not set up yet/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /copy server address/i })).not.toBeInTheDocument();
    expect(screen.queryByText(DEV_API_BASE_URL)).not.toBeInTheDocument();
  });

  it("a11y: the details list has no axe violations", async () => {
    const { container } = render(<SignInDetails email="sam@realco.test" secret={{ label: <>One-time password</>, value: "x" }} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
