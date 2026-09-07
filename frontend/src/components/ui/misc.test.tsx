// Presentational primitives: Badge, EmptyState, Progress, Toolbar
// (SearchField/SegmentedControl) and PageHeader. Assertions target what a user
// (or their assistive tech) perceives: text content, accessible values/names,
// and working interactions.
import { describe, it, expect, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { Inbox } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/empty-state";
import { Progress } from "@/components/ui/progress";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { PageHeader, SectionLabel, PageStack } from "@/components/ui/page-header";
import { Button } from "@/components/ui/button";
import { resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

function SearchHost() {
  const [q, setQ] = useState("");
  return <SearchField value={q} onChange={setQ} placeholder="Search members…" />;
}

function SegmentedHost() {
  const [view, setView] = useState<"active" | "archived">("active");
  return (
    <SegmentedControl
      value={view}
      onChange={setView}
      options={[
        { value: "active", label: "Active" },
        { value: "archived", label: "Archived" },
      ]}
    />
  );
}

describe("Badge", () => {
  it("renders its content for every tone", () => {
    const tones = ["default", "primary", "success", "warning", "danger", "info", "muted"] as const;
    render(
      <>
        {tones.map((t) => (
          <Badge key={t} tone={t}>
            {`${t} badge`}
          </Badge>
        ))}
      </>,
    );
    for (const t of tones) {
      expect(screen.getByText(`${t} badge`)).toBeInTheDocument();
    }
  });
});

describe("EmptyState", () => {
  it("communicates the state through text, not colour alone", () => {
    render(
      <EmptyState
        icon={Inbox}
        title="No screenshots yet"
        description="Captures will appear once tracking starts."
      />,
    );
    expect(screen.getByText("No screenshots yet")).toBeInTheDocument();
    expect(screen.getByText(/captures will appear once tracking starts/i)).toBeInTheDocument();
  });

  it("its action button invokes the handler", async () => {
    const user = userEvent.setup();
    let invoked = 0;
    render(
      <EmptyState
        icon={Inbox}
        title="No projects"
        action={{ label: "Create project", onClick: () => (invoked += 1) }}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Create project" }));
    expect(invoked).toBe(1);
  });

  it("a11y: has no axe violations", async () => {
    const { container } = render(
      <EmptyState
        icon={Inbox}
        title="No projects"
        description="Create one to get going."
        action={{ label: "Create project", onClick: () => undefined }}
      />,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("Progress", () => {
  it("exposes its value to assistive tech", () => {
    render(<Progress value={42} aria-label="Storage used" />);
    const bar = screen.getByRole("progressbar", { name: "Storage used" });
    expect(bar).toHaveAttribute("aria-valuenow", "42");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
  });

  it("clamps out-of-range values in the accessible value", () => {
    render(
      <>
        <Progress value={150} aria-label="Over" />
        <Progress value={-5} aria-label="Under" />
      </>,
    );
    expect(screen.getByRole("progressbar", { name: "Over" })).toHaveAttribute(
      "aria-valuenow",
      "100",
    );
    expect(screen.getByRole("progressbar", { name: "Under" })).toHaveAttribute(
      "aria-valuenow",
      "0",
    );
  });
});

describe("SearchField", () => {
  it("typing updates the query through onChange", async () => {
    const user = userEvent.setup();
    render(<SearchHost />);
    const box = screen.getByRole("textbox", { name: /search members/i });
    await user.type(box, "ana");
    expect(box).toHaveValue("ana");
  });
});

describe("SegmentedControl", () => {
  it("marks the current option selected and moves selection on click", async () => {
    const user = userEvent.setup();
    render(<SegmentedHost />);
    expect(screen.getByRole("tab", { name: "Active" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Archived" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    await user.click(screen.getByRole("tab", { name: "Archived" }));
    expect(screen.getByRole("tab", { name: "Archived" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Active" })).toHaveAttribute("aria-selected", "false");
  });
});

describe("PageHeader", () => {
  it("renders eyebrow, level-1 title, description and actions", () => {
    render(
      <PageHeader
        eyebrow="Workspace"
        title="Team"
        description="Manage members and roles."
        actions={<Button>Invite</Button>}
      />,
    );
    expect(screen.getByRole("heading", { level: 1, name: "Team" })).toBeInTheDocument();
    expect(screen.getByText("Workspace")).toBeInTheDocument();
    expect(screen.getByText("Manage members and roles.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Invite" })).toBeInTheDocument();
  });

  it("SectionLabel renders a level-2 heading", () => {
    render(<SectionLabel>Live now</SectionLabel>);
    expect(screen.getByRole("heading", { level: 2, name: "Live now" })).toBeInTheDocument();
  });
});

describe("composed page chrome a11y", () => {
  it("header + toolbar + empty state + progress produce no axe violations", async () => {
    const { container } = render(
      <PageStack>
        <PageHeader
          eyebrow="Workspace"
          title="Team"
          description="Manage members and roles."
          actions={<Button>Invite</Button>}
        />
        <SectionLabel>Members</SectionLabel>
        <Toolbar>
          <SearchHost />
          <SegmentedHost />
        </Toolbar>
        <EmptyState icon={Inbox} title="No members yet" description="Invite your first teammate." />
        <Progress value={30} aria-label="Seats used" />
        <Badge tone="success">Active</Badge>
      </PageStack>,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
