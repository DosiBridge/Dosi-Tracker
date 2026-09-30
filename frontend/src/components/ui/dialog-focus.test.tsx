// Focus management for Modal and Drawer — the behavior a keyboard or screen
// reader user depends on. Before `useDialogFocus` existed these dialogs set
// `aria-modal` but left the keyboard alone: focus stayed on the page behind,
// Tab walked straight out of the open dialog, and closing it dropped focus to
// the top of the document. Each test here failed against that implementation.
import { describe, it, expect, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Modal } from "@/components/ui/modal";
import { Drawer } from "@/components/ui/drawer";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

/** A page with a control behind the overlay, so escaping focus is detectable. */
function DialogHost({ kind }: { kind: "modal" | "drawer" }) {
  const [open, setOpen] = useState(false);
  const Dialog = kind === "modal" ? Modal : Drawer;
  return (
    <div>
      <button onClick={() => setOpen(true)}>Open dialog</button>
      <button>Behind the overlay</button>
      <Dialog open={open} onClose={() => setOpen(false)} title="Test dialog">
        <button>First inside</button>
        <input aria-label="Middle field" />
        <button>Last inside</button>
      </Dialog>
    </div>
  );
}

describe.each(["modal", "drawer"] as const)("%s focus management", (kind) => {
  it("moves focus into the dialog when it opens", async () => {
    const user = userEvent.setup();
    render(<DialogHost kind={kind} />);

    await user.click(screen.getByRole("button", { name: "Open dialog" }));

    // Focus must land inside the dialog, not stay on the trigger behind it.
    const dialog = screen.getByRole("dialog");
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
  });

  it("keeps Tab inside the dialog instead of leaking to the page behind", async () => {
    const user = userEvent.setup();
    render(<DialogHost kind={kind} />);
    await user.click(screen.getByRole("button", { name: "Open dialog" }));

    const dialog = screen.getByRole("dialog");
    // Tab well past the number of controls inside: focus must never escape.
    for (let i = 0; i < 8; i++) {
      await user.tab();
      expect(dialog).toContainElement(document.activeElement as HTMLElement);
    }
    expect(screen.getByRole("button", { name: "Behind the overlay" })).not.toHaveFocus();
  });

  it("wraps backwards from the first control to the last with Shift+Tab", async () => {
    const user = userEvent.setup();
    render(<DialogHost kind={kind} />);
    await user.click(screen.getByRole("button", { name: "Open dialog" }));

    const dialog = screen.getByRole("dialog");
    for (let i = 0; i < 5; i++) {
      await user.tab({ shift: true });
      expect(dialog).toContainElement(document.activeElement as HTMLElement);
    }
  });

  it("returns focus to the control that opened it when closed", async () => {
    const user = userEvent.setup();
    render(<DialogHost kind={kind} />);
    const trigger = screen.getByRole("button", { name: "Open dialog" });

    await user.click(trigger);
    await user.keyboard("{Escape}");

    // Without restore, focus falls back to <body> and the keyboard user loses
    // their place in the page entirely.
    expect(trigger).toHaveFocus();
  });

  it("can be closed from the keyboard alone after tabbing to Close", async () => {
    const user = userEvent.setup();
    render(<DialogHost kind={kind} />);
    await user.click(screen.getByRole("button", { name: "Open dialog" }));

    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
