// DropdownMenu as a WAI-ARIA menu button: a named menu of actions or choices
// that opens from the button (Enter, Space, ↓ on the first item; ↑ on the
// last), moves with the arrow keys, Home/End and first letters, picks with
// Enter, and closes on Escape, Tab, a pick or a press outside — handing focus
// back to the button. Disabled items are announced but can't be picked.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { FileText, Printer, Trash2 } from "lucide-react";
import { DropdownMenu, type DropdownMenuEntry } from "@/components/ui/dropdown-menu";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

function ExportMenu({ onPick, entries }: { onPick: (key: string) => void; entries?: DropdownMenuEntry[] }) {
  const items: DropdownMenuEntry[] = entries ?? [
    { key: "csv", label: "Daily log (CSV)", icon: FileText, onSelect: () => onPick("csv") },
    { key: "timesheet", label: "Team timesheet (CSV)", icon: FileText, onSelect: () => onPick("timesheet") },
    { kind: "separator", key: "sep" },
    { key: "print", label: "Print / PDF", icon: Printer, onSelect: () => onPick("print"), description: "Save as a PDF from the print dialog" },
  ];
  return (
    <div>
      <DropdownMenu label="Export" align="end" items={items} trigger={(props) => <button {...props}>Export</button>} />
      <button type="button">Next control</button>
    </div>
  );
}

const button = () => screen.getByRole("button", { name: "Export" });
const menu = () => screen.queryByRole("menu", { name: "Export" });
const item = (name: string | RegExp) => screen.getByRole("menuitem", { name });

describe("DropdownMenu", () => {
  it("is a menu button: closed until pressed, then a named menu of items", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    expect(button()).toHaveAttribute("aria-haspopup", "menu");
    expect(button()).toHaveAttribute("aria-expanded", "false");
    expect(menu()).toBeNull();
    await user.click(button());
    expect(menu()).not.toBeNull();
    expect(within(menu()!).getAllByRole("menuitem").map((el) => el.getAttribute("aria-label") ?? el.textContent)).toEqual([
      "Daily log (CSV)",
      "Team timesheet (CSV)",
      "Print / PDF",
    ]);
    // A second line describes the item without becoming part of its name.
    expect(item("Print / PDF")).toHaveAccessibleDescription("Save as a PDF from the print dialog");
    expect(within(menu()!).getByRole("separator")).toBeInTheDocument();
  });

  it("opened with the pointer, nothing is highlighted until the pointer or a key reaches an item", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    await user.click(button());
    expect(menu()).toHaveFocus();
    await user.hover(item("Team timesheet (CSV)"));
    expect(item("Team timesheet (CSV)")).toHaveFocus();

    await user.keyboard("{Escape}");
    await user.click(button());
    await user.keyboard("{ArrowDown}");
    expect(item("Daily log (CSV)")).toHaveFocus();
  });

  it("opens from the keyboard on the first item and moves with ↓ ↑ Home End (wrapping)", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    button().focus();
    await user.keyboard("{Enter}");
    expect(item("Daily log (CSV)")).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(item("Team timesheet (CSV)")).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(item("Daily log (CSV)")).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(item("Print / PDF")).toHaveFocus();
    await user.keyboard("{Home}");
    expect(item("Daily log (CSV)")).toHaveFocus();
    await user.keyboard("{End}");
    expect(item("Print / PDF")).toHaveFocus();
  });

  it("↓ on the button opens on the first item, ↑ on the last", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    button().focus();
    await user.keyboard("{ArrowUp}");
    expect(item("Print / PDF")).toHaveFocus();
    await user.keyboard("{Escape}");
    await user.keyboard("{ArrowDown}");
    expect(item("Daily log (CSV)")).toHaveFocus();
  });

  it("a letter jumps to the next item starting with it", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    button().focus();
    await user.keyboard("{Enter}");
    await user.keyboard("p");
    expect(item("Print / PDF")).toHaveFocus();
    await user.keyboard("t");
    expect(item("Team timesheet (CSV)")).toHaveFocus();
    await user.keyboard("z");
    expect(item("Team timesheet (CSV)")).toHaveFocus();
  });

  it("Enter picks the item, closes the menu and returns focus to the button", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<ExportMenu onPick={onPick} />);
    button().focus();
    await user.keyboard("{Enter}{ArrowDown}{Enter}");
    expect(onPick).toHaveBeenCalledWith("timesheet");
    expect(menu()).toBeNull();
    expect(button()).toHaveFocus();
  });

  it("a click picks too", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<ExportMenu onPick={onPick} />);
    await user.click(button());
    await user.click(item("Print / PDF"));
    expect(onPick).toHaveBeenCalledWith("print");
    expect(menu()).toBeNull();
  });

  it("Escape closes it back to the button", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    button().focus();
    await user.keyboard("{Enter}");
    await user.keyboard("{Escape}");
    expect(menu()).toBeNull();
    expect(button()).toHaveFocus();
  });

  it("Tab closes it from the button, so the browser's own Tab moves on to the next control", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    button().focus();
    await user.keyboard("{Enter}");
    // user-event moves Tab from the key's original target; a browser moves it
    // from the element focused when the key's default runs — the button.
    const notPrevented = fireEvent.keyDown(item("Daily log (CSV)"), { key: "Tab" });
    expect(notPrevented).toBe(true);
    expect(menu()).toBeNull();
    expect(button()).toHaveFocus();
  });

  it("a press outside closes it", async () => {
    const user = userEvent.setup();
    render(<ExportMenu onPick={() => {}} />);
    await user.click(button());
    await user.click(screen.getByRole("button", { name: "Next control" }));
    expect(menu()).toBeNull();
  });

  it("disabled items are announced and reachable, but can't be picked", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(
      <ExportMenu
        onPick={onPick}
        entries={[
          { key: "csv", label: "Daily log (CSV)", onSelect: () => onPick("csv") },
          { key: "matrix", label: "Team timesheet (CSV)", onSelect: () => onPick("matrix"), disabled: true },
        ]}
      />,
    );
    await user.click(button());
    const disabled = item("Team timesheet (CSV)");
    expect(disabled).toHaveAttribute("aria-disabled", "true");
    await user.click(disabled);
    expect(onPick).not.toHaveBeenCalled();
    expect(menu()).not.toBeNull();
    disabled.focus();
    await user.keyboard("{Enter}");
    expect(onPick).not.toHaveBeenCalled();
    await user.keyboard("{ArrowUp}");
    expect(item("Daily log (CSV)")).toHaveFocus();
  });

  it("marks destructive actions", async () => {
    const user = userEvent.setup();
    render(
      <ExportMenu onPick={() => {}} entries={[{ key: "remove", label: "Remove from workspace", icon: Trash2, tone: "danger", onSelect: () => {} }]} />,
    );
    await user.click(button());
    expect(item("Remove from workspace").className).toContain("text-danger-on-tint");
  });

  // Scanned as components (button + open menu), not as a page: axe's
  // best-practice "region" rule would flag any popup menu portaled to <body>.
  it("a11y: the button and the open menu have no axe violations", async () => {
    const user = userEvent.setup();
    const { container } = render(<ExportMenu onPick={() => {}} />);
    await user.click(button());
    expect(await axe(container)).toHaveNoViolations();
    expect(await axe(menu()!)).toHaveNoViolations();
  });
});

