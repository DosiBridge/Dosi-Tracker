// Form primitives (Input, Select, Switch, Button) tested as a user experiences
// them: reachable via their labels, operable by mouse AND keyboard, and inert
// when disabled.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { Input, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

/** Stateful host: the app always drives Switch as a controlled component. */
function SwitchHost({ description }: { description?: string }) {
  const [on, setOn] = useState(false);
  return <Switch checked={on} onChange={setOn} label="Email alerts" description={description} />;
}

describe("Input", () => {
  it("is reachable via its label and accepts typing", async () => {
    const user = userEvent.setup();
    render(
      <div>
        <label htmlFor="email">Work email</label>
        <Input id="email" />
      </div>,
    );
    const input = screen.getByLabelText("Work email");
    await user.type(input, "ana@dosi.dev");
    expect(input).toHaveValue("ana@dosi.dev");
  });

  it("rejects typing while disabled", async () => {
    const user = userEvent.setup();
    render(
      <div>
        <label htmlFor="email">Work email</label>
        <Input id="email" disabled defaultValue="locked" />
      </div>,
    );
    const input = screen.getByLabelText("Work email");
    expect(input).toBeDisabled();
    await user.type(input, "nope");
    expect(input).toHaveValue("locked");
  });
});

describe("Select", () => {
  it("is reachable via its label and lets the user pick an option", async () => {
    const user = userEvent.setup();
    render(
      <div>
        <label htmlFor="role">Role</label>
        <Select id="role" defaultValue="worker">
          <option value="worker">Worker</option>
          <option value="admin">Admin</option>
        </Select>
      </div>,
    );
    const select = screen.getByLabelText("Role");
    expect(select).toHaveValue("worker");
    await user.selectOptions(select, "Admin");
    expect(select).toHaveValue("admin");
  });
});

describe("Switch", () => {
  it("is found by its label text and exposes the switch role", () => {
    render(<SwitchHost />);
    const sw = screen.getByLabelText("Email alerts");
    expect(sw).toHaveAttribute("role", "switch");
  });

  it("shows its description text", () => {
    render(<SwitchHost description="Daily digest at 9am" />);
    expect(screen.getByText("Daily digest at 9am")).toBeInTheDocument();
  });

  it("toggles on click, both directions", async () => {
    const user = userEvent.setup();
    render(<SwitchHost />);
    const sw = screen.getByRole("switch");
    expect(sw).not.toBeChecked();
    await user.click(sw);
    expect(sw).toBeChecked();
    await user.click(sw);
    expect(sw).not.toBeChecked();
  });

  it("toggles from the keyboard with Space and with Enter", async () => {
    const user = userEvent.setup();
    render(<SwitchHost />);
    const sw = screen.getByRole("switch");
    await user.tab();
    expect(sw).toHaveFocus();
    await user.keyboard(" ");
    expect(sw).toBeChecked();
    await user.keyboard("{Enter}");
    expect(sw).not.toBeChecked();
  });
});

describe("Button", () => {
  const variants = ["primary", "secondary", "outline", "ghost", "danger"] as const;

  it("every variant renders its accessible name", () => {
    render(
      <>
        {variants.map((v) => (
          <Button key={v} variant={v}>
            {`${v} action`}
          </Button>
        ))}
      </>,
    );
    for (const v of variants) {
      expect(screen.getByRole("button", { name: `${v} action` })).toBeInTheDocument();
    }
  });

  it("a disabled button never fires its handler", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        Save
      </Button>,
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(onClick).not.toHaveBeenCalled();
  });

  it("an enabled button fires on click and on keyboard activation", async () => {
    const user = userEvent.setup();
    let saves = 0;
    render(<Button onClick={() => (saves += 1)}>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" });
    await user.click(button);
    expect(saves).toBe(1);
    button.focus();
    await user.keyboard("{Enter}");
    expect(saves).toBe(2);
  });
});

describe("form controls a11y", () => {
  it("labelled controls together produce no axe violations", async () => {
    const { container } = render(
      <form aria-label="Notification settings">
        <label htmlFor="n">Name</label>
        <Input id="n" />
        <label htmlFor="r">Role</label>
        <Select id="r">
          <option>Worker</option>
        </Select>
        <SwitchHost description="Daily digest at 9am" />
        <Button>Save</Button>
      </form>,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
