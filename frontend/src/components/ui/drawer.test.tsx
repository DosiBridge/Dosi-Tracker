// Drawer behavior from the user's side: hidden while closed; a named dialog
// with title and body while open; closes via Escape, its Close button, or a
// backdrop click; locks body scroll only while open.
import { describe, it, expect, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { Drawer } from "@/components/ui/drawer";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

/** Stateful host so close interactions actually remove the dialog, as in the app. */
function DrawerHost({ initialOpen = true }: { initialOpen?: boolean }) {
  const [open, setOpen] = useState(initialOpen);
  return (
    <Drawer open={open} onClose={() => setOpen(false)} title="Member details">
      <p>Drawer body content</p>
    </Drawer>
  );
}

function backdropOf(container: HTMLElement): HTMLElement {
  const el = container.querySelector(".animate-fade-in");
  if (!(el instanceof HTMLElement)) throw new Error("backdrop not rendered");
  return el;
}

describe("Drawer", () => {
  it("renders nothing while closed", () => {
    render(<DrawerHost initialOpen={false} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Drawer body content")).not.toBeInTheDocument();
  });

  it("when open, shows a named dialog with title and body", () => {
    render(<DrawerHost />);
    const dialog = screen.getByRole("dialog", { name: "Member details" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(within(dialog).getByRole("heading", { name: "Member details" })).toBeInTheDocument();
    expect(within(dialog).getByText("Drawer body content")).toBeInTheDocument();
  });

  it("Escape closes the drawer", async () => {
    const user = userEvent.setup();
    render(<DrawerHost />);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("the Close button closes the drawer", async () => {
    const user = userEvent.setup();
    render(<DrawerHost />);
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("clicking the backdrop closes; clicking inside the drawer does not", async () => {
    const user = userEvent.setup();
    const { container } = render(<DrawerHost />);
    await user.click(screen.getByText("Drawer body content"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.click(backdropOf(container));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("locks body scroll while open and releases it on close", async () => {
    const user = userEvent.setup();
    render(<DrawerHost />);
    expect(document.body.style.overflow).toBe("hidden");
    await user.keyboard("{Escape}");
    expect(document.body.style.overflow).toBe("");
  });

  it("a11y: the open drawer has no axe violations", async () => {
    const { container } = render(<DrawerHost />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
