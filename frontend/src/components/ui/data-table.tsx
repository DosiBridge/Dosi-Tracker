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
  /** Hide this field in the mobile card stack (still shown in table). */
  hideOnMobile?: boolean;
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
  /**
   * Mobile strategy:
   * - `cards` (default) — stacked label/value cards below `md`
   * - `scroll` — keep table with horizontal scroll + sticky first column
   */
  mobileLayout = "cards",
}: {
  columns: Column<T>[];
  rows: T[];
  initialSort?: { key: string; dir: "asc" | "desc" };
  emptyText?: string;
  footer?: React.ReactNode;
  className?: string;
  mobileLayout?: "scroll" | "cards";
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
  const useCards = mobileLayout === "cards";
  const mobileCols = columns.filter((c) => !c.hideOnMobile);

  return (
    <div className={cn(className)}>
      {/* Mobile card stack */}
      {useCards && (
        <div className="space-y-3 md:hidden">
          {sorted.length === 0 ? (
            <p className="py-10 text-center text-sm text-muted-foreground">{emptyText}</p>
          ) : (
            sorted.map((row, i) => (
              <div
                key={i}
                className="rounded-xl border border-border bg-card/50 p-3.5 shadow-sm"
              >
                {mobileCols.map((c, ci) => (
                  <div
                    key={c.key}
                    className={cn(
                      "flex items-start justify-between gap-3 py-2",
                      ci > 0 && "border-t border-border/60",
                      ci === 0 && "pb-2.5 pt-0"
                    )}
                  >
                    <span className="shrink-0 pt-0.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                      {c.header}
                    </span>
                    <div
                      className={cn(
                        "min-w-0 text-sm",
                        c.align === "right" && "text-right",
                        c.align === "center" && "text-center",
                        c.className
                      )}
                    >
                      {c.render(row)}
                    </div>
                  </div>
                ))}
              </div>
            ))
          )}
          {footer && sorted.length > 0 && (
            <div className="rounded-xl border border-border bg-muted/40 px-3.5 py-3 text-sm font-semibold">
              {footer}
            </div>
          )}
        </div>
      )}

      {/* Table (always on md+, or always when scroll mode) */}
      <div
        className={cn(
          "overflow-x-auto overscroll-x-contain",
          useCards && "hidden md:block",
          /* Bleed into card padding so the scrollbar feels full-width */
          "-mx-1 px-1"
        )}
      >
        <table className="w-full min-w-[560px] border-collapse text-sm">
          <thead>
            <tr className="border-b border-border">
              {columns.map((c, ci) => (
                <th
                  key={c.key}
                  className={cn(
                    "whitespace-nowrap px-3 py-2.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground",
                    alignClass[c.align ?? "left"],
                    c.sortValue && "cursor-pointer select-none hover:text-foreground",
                    ci === 0 && "sticky left-0 z-10 bg-card",
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
                <tr key={i} className="group border-b border-border/60 transition-colors hover:bg-muted/50">
                  {columns.map((c, ci) => (
                    <td
                      key={c.key}
                      className={cn(
                        "px-3 py-2.5",
                        alignClass[c.align ?? "left"],
                        ci === 0 && "sticky left-0 z-10 bg-card group-hover:bg-muted/50",
                        c.className
                      )}
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
    </div>
  );
}
