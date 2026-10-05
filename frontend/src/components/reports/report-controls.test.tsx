// The period toolbar's opt-in custom range: "Custom" opens a small form (from,
// to, Apply) that only accepts real dates in order, ending by today and at
// most 366 days long; while a custom range is active the toolbar shows its
// dates, its length and the reporting zone. A report that doesn't opt in gets
// exactly the toolbar it had: presets, ‹ › stepping, no extras.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { ALL } from "@/lib/report-math";
import { NOW } from "@/lib/mock-data";
import { addZonedDays, zonedDayKey } from "@/lib/time-zone";
import { resetPrototypeState } from "@/test/harness";
import { CustomRangeForm, PeriodToolbar, type CustomRange, type PeriodPreset, type ToolbarPeriod } from "./report-controls";

beforeEach(() => {
  resetPrototypeState();
});

interface HostProps {
  custom?: boolean;
  initialPreset?: ToolbarPeriod;
  initialRange?: CustomRange | null;
  onPreset?: (p: PeriodPreset) => void;
  onRange?: (r: CustomRange) => void;
  timeZone?: string;
  spanLabel?: string;
  today?: string;
  suggestedRange?: CustomRange;
}

/** A report's period state around the toolbar, as Work Report keeps it. */
function Host({ custom = true, initialPreset = "7d", initialRange = null, onPreset, onRange, timeZone, spanLabel, today = "2026-07-20", suggestedRange }: HostProps) {
  const [preset, setPreset] = useState<ToolbarPeriod>(initialPreset);
  const [range, setRange] = useState<CustomRange | null>(initialRange);
  const [offset, setOffset] = useState(0);
  return (
    <PeriodToolbar
      preset={preset}
      onPreset={(p) => {
        setPreset(p);
        setOffset(0);
        onPreset?.(p);
      }}
      title={offset === 0 ? "Last 7 days" : "7-day period"}
      offset={offset}
      onStep={(by) => setOffset((o) => Math.min(0, o + by))}
      projectId={ALL}
      onProject={() => {}}
      memberId={ALL}
      onMember={() => {}}
      liveWorkerId={null}
      allowCustom={custom}
      range={range}
      onRange={(r) => {
        setRange(r);
        setPreset("custom");
        onRange?.(r);
      }}
      suggestedRange={suggestedRange}
      today={today}
      timeZone={timeZone}
      spanLabel={spanLabel}
    />
  );
}

const customButton = () => screen.getByRole("button", { name: "Custom" });
const form = () => screen.queryByRole("dialog", { name: "Custom range" });
const setDate = (label: "From" | "To", value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });

