// Popover from the user's side: a button opens a named panel that sits above
// the page (portaled to <body>, above the sidebar), takes focus, keeps Tab
// inside, and closes on Escape, a press outside, the button, or its own
// close — handing focus back to the button. Panels stack: Escape closes only
// the newest, and one opened inside a dialog closes without closing the dialog.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { Modal } from "@/components/ui/modal";
import { Popover } from "@/components/ui/popover";
import { FLOATING_LAYER_SELECTOR } from "@/components/ui/floating";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** A popover holding a small form, plus a control elsewhere on the page. */
function FormPopover({ onApply = () => {} }: { onApply?: () => void }) {
  return (
    <div>
      <Popover
        label="Custom range"
        className="w-72 p-3"
        trigger={(props, { open }) => (
          <button {...props}>
            Custom {open ? "(open)" : ""}
          </button>
        )}
      >
        {({ close }) => (
          <div>
            <label htmlFor="from">From</label>
            <input id="from" />
            <label htmlFor="to">To</label>
            <input id="to" />
            <button
              type="button"
              onClick={() => {
                onApply();
                close();
              }}
            >
              Apply
            </button>
          </div>
        )}
      </Popover>
      <button type="button">Elsewhere</button>
    </div>
  );
}

const trigger = () => screen.getByRole("button", { name: /^Custom/ });
const panel = () => screen.queryByRole("dialog", { name: "Custom range" });

describe("Popover", () => {
  it("is closed until its button is pressed, and says so", () => {
    render(<FormPopover />);
    expect(panel()).toBeNull();
    expect(trigger()).toHaveAttribute("aria-haspopup", "dialog");
    expect(trigger()).toHaveAttribute("aria-expanded", "false");
    expect(trigger()).not.toHaveAttribute("aria-controls");
  });

  it("opens a named panel portaled to <body>, layered above the sidebar, with focus inside", async () => {
    const user = userEvent.setup();
    const { container } = render(<FormPopover />);
    await user.click(trigger());
    const dialog = panel();
    expect(dialog).not.toBeNull();
    expect(container.contains(dialog)).toBe(false);
    expect(dialog!.parentElement).toBe(document.body);
    // The sidebar is z-50; the panel must sit above it.
    expect(dialog!.className).toContain("z-[60]");
    expect(dialog).toBeVisible();
    expect(dialog!.matches(FLOATING_LAYER_SELECTOR)).toBe(true);
    expect(trigger()).toHaveAttribute("aria-expanded", "true");
    expect(trigger()).toHaveAttribute("aria-controls", dialog!.id);
    expect(screen.getByLabelText("From")).toHaveFocus();
  });

  it("Escape closes it and returns focus to the button", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    await user.click(trigger());
    await user.keyboard("{Escape}");
    expect(panel()).toBeNull();
    expect(trigger()).toHaveFocus();
    expect(trigger()).toHaveAttribute("aria-expanded", "false");
  });

  it("a press outside closes it and leaves focus where the press landed; a press inside does not", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    await user.click(trigger());
    await user.click(screen.getByLabelText("To"));
    expect(panel()).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Elsewhere" }));
    expect(panel()).toBeNull();
    expect(screen.getByRole("button", { name: "Elsewhere" })).toHaveFocus();
  });

  it("pressing its button again closes it", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    await user.click(trigger());
    await user.click(trigger());
    expect(panel()).toBeNull();
  });

  it("close() from inside closes it and hands focus back to the button", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(<FormPopover onApply={onApply} />);
    await user.click(trigger());
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalledTimes(1);
    expect(panel()).toBeNull();
    expect(trigger()).toHaveFocus();
  });

  it("keeps Tab inside the open panel, both ways", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    await user.click(trigger());
    await user.tab();
    expect(screen.getByLabelText("To")).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Apply" })).toHaveFocus();
    await user.tab();
    expect(screen.getByLabelText("From")).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Apply" })).toHaveFocus();
  });

  it("opens from the keyboard (Enter) like any button", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    trigger().focus();
    await user.keyboard("{Enter}");
    expect(panel()).not.toBeNull();
    expect(screen.getByLabelText("From")).toHaveFocus();
  });

  it("can be controlled by its owner", async () => {
    const user = userEvent.setup();
    const changes: boolean[] = [];
    function Controlled() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <Popover
            label="Filters"
            open={open}
            onOpenChange={(next) => {
              changes.push(next);
              setOpen(next);
            }}
            trigger={(props) => <button {...props}>Filters</button>}
          >
            <p>Filter body</p>
          </Popover>
          <button type="button" onClick={() => setOpen(true)}>
            Open from outside
          </button>
        </>
      );
    }
    render(<Controlled />);
    await user.click(screen.getByRole("button", { name: "Open from outside" }));
    const filters = screen.getByRole("dialog", { name: "Filters" });
    // Nothing focusable inside: the panel itself holds focus.
    expect(filters).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Filters" })).toBeNull();
    expect(changes).toEqual([false]);
    await user.click(screen.getByRole("button", { name: "Filters" }));
    expect(changes).toEqual([false, true]);
  });

  it("an owner closing it directly still hands focus back to its button", async () => {
    const user = userEvent.setup();
    function Owner() {
      const [open, setOpen] = useState(false);
      return (
        <Popover label="Rename" open={open} onOpenChange={setOpen} trigger={(props) => <button {...props}>Rename</button>}>
          <input aria-label="New name" />
          <button type="button" onClick={() => setOpen(false)}>
            Save
          </button>
        </Popover>
      );
    }
    render(<Owner />);
    await user.click(screen.getByRole("button", { name: "Rename" }));
    expect(screen.getByLabelText("New name")).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.queryByRole("dialog", { name: "Rename" })).toBeNull();
    expect(screen.getByRole("button", { name: "Rename" })).toHaveFocus();
  });

  it("still shows the panel (and closes on Escape) when the trigger's id was overridden", async () => {
    const user = userEvent.setup();
    render(
      <Popover label="Help" trigger={(props) => <button {...props} id="my-own-id">Help</button>}>
        <p>Help body</p>
      </Popover>,
    );
    await user.click(screen.getByRole("button", { name: "Help" }));
    expect(screen.getByRole("dialog", { name: "Help" })).toBeVisible();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Help" })).toBeNull();
  });

  it("is named by its button when it has no label of its own", async () => {
    const user = userEvent.setup();
    render(
      <Popover trigger={(props) => <button {...props}>Columns</button>}>
        <p>Choose columns</p>
      </Popover>,
    );
    await user.click(screen.getByRole("button", { name: "Columns" }));
    expect(screen.getByRole("dialog", { name: "Columns" })).toBeInTheDocument();
  });

  it("a11y: the open panel has no axe violations", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    await user.click(trigger());
    expect(await axe(document.body)).toHaveNoViolations();
  });
});

