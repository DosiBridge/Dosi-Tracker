"use client";

import { useCallback, useEffect, useEffectEvent, useId, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";
import {
  FOCUSABLE_SELECTOR,
  computeFloatingPosition,
  floatingOrigin,
  type FloatingAlign,
  type FloatingSide,
} from "./floating";

/* ============================================================================
 * POPOVER — a panel anchored to the button that opens it (menus, small forms).
 *
 * The panel is portaled to <body> and positioned `fixed` above everything
 * else on the page (the sidebar included), flipping to the side with room and
 * sliding inside the viewport (see floating.ts). It closes on Escape, on a
 * press outside it, and when its trigger is pressed again; Escape and an
 * explicit close hand focus back to the trigger.
 *
 * Open panels form a stack: Escape closes only the newest, and a press inside
 * a panel opened from this one doesn't count as "outside". A dialog's focus
 * trap leaves Tab to a panel opened from inside it.
 * ========================================================================== */

/** Why a panel closed. */
export type DismissReason = "escape" | "outside" | "trigger" | "tab" | "select" | "close";

/**
 * How the panel was opened, for choosing what takes focus: from the keyboard
 * ("first"; "last" for a menu opened with ↑) or with a pointer ("pointer").
 */
export type FocusIntent = "first" | "last" | "pointer";

/** Spread ALL of these onto the <button> (or Button) that opens the panel. */
export interface PopoverTriggerProps {
  /** How the popover finds its trigger (to anchor the panel and return focus): don't override it. */
  id: string;
  type: "button";
  "aria-haspopup": "dialog" | "menu";
  "aria-expanded": boolean;
  "aria-controls": string | undefined;
  onClick: (event: React.MouseEvent<HTMLElement>) => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLElement>) => void;
}

export interface PopoverApi {
  /** Close the panel and return focus to its trigger. */
  close: (reason?: DismissReason) => void;
}

export interface PopoverProps {
  /** Renders the control that opens the panel; spread `props` onto it. */
  trigger: (props: PopoverTriggerProps, state: { open: boolean }) => React.ReactNode;
  /** The panel's content; as a function it receives `close` (e.g. for an Apply button). */
  children: React.ReactNode | ((api: PopoverApi) => React.ReactNode);
  /** The panel's accessible name; without one it is named by its trigger. */
  label?: string;
  /** "dialog" (default) traps Tab inside the panel; "menu" closes on Tab and opens from the trigger with ↓/↑. */
  role?: "dialog" | "menu";
  /** Controlled open state; leave undefined to let the popover manage it. */
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Preferred side (default "bottom"); flips when the other side has more room. */
  side?: FloatingSide;
  /** Preferred alignment with the trigger (default "start"). */
  align?: FloatingAlign;
  /** Gap between trigger and panel, px (default 8). */
  offset?: number;
  /** Classes for the panel: width and padding (e.g. "w-72 p-3"). */
  className?: string;
  /** The element to focus on open; default the panel's first focusable element (else the panel). */
  initialFocus?: (panel: HTMLElement, intent: FocusIntent) => HTMLElement | null | undefined;
  /** Key presses inside the panel, before the popover's own Tab handling (menus add arrow keys). */
  onKeyDown?: (event: React.KeyboardEvent<HTMLDivElement>) => void;
}

/* ----------------------------- The open layers ----------------------------- */

interface Layer {
  /** Whether a node is inside this layer's panel or trigger. */
  contains: (node: Node) => boolean;
}

/** Open panels, oldest first. Module state: only effects touch it, never render. */
const openLayers: Layer[] = [];

/* ----------------------------- Helpers ----------------------------- */

const subscribeNothing = () => () => {};
const onClient = () => true;
const onServer = () => false;

/** The focusable elements inside `root`, in order. */
export function focusableIn(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (el) => !el.hasAttribute("disabled") && el.getAttribute("aria-hidden") !== "true",
  );
}

/** The viewport a `fixed` panel is laid out in (without scrollbars). */
function viewportSize(): { width: number; height: number } {
  const root = document.documentElement;
  return { width: root.clientWidth || window.innerWidth, height: root.clientHeight || window.innerHeight };
}

/* ----------------------------- Popover ----------------------------- */

