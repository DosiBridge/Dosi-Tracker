// DataTable is the shared list surface (Team, Projects, Host console). These
// tests pin its user-visible contract: rows render from data, clicking a
// sortable header re-orders rows (descending first, then ascending), empty and
// footer states show, the mobile card stack mirrors the table minus
// `hideOnMobile` columns, and it survives 1000 rows with sorting still exact.
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { DataTable, type Column } from "@/components/ui/data-table";
import { resetPrototypeState } from "@/test/harness";
import { buildUser } from "@/test/factories";
import type { User } from "@/lib/types";

beforeEach(() => {
  resetPrototypeState();
});

const columns: Column<User>[] = [
  { key: "name", header: "Member", sortValue: (u) => u.name, render: (u) => u.name },
  {
    key: "tracked",
    header: "Tracked",
    align: "right",
    sortValue: (u) => u.trackedToday,
    render: (u) => `${u.trackedToday}m`,
  },
  { key: "designation", header: "Designation", hideOnMobile: true, render: (u) => u.designation },
];

/** Three members whose numeric and lexical sort orders DIFFER ("9" > "80" lexically). */
function crew(): User[] {
  return [
    buildUser({ name: "Zoe", trackedToday: 9, designation: "Designer" }),
    buildUser({ name: "Ana", trackedToday: 700, designation: "Engineer" }),
    buildUser({ name: "Mia", trackedToday: 80, designation: "PM" }),
  ];
}

/** First-column text of each body row, in DOM order. */
function firstColumnTexts(container: HTMLElement): string[] {
  const tbody = container.querySelector("tbody");
  if (!tbody) throw new Error("table body not rendered");
  return Array.from(tbody.querySelectorAll("tr")).map(
    (tr) => within(tr).getAllByRole("cell")[0].textContent ?? "",
  );
}

