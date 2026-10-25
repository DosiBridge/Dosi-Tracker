vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { ErrorFallback } from "./error-fallback";

let consoleError: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  consoleError.mockRestore();
});

describe("ErrorFallback", () => {
  it("explains, offers Retry and a way back, and never shows the error's message", async () => {
    const retry = vi.fn();
    const user = userEvent.setup();
    render(<ErrorFallback error={new Error("secret internal detail")} retry={retry} boundary="dashboard" />);

    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Something went wrong" })).toHaveFocus();
    expect(screen.queryByText(/secret internal detail/)).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Go to dashboard" })).toHaveAttribute("href", "/dashboard");

    await user.click(screen.getByRole("button", { name: /try again/i }));
    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("shows the server's reference when there is one", () => {
    render(<ErrorFallback error={Object.assign(new Error("x"), { digest: "4242" })} retry={() => {}} boundary="host" />);
    expect(screen.getByText("4242")).toBeInTheDocument();
    expect(screen.getByText(/the reference below/)).toBeInTheDocument();
  });

  it("reports the error once, without the page's ids or query", () => {
    const error = new Error("x");
    const { rerender } = render(<ErrorFallback error={error} retry={() => {}} boundary="dashboard" />);
    rerender(<ErrorFallback error={error} retry={() => {}} boundary="dashboard" />);
    const reports = consoleError.mock.calls.filter((c: unknown[]) => c[0] === "[dosi-tracker] render error");
    expect(reports).toHaveLength(1);
    expect(reports[0][1]).toMatchObject({ kind: "Error", boundary: "dashboard" });
  });

  it("a11y: no axe violations", async () => {
    const { container } = render(<ErrorFallback error={new Error("x")} retry={() => {}} boundary="dashboard" />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