export function Popover({
  trigger,
  children,
  label,
  role = "dialog",
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  side = "bottom",
  align = "start",
  offset = 8,
  className,
  initialFocus,
  onKeyDown,
}: PopoverProps) {
  const id = useId();
  const triggerId = `${id}trigger`;
  const panelId = `${id}panel`;
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const open = openProp ?? uncontrolledOpen;
  const [intent, setIntent] = useState<FocusIntent>("first");
  const panelRef = useRef<HTMLDivElement>(null);
  // A portal needs <body>: render the panel only once mounted in the browser.
  const mounted = useSyncExternalStore(subscribeNothing, onClient, onServer);

  const setOpen = useCallback(
    (next: boolean) => {
      if (openProp === undefined) setUncontrolledOpen(next);
      onOpenChange?.(next);
    },
    [openProp, onOpenChange],
  );

  const close = useCallback(
    (reason: DismissReason = "close") => {
      // Focus goes back to the trigger first, so whatever the close opens next
      // (a dialog) returns there too. A press outside leaves focus where it landed.
      if (reason !== "outside") document.getElementById(triggerId)?.focus({ preventScroll: true });
      setOpen(false);
    },
    [setOpen, triggerId],
  );

  /* Escape and presses outside, for the newest open panel. */
  const dismiss = useEffectEvent((reason: DismissReason) => close(reason));
  useEffect(() => {
    if (!open) return;
    const layer: Layer = {
      contains: (node) => !!(panelRef.current?.contains(node) || document.getElementById(triggerId)?.contains(node)),
    };
    openLayers.push(layer);
    // Whether focus is in the panel, so a close its owner makes (a controlled `open`) can hand it back.
    let focusWithin = false;
    const onDocumentFocusIn = (event: FocusEvent) => {
      focusWithin = event.target instanceof Node && !!panelRef.current?.contains(event.target);
    };
    const onDocumentKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || openLayers[openLayers.length - 1] !== layer) return;
      // Consumed here, so a dialog underneath stays open.
      event.preventDefault();
      event.stopPropagation();
      dismiss("escape");
    };
    const onDocumentPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      // Inside this panel, its trigger, or a panel opened from it: not outside.
      if (openLayers.slice(openLayers.indexOf(layer)).some((l) => l.contains(target))) return;
      dismiss("outside");
    };
    document.addEventListener("focusin", onDocumentFocusIn, true);
    document.addEventListener("keydown", onDocumentKeyDown, true);
    document.addEventListener("pointerdown", onDocumentPointerDown, true);
    return () => {
      document.removeEventListener("focusin", onDocumentFocusIn, true);
      document.removeEventListener("keydown", onDocumentKeyDown, true);
      document.removeEventListener("pointerdown", onDocumentPointerDown, true);
      const index = openLayers.indexOf(layer);
      if (index !== -1) openLayers.splice(index, 1);
      // The panel went away with focus inside it: don't strand focus on <body>.
      if (focusWithin && (!document.activeElement || document.activeElement === document.body)) {
        document.getElementById(triggerId)?.focus({ preventScroll: true });
      }
    };
  }, [open, triggerId]);

  /* Position: before paint, and again whenever the page scrolls, resizes or the panel's content changes size. */
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!open || !mounted || !panel) return;
    const place = () => {
      // The natural width is measured without the cap; the natural height
      // comes from scrollHeight, so the cap (and the scroll inside it) stays put.
      panel.style.maxWidth = "";
      const anchor = document.getElementById(triggerId);
      const position = computeFloatingPosition({
        // A trigger that can't be found (its id overridden) still gets a visible panel, at the corner.
        anchor: anchor ? anchor.getBoundingClientRect() : { top: 0, left: 0, width: 0, height: 0 },
        floating: { width: panel.offsetWidth, height: panel.scrollHeight + (panel.offsetHeight - panel.clientHeight) },
        viewport: viewportSize(),
        side,
        align,
        offset,
      });
      panel.style.top = `${position.top}px`;
      panel.style.left = `${position.left}px`;
      panel.style.maxHeight = `${position.maxHeight}px`;
      panel.style.maxWidth = `${position.maxWidth}px`;
      panel.style.transformOrigin = floatingOrigin(position);
      panel.dataset.side = position.side;
      panel.dataset.align = position.align;
      panel.style.visibility = "visible";
    };
    const onScroll = (event: Event) => {
      // Scrolling inside the panel moves nothing.
      if (event.target instanceof Node && panel.contains(event.target)) return;
      place();
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", onScroll, { capture: true, passive: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => place());
    observer?.observe(panel);
    const anchor = document.getElementById(triggerId);
    if (anchor) observer?.observe(anchor);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", onScroll, { capture: true });
      observer?.disconnect();
    };
  }, [open, mounted, side, align, offset, triggerId]);

  /* Focus moves into the panel once it is placed (and visible). */
  const focusInitial = useEffectEvent((panel: HTMLElement) => {
    const target = initialFocus?.(panel, intent) ?? focusableIn(panel)[0] ?? panel;
    target.focus({ preventScroll: true });
  });
  useEffect(() => {
    const panel = panelRef.current;
    if (open && mounted && panel) focusInitial(panel);
  }, [open, mounted]);

  function onPanelKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    onKeyDown?.(event);
    if (event.defaultPrevented || event.key !== "Tab") return;
    if (role === "menu") {
      // Tab leaves a menu: focus returns to the trigger and the browser's Tab moves on from there.
      close("tab");
      return;
    }
    // A dialog keeps Tab inside until it closes.
    const items = focusableIn(event.currentTarget);
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === event.currentTarget)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  const triggerProps: PopoverTriggerProps = {
    id: triggerId,
    type: "button",
    "aria-haspopup": role,
    "aria-expanded": open,
    "aria-controls": open ? panelId : undefined,
    onClick: (event) => {
      if (open) {
        close("trigger");
      } else {
        // A click from Enter or Space carries no pointer detail.
        setIntent(event.detail === 0 ? "first" : "pointer");
        setOpen(true);
      }
    },
    onKeyDown: (event) => {
      if (role !== "menu" || open || (event.key !== "ArrowDown" && event.key !== "ArrowUp")) return;
      event.preventDefault();
      setIntent(event.key === "ArrowUp" ? "last" : "first");
      setOpen(true);
    },
  };

  const panel =
    open && mounted
      ? createPortal(
          <div
            ref={panelRef}
            id={panelId}
            role={role}
            aria-label={label}
            aria-labelledby={label ? undefined : triggerId}
            tabIndex={-1}
            // FLOATING_LAYER_ATTRIBUTE: a dialog's focus trap leaves Tab inside this panel alone.
            data-floating-layer=""
            onKeyDown={onPanelKeyDown}
            // Hidden until placed (before the first paint), so it never flashes at the corner.
            style={{ top: 0, left: 0, visibility: "hidden" }}
            className={cn(
              "no-print fixed z-[60] overflow-y-auto overscroll-contain rounded-xl border border-border bg-card text-card-foreground card-elev-lg outline-none animate-scale-in",
              className,
            )}
          >
            {typeof children === "function" ? children({ close }) : children}
          </div>,
          document.body,
        )
      : null;

  return (
    <>
      {trigger(triggerProps, { open })}
      {panel}
    </>
  );
}