describe("DataTable", () => {
  it("renders a body row for every data item, preserving the given order", () => {
    const { container } = render(
      <DataTable columns={columns} rows={crew()} mobileLayout="scroll" />,
    );
    expect(firstColumnTexts(container)).toEqual(["Zoe", "Ana", "Mia"]);
    const table = screen.getByRole("table");
    expect(within(table).getByText("700m")).toBeInTheDocument();
    expect(within(table).getByText("Designer")).toBeInTheDocument();
  });

  it("renders every column header", () => {
    render(<DataTable columns={columns} rows={crew()} mobileLayout="scroll" />);
    for (const header of ["Member", "Tracked", "Designation"]) {
      expect(screen.getByRole("columnheader", { name: header })).toBeInTheDocument();
    }
  });

  it("sorts descending on the first header click, ascending on the second", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <DataTable columns={columns} rows={crew()} mobileLayout="scroll" />,
    );
    await user.click(screen.getByRole("columnheader", { name: "Member" }));
    expect(firstColumnTexts(container)).toEqual(["Zoe", "Mia", "Ana"]);
    await user.click(screen.getByRole("columnheader", { name: "Member" }));
    expect(firstColumnTexts(container)).toEqual(["Ana", "Mia", "Zoe"]);
  });

  it("sorts numeric columns by value, not lexically", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <DataTable columns={columns} rows={crew()} mobileLayout="scroll" />,
    );
    // Descending by minutes: 700, 80, 9 (a lexical sort would order "9" > "80" > "700").
    await user.click(screen.getByRole("columnheader", { name: "Tracked" }));
    expect(firstColumnTexts(container)).toEqual(["Ana", "Mia", "Zoe"]);
    await user.click(screen.getByRole("columnheader", { name: "Tracked" }));
    expect(firstColumnTexts(container)).toEqual(["Zoe", "Mia", "Ana"]);
  });

  it("clicking a column without sortValue leaves the order untouched", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <DataTable columns={columns} rows={crew()} mobileLayout="scroll" />,
    );
    await user.click(screen.getByRole("columnheader", { name: "Designation" }));
    expect(firstColumnTexts(container)).toEqual(["Zoe", "Ana", "Mia"]);
  });

  it("applies initialSort before any interaction", () => {
    const { container } = render(
      <DataTable
        columns={columns}
        rows={crew()}
        initialSort={{ key: "name", dir: "asc" }}
        mobileLayout="scroll"
      />,
    );
    expect(firstColumnTexts(container)).toEqual(["Ana", "Mia", "Zoe"]);
  });

  it("shows the empty text (and keeps headers) when there are no rows", () => {
    render(
      <DataTable
        columns={columns}
        rows={[]}
        emptyText="Nobody here yet."
        mobileLayout="scroll"
      />,
    );
    expect(screen.getByText("Nobody here yet.")).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "Member" })).toBeInTheDocument();
  });

  it("falls back to the default empty text", () => {
    render(<DataTable columns={columns} rows={[]} mobileLayout="scroll" />);
    expect(screen.getByText("No data.")).toBeInTheDocument();
  });

  it("cell renderers act on their own row (action button reports the right member)", async () => {
    const user = userEvent.setup();
    const removed: string[] = [];
    const actionColumns: Column<User>[] = [
      ...columns,
      {
        key: "actions",
        header: "Actions",
        render: (u) => (
          <button type="button" onClick={() => removed.push(u.name)}>
            Remove {u.name}
          </button>
        ),
      },
    ];
    render(<DataTable columns={actionColumns} rows={crew()} mobileLayout="scroll" />);
    await user.click(screen.getByRole("button", { name: "Remove Mia" }));
    expect(removed).toEqual(["Mia"]);
  });

  it("renders the footer row under the table", () => {
    render(
      <DataTable
        columns={columns}
        rows={crew()}
        mobileLayout="scroll"
        footer={<td colSpan={3}>Total 789m</td>}
      />,
    );
    const table = screen.getByRole("table");
    expect(within(table).getByText("Total 789m")).toBeInTheDocument();
  });

  it("card layout repeats headers as per-card labels but omits hideOnMobile columns from the stack", () => {
    render(<DataTable columns={columns} rows={crew().slice(0, 2)} />);
    // "Tracked" appears once in the table header and once per card (2 rows).
    expect(screen.getAllByText("Tracked")).toHaveLength(3);
    // "Designation" is hideOnMobile: only the table header shows it as a label.
    expect(screen.getAllByText("Designation")).toHaveLength(1);
    // Values still render in both the card stack and the table.
    expect(screen.getAllByText("9m")).toHaveLength(2);
  });

  it("TORTURE: 1000 rows render completely and sorting stays exact", async () => {
    const rows = Array.from({ length: 1000 }, (_, i) => {
      // (i * 389) % 1000 visits every value 0..999 exactly once — a fixed shuffle.
      const n = (i * 389) % 1000;
      return buildUser({ name: `Member ${String(n).padStart(4, "0")}`, trackedToday: n });
    });
    // pointerEventsCheck is disabled for this case only: user-event otherwise
    // recomputes pointer-events visibility across all 1000 rows on every click,
    // which dominates the runtime (~30s) and makes the test time out on a busy
    // machine. The behavior under test is sort correctness at scale, and the
    // ordinary click path is already covered by the sorting tests above.
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    const { container } = render(
      <DataTable columns={columns} rows={rows} mobileLayout="scroll" />,
    );
    expect(firstColumnTexts(container)).toHaveLength(1000);

    await user.click(screen.getByRole("columnheader", { name: "Member" }));
    const descending = Array.from(
      { length: 1000 },
      (_, i) => `Member ${String(999 - i).padStart(4, "0")}`,
    );
    expect(firstColumnTexts(container)).toEqual(descending);

    await user.click(screen.getByRole("columnheader", { name: "Member" }));
    expect(firstColumnTexts(container)).toEqual([...descending].reverse());
  }, 60000);

  it("a11y: populated table has no axe violations", async () => {
    const { container } = render(
      <DataTable columns={columns} rows={crew()} mobileLayout="scroll" />,
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
