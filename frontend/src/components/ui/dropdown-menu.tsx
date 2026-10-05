"use client";

import { useId } from "react";
import type { LucideIcon } from "lucide-react";
import { Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { Popover, type FocusIntent, type PopoverProps } from "./popover";

/* ============================================================================
 * DROPDOWN MENU — a button that opens a list of actions or choices, in the
 * look of the report menus (Export, period picker): a rounded card of rows
 * with an icon, an optional description, and a check on the chosen one.
 *
 * Keyboard (WAI-ARIA menu button): Enter, Space or ↓ on the button opens it
 * on the first item (the chosen one, in a list of choices); ↑ opens it on the
 * last. ↓ ↑ Home End move, a letter jumps to the next item starting with it,
 * Enter or Space picks, Escape closes back to the button, Tab closes and moves
 * on. Built on Popover, so it never opens off-screen or under the sidebar.
 * ========================================================================== */

export interface DropdownMenuItem {
  kind?: "item";
  /** Stable key, unique in the menu. */
  key: string;
  label: string;
  onSelect: () => void;
  icon?: LucideIcon;
  /** A second, quieter line under the label. */
  description?: string;
  /** Shown but not choosable (announced as unavailable). */
  disabled?: boolean;
  /** In a list of choices: whether this one is chosen (a check, announced as checked). Leave undefined for actions. */
  checked?: boolean;
  /** "danger" for destructive actions (remove, delete). */
  tone?: "default" | "danger";
}

/** A line between groups of items. */
export interface DropdownMenuSeparator {
  kind: "separator";
  key: string;
}

/** A small heading naming the items after it (up to the next heading or separator). */
export interface DropdownMenuHeading {
  kind: "heading";
  key: string;
  label: string;
}

export type DropdownMenuEntry = DropdownMenuItem | DropdownMenuSeparator | DropdownMenuHeading;

export interface DropdownMenuProps
  extends Pick<PopoverProps, "trigger" | "label" | "open" | "defaultOpen" | "onOpenChange" | "side" | "align" | "offset"> {
  items: readonly DropdownMenuEntry[];
  /** Classes for the menu panel (default width "w-56"). */
  className?: string;
}

const ITEM_SELECTOR = "[data-menu-item]";

function isItem(entry: DropdownMenuEntry): entry is DropdownMenuItem {
  return entry.kind === undefined || entry.kind === "item";
}

/** The items of an open menu, in order. */
function menuItems(panel: HTMLElement): HTMLElement[] {
  return Array.from(panel.querySelectorAll<HTMLElement>(ITEM_SELECTOR));
}

/**
 * What takes focus when the menu opens. From the keyboard: the chosen item,
 * else the first (the last when opened with ↑). With a pointer: the menu
 * itself, so no item looks picked before the pointer reaches one.
 */
function initialFocus(panel: HTMLElement, intent: FocusIntent): HTMLElement | null {
  if (intent === "pointer") return panel;
  const items = menuItems(panel);
  if (intent === "last") return items[items.length - 1] ?? panel;
  return items.find((el) => el.getAttribute("aria-checked") === "true") ?? items[0] ?? panel;
}

/** ↓ ↑ Home End between the items, and a letter to jump to the next item starting with it. */
function onMenuKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const items = menuItems(event.currentTarget);
  if (items.length === 0) return;
  const current = items.indexOf(document.activeElement as HTMLElement);
  const go = (index: number) => {
    event.preventDefault();
    items[(index + items.length) % items.length].focus();
  };
  switch (event.key) {
    case "ArrowDown":
      go(current + 1);
      return;
    case "ArrowUp":
      go(current === -1 ? items.length - 1 : current - 1);
      return;
    case "Home":
      go(0);
      return;
    case "End":
      go(items.length - 1);
      return;
  }
  if (event.key.length !== 1 || event.key === " ") return;
  const letter = event.key.toLocaleLowerCase();
  for (let step = 1; step <= items.length; step++) {
    const index = (current + step + items.length) % items.length;
    if ((items[index].dataset.label ?? "").toLocaleLowerCase().startsWith(letter)) {
      go(index);
      return;
    }
  }
}