describe("Popover — stacking", () => {
  function Nested() {
    return (
      <Popover label="Outer" trigger={(props) => <button {...props}>Outer</button>}>
        <p>Outer body</p>
        <Popover label="Inner" trigger={(props) => <button {...props}>Inner</button>}>
          <p>Inner body</p>
        </Popover>
      </Popover>
    );
  }

  it("Escape closes only the newest panel; a press inside a panel opened from another keeps both", async () => {
    const user = userEvent.setup();
    render(<Nested />);
    await user.click(screen.getByRole("button", { name: "Outer" }));
    await user.click(screen.getByRole("button", { name: "Inner" }));
    await user.click(screen.getByText("Inner body"));
    expect(screen.getByRole("dialog", { name: "Inner" })).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Outer" })).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull();
    expect(screen.getByRole("dialog", { name: "Outer" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Inner" })).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Outer" })).toBeNull();
    expect(screen.getByRole("button", { name: "Outer" })).toHaveFocus();
  });

  it("inside a dialog: Escape closes the panel but not the dialog, and Tab stays in the panel", async () => {
    const user = userEvent.setup();
    function InDialog() {
      const [open, setOpen] = useState(true);
      return (
        <Modal open={open} onClose={() => setOpen(false)} title="Edit member">
          <button type="button">Save</button>
          <FormPopover />
        </Modal>
      );
    }
    render(<InDialog />);
    const dialog = screen.getByRole("dialog", { name: "Edit member" });
    await user.click(within(dialog).getByRole("button", { name: /^Custom/ }));
    expect(screen.getByLabelText("From")).toHaveFocus();
    await user.tab();
    expect(screen.getByLabelText("To")).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(panel()).toBeNull();
    expect(screen.getByRole("dialog", { name: "Edit member" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /^Custom/ })).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Edit member" })).toBeNull();
  });
});

describe("Popover — placement", () => {
  /** Lay the page out as a 360×640 phone with a 224×300 panel; jsdom has no layout of its own. */
  function phoneLayout(triggerRect: { left: number; top: number; width: number; height: number }) {
    const isPanel = (el: Element) => el.matches(FLOATING_LAYER_SELECTOR);
    vi.spyOn(Element.prototype, "clientWidth", "get").mockImplementation(function (this: Element) {
      return this === document.documentElement ? 360 : 0;
    });
    vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
      return this === document.documentElement ? 640 : isPanel(this) ? 300 : 0;
    });
    vi.spyOn(Element.prototype, "scrollHeight", "get").mockImplementation(function (this: Element) {
      return isPanel(this) ? 300 : 0;
    });
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (this: HTMLElement) {
      return isPanel(this) ? 224 : 0;
    });
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
      return isPanel(this) ? 300 : 0;
    });
    const rect = { ...triggerRect, right: triggerRect.left + triggerRect.width, bottom: triggerRect.top + triggerRect.height, x: triggerRect.left, y: triggerRect.top };
    vi.spyOn(trigger(), "getBoundingClientRect").mockReturnValue({ ...rect, toJSON: () => rect } as DOMRect);
  }

  it("flips above and onto the trigger's far edge near a phone's bottom-right corner", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    phoneLayout({ left: 280, top: 560, width: 64, height: 40 });
    await user.click(trigger());
    const dialog = panel()!;
    expect(dialog.dataset.side).toBe("top");
    expect(dialog.dataset.align).toBe("end");
    expect(dialog.style.top).toBe(`${560 - 8 - 300}px`);
    expect(dialog.style.left).toBe(`${280 + 64 - 224}px`);
    expect(dialog.style.maxWidth).toBe("344px");
    expect(dialog.style.transformOrigin).toBe("bottom right");
  });

  it("moves with its trigger when the window resizes", async () => {
    const user = userEvent.setup();
    render(<FormPopover />);
    phoneLayout({ left: 20, top: 100, width: 64, height: 40 });
    await user.click(trigger());
    expect(panel()!.style.top).toBe("148px");
    vi.spyOn(trigger(), "getBoundingClientRect").mockReturnValue({ left: 20, top: 200, width: 64, height: 40, right: 84, bottom: 240, x: 20, y: 200, toJSON: () => ({}) } as DOMRect);
    fireEvent(window, new Event("resize"));
    expect(panel()!.style.top).toBe("248px");
    expect(panel()!.dataset.side).toBe("bottom");
  });
});
