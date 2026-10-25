// Every route segment has its own error boundary, so a crash keeps the shell up
// and offers Retry plus a way back (components/error-fallback renders the card).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import DashboardError from "@/app/(dashboard)/error";
import HostError from "@/app/(host)/error";
import InstallError from "@/app/install/error";
import LoginError from "@/app/login/error";
import WorkspacesError from "@/app/workspaces/error";
import InviteError from "@/app/invite/[token]/error";
import GlobalError from "@/app/global-error";

let consoleError: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  consoleError.mockRestore();
});

describe("segment error boundaries", () => {
  it.each([
    ["workspace pages", DashboardError, "/dashboard"],
    ["the console", HostError, "/host"],
    ["the install guides", InstallError, "/install"],
    ["sign-in", LoginError, "/login"],
    ["the workspace chooser", WorkspacesError, "/workspaces"],
    ["invitations", InviteError, "/login"],
  ])("%s offer Retry and a way back to %s", async (_label, Boundary, home) => {
    const retry = vi.fn();
    const user = userEvent.setup();
    render(<Boundary error={new Error("x")} retry={retry} />);
    expect(screen.getByRole("heading", { name: "Something went wrong" })).toBeInTheDocument();
    expect(screen.getAllByRole("link").map((a) => a.getAttribute("href"))).toContain(home);
    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(retry).toHaveBeenCalled();
  });
});

describe("global error boundary", () => {
  it("brings its own document and still offers Retry", async () => {
    const retry = vi.fn();
    const user = userEvent.setup();
    // React warns about <html> inside a test container; the markup is what matters here.
    render(<GlobalError error={new Error("x")} retry={retry} />);
    expect(screen.getByRole("heading", { name: "Something went wrong" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Reload Dosi-Tracker" })).toHaveAttribute("href", "/");
    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(retry).toHaveBeenCalled();
  });
});
