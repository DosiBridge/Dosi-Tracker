"use client";

import type { LucideIcon } from "lucide-react";
import { ChevronDown, Download, FileText, Printer } from "lucide-react";
import { DropdownMenu, type DropdownMenuEntry } from "@/components/ui/dropdown-menu";
import { toast } from "@/components/toast";
import { cn } from "@/lib/utils";

/*
 * The Export menu on its own. Pages outside the report family (Billing, the
 * host consoles) offer the same downloads; importing it from here keeps the
 * report kit's charts and period logic out of their bundles. report-shell
 * re-exports it, so the reports import it from there as before.
 */

/** An additional download offered under Export (e.g. a second CSV). */
export interface ExportMenuItem {
  label: string;
  onSelect: () => void;
  icon?: LucideIcon;
}

/**
 * Export: the CSV downloads and Print / PDF, in the collision-aware menu (it
 * flips and slides to stay on screen and opens above the sidebar). Pass
 * `disabled` while the figures are loading, failed or empty: an export then
 * would be a well-formed file of wrong numbers.
 */
export function ExportMenu({
  onExportCSV,
  csvLabel = "Download CSV",
  extraItems = [],
  disabled = false,
  disabledReason = "Export is available once the report has loaded",
}: {
  onExportCSV: () => void;
  /** Label of the primary CSV item. */
  csvLabel?: string;
  /** Further downloads listed after the primary CSV (each confirms with the same toast). */
  extraItems?: ExportMenuItem[];
  /** The figures aren't ready (loading, failed or empty): nothing can be exported. */
  disabled?: boolean;
  /** Why it is disabled (shown as the button's tooltip). */
  disabledReason?: string;
}) {
  const downloads = [{ label: csvLabel, onSelect: onExportCSV, icon: FileText }, ...extraItems];
  const items: DropdownMenuEntry[] = [
    ...downloads.map((item, i) => ({
      key: `download-${i}`,
      label: item.label,
      icon: item.icon ?? FileText,
      onSelect: () => {
        item.onSelect();
        toast({ title: "Export ready", description: "Your CSV has been downloaded.", tone: "success" });
      },
    })),
    { kind: "separator", key: "print-separator" },
    {
      key: "print",
      label: "Print / PDF",
      icon: Printer,
      description: "Save as a PDF from the print dialog",
      onSelect: () => {
        toast({ title: "Preparing print view", tone: "info" });
        setTimeout(() => window.print(), 200);
      },
    },
  ];
  return (
    <DropdownMenu
      label="Export"
      align="end"
      items={items}
      className={extraItems.length ? "w-64" : "w-56"}
      trigger={(props, { open }) => (
        <button
          {...props}
          disabled={disabled}
          title={disabled ? disabledReason : undefined}
          className={cn(
            "no-print inline-flex h-10 w-full items-center justify-center gap-2 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-[filter] hover:brightness-105 sm:w-auto",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
            "disabled:pointer-events-none disabled:opacity-50",
          )}
        >
          <Download aria-hidden className="h-4 w-4" /> Export
          <ChevronDown aria-hidden className={cn("h-4 w-4 transition-transform motion-reduce:transition-none", open && "rotate-180")} />
        </button>
      )}
    />
  );
}