describe("DropdownMenu — a list of choices", () => {
  function PeriodPicker() {
    const [value, setValue] = useState("30d");
    const options = [
      { key: "7d", label: "Last 7 days" },
      { key: "30d", label: "Last 30 days" },
      { key: "month", label: "This month" },
    ];
    return (
      <DropdownMenu
        label="Period"
        items={[
          { kind: "heading", key: "rolling", label: "Rolling" },
          ...options.slice(0, 2).map((o) => ({ ...o, checked: value === o.key, onSelect: () => setValue(o.key) })),
          { kind: "heading", key: "calendar", label: "Calendar" },
          { ...options[2], checked: value === options[2].key, onSelect: () => setValue(options[2].key) },
        ]}
        trigger={(props) => <button {...props}>{options.find((o) => o.key === value)?.label}</button>}
      />
    );
  }

  it("announces the chosen one as checked and opens on it", async () => {
    const user = userEvent.setup();
    render(<PeriodPicker />);
    screen.getByRole("button", { name: "Last 30 days" }).focus();
    await user.keyboard("{Enter}");
    const chosen = screen.getByRole("menuitemradio", { name: "Last 30 days" });
    expect(chosen).toHaveAttribute("aria-checked", "true");
    expect(chosen).toHaveFocus();
    expect(screen.getByRole("menuitemradio", { name: "Last 7 days" })).toHaveAttribute("aria-checked", "false");
  });

  it("names each group by its heading, and picking a choice updates the button", async () => {
    const user = userEvent.setup();
    render(<PeriodPicker />);
    await user.click(screen.getByRole("button", { name: "Last 30 days" }));
    const calendar = screen.getByRole("group", { name: "Calendar" });
    await user.click(within(calendar).getByRole("menuitemradio", { name: "This month" }));
    expect(screen.getByRole("button", { name: "This month" })).toHaveFocus();
  });

  it("a11y: a menu of choices with headings has no axe violations", async () => {
    const user = userEvent.setup();
    const { container } = render(<PeriodPicker />);
    await user.click(screen.getByRole("button", { name: "Last 30 days" }));
    expect(await axe(container)).toHaveNoViolations();
    expect(await axe(screen.getByRole("menu", { name: "Period" }))).toHaveNoViolations();
  });
});