describe("PeriodToolbar without the custom range (existing reports)", () => {
  it("renders the presets and stepping only — no Custom button, no dates line, no zone", async () => {
    const user = userEvent.setup();
    const { container } = render(<Host custom={false} />);
    const group = screen.getByRole("group", { name: "Period" });
    expect(within(group).getAllByRole("button").map((b) => b.textContent)).toEqual(["Week", "7 days", "30 days", "Month"]);
    expect(within(group).getByRole("button", { name: "7 days" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "Custom" })).toBeNull();
    expect(screen.queryByText(/Reporting time zone/)).toBeNull();
    expect(screen.getByRole("button", { name: "Next period" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Previous period" }));
    expect(screen.getByText("7-day period")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back to today" }));
    expect(screen.getByText("Last 7 days")).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations();
  });

  it("offers no Custom button without a way to apply a range", () => {
    render(
      <PeriodToolbar
        preset="week"
        onPreset={() => {}}
        title="This week"
        offset={0}
        onStep={() => {}}
        projectId={ALL}
        onProject={() => {}}
        memberId={ALL}
        onMember={() => {}}
        liveWorkerId={null}
        allowCustom
      />,
    );
    expect(screen.queryByRole("button", { name: "Custom" })).toBeNull();
  });
});

describe("PeriodToolbar — custom range", () => {
  it("Custom opens a from/to form, prefilled with the 7 days ending today, naming the reporting zone", async () => {
    const user = userEvent.setup();
    render(<Host timeZone="Asia/Dhaka" />);
    expect(customButton()).toHaveAttribute("aria-haspopup", "dialog");
    expect(customButton()).toHaveAttribute("aria-pressed", "false");
    await user.click(customButton());
    const dialog = form()!;
    expect(screen.getByLabelText("From")).toHaveValue("2026-07-14");
    expect(screen.getByLabelText("To")).toHaveValue("2026-07-20");
    expect(screen.getByLabelText("From")).toHaveFocus();
    expect(screen.getByLabelText("To")).toHaveAttribute("max", "2026-07-20");
    expect(within(dialog).getByText("7 days · 14 Jul – 20 Jul 2026")).toBeInTheDocument();
    expect(within(dialog).getByText("Days in Dhaka (UTC+6)")).toBeInTheDocument();
  });

  it("starts from the report's suggestion (e.g. the current period) when no custom range is applied", async () => {
    const user = userEvent.setup();
    render(<Host suggestedRange={{ from: "2026-07-01", to: "2026-07-20" }} />);
    await user.click(customButton());
    expect(screen.getByLabelText("From")).toHaveValue("2026-07-01");
    expect(screen.getByLabelText("To")).toHaveValue("2026-07-20");
  });

  it("Apply hands the range to the report, closes the form and shows the span, its length and the zone", async () => {
    const user = userEvent.setup();
    const onRange = vi.fn();
    render(<Host onRange={onRange} timeZone="Asia/Kathmandu" />);
    await user.click(customButton());
    setDate("From", "2026-07-01");
    setDate("To", "2026-07-15");
    expect(within(form()!).getByText("15 days · 1 Jul – 15 Jul 2026")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(onRange).toHaveBeenCalledWith({ from: "2026-07-01", to: "2026-07-15" });
    expect(form()).toBeNull();
    expect(customButton()).toHaveFocus();
    expect(customButton()).toHaveAttribute("aria-pressed", "true");
    const presets = within(screen.getByRole("group", { name: "Period" }));
    for (const name of ["Week", "7 days", "30 days", "Month"]) {
      expect(presets.getByRole("button", { name })).toHaveAttribute("aria-pressed", "false");
    }
    expect(screen.getByText("1 Jul – 15 Jul 2026")).toBeInTheDocument();
    expect(screen.getByText("15 days")).toBeInTheDocument();
    expect(screen.getByText("Kathmandu (UTC+5:45)")).toBeInTheDocument();
    expect(screen.getByText("Reporting time zone:")).toHaveClass("sr-only");
    // A custom range doesn't step.
    expect(screen.queryByRole("button", { name: "Previous period" })).toBeNull();
  });

  it("Enter in a date field applies too", async () => {
    const user = userEvent.setup();
    const onRange = vi.fn();
    render(<Host onRange={onRange} />);
    await user.click(customButton());
    setDate("From", "2026-06-16");
    setDate("To", "2026-06-30");
    screen.getByLabelText("To").focus();
    await user.keyboard("{Enter}");
    expect(onRange).toHaveBeenCalledWith({ from: "2026-06-16", to: "2026-06-30" });
  });

  it.each([
    ["an end before the start", "2026-07-15", "2026-07-01", "To", "The end date can't be before the start date."],
    ["an end after today", "2026-07-01", "2026-07-21", "To", "The range can't end after today."],
    ["no start date", "", "2026-07-10", "From", "Enter a start date."],
    ["more than 366 days", "2025-01-01", "2026-07-20", "From", "Choose 366 days or fewer — this range has 566."],
  ] as const)("refuses %s, naming the field to fix", async (_case, from, to, field, message) => {
    const user = userEvent.setup();
    const onRange = vi.fn();
    render(<Host onRange={onRange} />);
    await user.click(customButton());
    setDate("From", from);
    setDate("To", to);
    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(onRange).not.toHaveBeenCalled();
    expect(form()).not.toBeNull();
    expect(screen.getByRole("alert")).toHaveTextContent(message);
    const input = screen.getByLabelText(field);
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription(message);
    expect(input).toHaveFocus();

    // Editing clears the complaint.
    setDate(field, field === "From" ? "2026-07-01" : "2026-07-10");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(input).not.toHaveAttribute("aria-invalid");
  });

  it("Cancel and Escape close the form without changing the period", async () => {
    const user = userEvent.setup();
    const onRange = vi.fn();
    render(<Host onRange={onRange} />);
    await user.click(customButton());
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(form()).toBeNull();
    await user.click(customButton());
    await user.keyboard("{Escape}");
    expect(form()).toBeNull();
    expect(customButton()).toHaveFocus();
    expect(onRange).not.toHaveBeenCalled();
    expect(customButton()).toHaveAttribute("aria-pressed", "false");
  });

  it("reopens on the applied range, and a preset brings the stepping back", async () => {
    const user = userEvent.setup();
    const onPreset = vi.fn();
    render(<Host initialPreset="custom" initialRange={{ from: "2026-07-01", to: "2026-07-15" }} onPreset={onPreset} />);
    expect(screen.getByText("1 Jul – 15 Jul 2026")).toBeInTheDocument();
    await user.click(customButton());
    expect(screen.getByLabelText("From")).toHaveValue("2026-07-01");
    expect(screen.getByLabelText("To")).toHaveValue("2026-07-15");
    await user.keyboard("{Escape}");

    await user.click(screen.getByRole("button", { name: "Week" }));
    expect(onPreset).toHaveBeenCalledWith("week");
    expect(screen.getByRole("button", { name: "Previous period" })).toBeInTheDocument();
    expect(customButton()).toHaveAttribute("aria-pressed", "false");
  });

  it("shows a preset's dates and the zone when the report passes them", () => {
    render(<Host spanLabel="29 Sep – 5 Oct 2026" timeZone="UTC" />);
    expect(screen.getByText("29 Sep – 5 Oct 2026")).toBeInTheDocument();
    expect(screen.getByText("UTC", { exact: true })).toBeInTheDocument();
  });

  it("a11y: the toolbar and the open custom form have no axe violations", async () => {
    const user = userEvent.setup();
    const { container } = render(<Host timeZone="Asia/Dhaka" spanLabel="14 Jul – 20 Jul 2026" />);
    await user.click(customButton());
    expect(await axe(container)).toHaveNoViolations();
    expect(await axe(form()!)).toHaveNoViolations();
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("CustomRangeForm on its own", () => {
  it("reads 'today' from the session clock in the zone when not told (the demo's frozen clock here)", () => {
    render(<CustomRangeForm timeZone="Pacific/Kiritimati" onApply={() => {}} onCancel={() => {}} />);
    const today = zonedDayKey(NOW, "Pacific/Kiritimati");
    expect(screen.getByLabelText("To")).toHaveValue(today);
    expect(screen.getByLabelText("From")).toHaveValue(addZonedDays(today, -6));
    expect(screen.getByLabelText("To")).toHaveAttribute("max", today);
  });

  it("honours a shorter limit", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(<CustomRangeForm today="2026-07-20" maxDays={31} initial={{ from: "2026-06-01", to: "2026-07-20" }} onApply={onApply} onCancel={() => {}} />);
    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Choose 31 days or fewer — this range has 50.");
    expect(onApply).not.toHaveBeenCalled();
  });
});
