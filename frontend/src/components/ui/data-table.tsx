"use client";

import { useMemo, useState } from "react";
import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react";
import { cn } from "@/lib/utils";

export interface Column<T> {
  key: string;
  header: string;
  align?: "left" | "right" | "center";
  className?: string;
  headerClassName?: string;
  sortValue?: (row: T) => number | string;
  render: (row: T) => React.ReactNode;
}

export function DataTable<T>({
  columns,
  rows,
  initialSort,
  emptyText = "No data.",
  footer,
  className,
}: {
  columns: Column<T>[];
  rows: T[];
  initialSort?: { key: string; dir: "asc" | "desc" };
  emptyText?: string;
  footer?: React.ReactNode;
  className?: string;
}) {
  const [sort, setSort] = useState(initialSort ?? null);

  const sorted = useMemo(() => {
    if (!sort) return rows;
    const col = columns.find((c) => c.key === sort.key);
    if (!col?.sortValue) return rows;
    const dir = sort.dir === "asc" ? 1 : -1;
    return [...rows].sort((a, b) => {
      const av = col.sortValue!(a);
      const bv = col.sortValue!(b);
      if (av < bv) return -1 * dir;
      if (av > bv) return 1 * dir;
      return 0;
    });
  }, [rows, sort, columns]);

  function toggleSort(key: string) {
    setSort((s) =>
      s?.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "desc" }
    );
  }

  const alignClass = { left: "text-left", right: "text-right", center: "text-center" };

  return (
    <div className={cn("overflow-x-auto", className)}>
      <table className="w-full min-w-[560px] border-collapse text-sm">
        <thead>
          <tr className="border-b border-border">
            {columns.map((c) => (
              <th
                key={c.key}
                className={cn(
                  "whitespace-nowrap px-3 py-2.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground",
                  alignClass[c.align ?? "left"],
                  c.sortValue && "cursor-pointer select-none hover:text-foreground",
                  c.headerClassName
                )}
                onClick={() => c.sortValue && toggleSort(c.key)}
              >
                <span className={cn("inline-flex items-center gap-1", c.align === "right" && "flex-row-reverse")}>
                  {c.header}
                  {c.sortValue &&
                    (sort?.key === c.key ? (
                      sort.dir === "asc" ? <ArrowUp className="h-3 w-3" /> : <ArrowDown className="h-3 w-3" />
                    ) : (
                      <ChevronsUpDown className="h-3 w-3 opacity-40" />
                    ))}
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.length === 0 ? (
            <tr>
              <td colSpan={columns.length} className="px-3 py-10 text-center text-muted-foreground">
                {emptyText}
              </td>
            </tr>
          ) : (
            sorted.map((row, i) => (
              <tr key={i} className="border-b border-border/60 transition-colors hover:bg-muted/50">
                {columns.map((c) => (
                  <td
                    key={c.key}
                    className={cn("px-3 py-2.5", alignClass[c.align ?? "left"], c.className)}
                  >
                    {c.render(row)}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
        {footer && (
          <tfoot>
            <tr className="border-t-2 border-border font-semibold">{footer}</tr>
          </tfoot>
        )}
      </table>
    </div>
  );
}
