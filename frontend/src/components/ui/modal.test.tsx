// Modal behavior from the user's side: nothing renders while closed; when open
// it announces itself as a named dialog showing title/description/body; it
// closes via Escape, the Close button, or a backdrop click (but NOT a click
// inside); and it locks body scroll only while open.
import { describe, it, expect, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { Modal } from "@/components/ui/modal";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

/** Stateful host so close interactions actually remove the dialog, as in the app. */
function ModalHost({ initialOpen = true }: { initialOpen?: boolean }) {
  const [open, setOpen] = useState(initialOpen);
  return (
    <Modal
      open={open}
      onClose={() => setOpen(false)}
      title="Remove member"
      description="This cannot be undone."
    >
      <p>Modal body content</p>
    </Modal>
  );
}

function backdropOf(container: HTMLElement): HTMLElement {
  // The backdrop is the only element with the fade-in animation; it has no
  // semantic role by design (it is the click-outside target).
  const el = container.querySelector(".animate-fade-in");
  if (!(el instanceof HTMLElement)) throw new Error("backdrop not rendered");
  return el;
}

describe("Modal", () => {
  it("renders nothing while closed", () => {
    render(<ModalHost initialOpen={false} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Modal body content")).not.toBeInTheDocument();
  });

  it("when open, shows a named dialog with title, description and body", () => {
    render(<ModalHost />);
    const dialog = screen.getByRole("dialog", { name: "Remove member" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(within(dialog).getByRole("heading", { name: "Remove member" })).toBeInTheDocument();
    expect(within(dialog).getByText("This cannot be undone.")).toBeInTheDocument();
    expect(within(dialog).getByText("Modal body content")).toBeInTheDocument();
  });

  it("Escape closes the dialog", async () => {
    const user = userEvent.setup();
    render(<ModalHost />);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("the Close button closes the dialog", async () => {
    const user = userEvent.setup();
    render(<ModalHost />);
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("clicking the backdrop closes; clicking inside the dialog does not", async () => {
    const user = userEvent.setup();
    const { container } = render(<ModalHost />);
    await user.click(screen.getByText("Modal body content"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.click(backdropOf(container));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("locks body scroll while open and releases it on close", async () => {
    const user = userEvent.setup();
    render(<ModalHost />);
    expect(document.body.style.overflow).toBe("hidden");
    await user.keyboard("{Escape}");
    expect(document.body.style.overflow).toBe("");
  });

  it("a11y: the open modal has no axe violations", async () => {
    const { container } = render(<ModalHost />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