interface Section {
  key: string;
  heading: DropdownMenuHeading | null;
  items: DropdownMenuItem[];
}

/** The entries as sections: a heading or a separator starts a new one; empty ones are dropped. */
function toSections(entries: readonly DropdownMenuEntry[]): Section[] {
  const sections: Section[] = [];
  for (const entry of entries) {
    if (entry.kind === "separator" || entry.kind === "heading") {
      sections.push({ key: entry.key, heading: entry.kind === "heading" ? entry : null, items: [] });
    } else if (isItem(entry)) {
      if (sections.length === 0) sections.push({ key: `${entry.key}-first`, heading: null, items: [] });
      sections[sections.length - 1].items.push(entry);
    }
  }
  return sections.filter((s) => s.items.length > 0);
}

export function DropdownMenu({ items, className, ...popover }: DropdownMenuProps) {
  const id = useId();
  const checkable = items.some((entry) => isItem(entry) && entry.checked !== undefined);
  const sections = toSections(items);

  return (
    <Popover {...popover} role="menu" initialFocus={initialFocus} onKeyDown={onMenuKeyDown} className={cn("w-56 p-1", className)}>
      {({ close }) =>
        sections.map((section, index) => {
          const rows = section.items.map((item) => {
            const Icon = item.icon;
            const descriptionId = item.description ? `${id}${item.key}-description` : undefined;
            return (
              <button
                key={item.key}
                type="button"
                role={checkable ? "menuitemradio" : "menuitem"}
                aria-checked={checkable ? item.checked === true : undefined}
                aria-disabled={item.disabled || undefined}
                // The label alone names the item; its second line is read as a description.
                aria-label={descriptionId ? item.label : undefined}
                aria-describedby={descriptionId}
                tabIndex={-1}
                data-menu-item=""
                data-label={item.label}
                onClick={() => {
                  if (item.disabled) return;
                  close("select");
                  item.onSelect();
                }}
                onPointerMove={(event) => {
                  // Pointer and keyboard share one highlight: hovering an item focuses it.
                  if (!item.disabled && document.activeElement !== event.currentTarget) {
                    event.currentTarget.focus({ preventScroll: true });
                  }
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm outline-none transition-colors",
                  "hover:bg-muted focus:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                  item.tone === "danger" ? "text-danger-on-tint" : item.checked ? "font-medium text-primary-strong" : "text-foreground",
                  item.disabled && "cursor-not-allowed opacity-50 hover:bg-transparent",
                )}
              >
                {Icon && (
                  <Icon aria-hidden className={cn("h-4 w-4 shrink-0", item.tone === "danger" ? "text-danger-on-tint" : "text-muted-foreground")} />
                )}
                <span className="min-w-0 flex-1">
                  <span className="block">{item.label}</span>
                  {item.description && (
                    <span id={descriptionId} className="block text-xs font-normal text-muted-foreground">
                      {item.description}
                    </span>
                  )}
                </span>
                {item.checked && <Check aria-hidden className="h-4 w-4 shrink-0" />}
              </button>
            );
          });

          if (!section.heading) {
            return (
              <div key={section.key} role="none">
                {index > 0 && <div role="separator" className="-mx-1 my-1 h-px bg-border" />}
                {rows}
              </div>
            );
          }
          const headingId = `${id}${section.key}`;
          return (
            <div key={section.key} role="group" aria-labelledby={headingId}>
              {index > 0 && <div role="separator" className="-mx-1 my-1 h-px bg-border" />}
              <div id={headingId} className="px-3 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                {section.heading.label}
              </div>
              {rows}
            </div>
          );
        })
      }
    </Popover>
  );
}
