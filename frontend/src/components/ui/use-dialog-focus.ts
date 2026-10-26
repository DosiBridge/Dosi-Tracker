"use client";

import { useEffect, useRef, type RefObject } from "react";
import { FLOATING_LAYER_SELECTOR, FOCUSABLE_SELECTOR as FOCUSABLE } from "./floating";

/*
 * Open dialogs, oldest first. Dialogs nest (a confirmation over an edit
 * form, a dialog over a drawer): the keyboard belongs to the newest one.
 * Escape closes only it, only it traps Tab, and the page behind stays
 * locked until the last one closes.
 */
const openDialogs: HTMLElement[] = [];

/** Whether `el` is the newest open dialog — the one Escape and Tab act on. */
export function isTopmostDialog(el: HTMLElement | null): boolean {
  return el !== null && openDialogs[openDialogs.length - 1] === el;
}

let scrollLocks = 0;
let overflowBefore = "";

/**
 * Stop the page behind scrolling while a dialog is open; returns the release.
 * Counted, so a nested dialog closing leaves the page locked under the one
 * still open.
 */
export function lockBodyScroll(): () => void {
  if (scrollLocks++ === 0) {
    overflowBefore = document.body.style.overflow;
    document.body.style.overflow = "hidden";
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    if (--scrollLocks === 0) document.body.style.overflow = overflowBefore;
  };
}

/**
 * Focus management for a modal dialog: moves focus into the dialog when it
 * opens, keeps Tab/Shift+Tab cycling inside it while open, and returns focus
 * to whatever was focused before on close.
 *
 * `aria-modal` alone only tells screen readers that the page behind is inert —
 * it does not stop the keyboard. Without this, a keyboard user tabs straight
 * out of an open dialog and onto the page underneath, with no visible way back.
 *
 * Attach the returned ref to the dialog element, which should also carry
 * `tabIndex={-1}` so it can hold focus when it contains no focusable children.
 * `initialFocus` names the control to start on (a composer, say) instead of
 * the first one; it is skipped while disabled or outside the dialog.
 */
export function useDialogFocus<T extends HTMLElement>(open: boolean, initialFocus?: RefObject<HTMLElement | null>) {
  const ref = useRef<T>(null);

  useEffect(() => {
    if (!open) return;
    const dialog = ref.current;
    if (!dialog) return;

    const restoreTo = document.activeElement as HTMLElement | null;
    openDialogs.push(dialog);

    // Make everything outside the dialog inert while it is open. The focus trap
    // below handles Tab, but a screen reader's virtual cursor ignores focus
    // order entirely — without this it reads straight through the page behind
    // the overlay with no sign a dialog is open.
    const siblings = Array.from(document.body.children).filter(
      (el) => el !== dialog && !el.contains(dialog) && el.tagName !== "SCRIPT",
    ) as HTMLElement[];
    const previouslyInert = siblings.map((el) => el.hasAttribute("inert"));
    siblings.forEach((el) => el.setAttribute("inert", ""));

    // Recomputed on every Tab: dialog content is dynamic (wizard steps swap
    // their controls, lists load in), so a snapshot taken on open goes stale.
    const focusable = () =>
      Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (el) => !el.hasAttribute("disabled") && el.getAttribute("aria-hidden") !== "true",
      );

    const preferred = initialFocus?.current;
    const start = preferred && dialog.contains(preferred) && !preferred.hasAttribute("disabled") ? preferred : focusable()[0];
    (start ?? dialog).focus();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      // A dialog opened over this one keeps Tab inside itself.
      if (!isTopmostDialog(dialog)) return;
      // A menu or popover opened from inside the dialog is portaled to <body>
      // and handles Tab itself; pulling focus back here would strand it.
      const current = document.activeElement;
      if (current instanceof Element && current.closest(FLOATING_LAYER_SELECTOR)) return;
      const items = focusable();
      if (items.length === 0) {
        e.preventDefault();
        dialog.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      const outside = !dialog.contains(active);

      if (e.shiftKey && (active === first || outside)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || outside)) {
        e.preventDefault();
        first.focus();
      }
    };

    // Capture phase so the trap wins over anything inside the dialog.
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      const index = openDialogs.lastIndexOf(dialog);
      if (index !== -1) openDialogs.splice(index, 1);
      siblings.forEach((el, i) => {
        if (!previouslyInert[i]) el.removeAttribute("inert");
      });
      // Only restore if focus is still somewhere we put it; if the app moved
      // focus deliberately on close, leave it alone.
      if (restoreTo?.isConnected) restoreTo.focus();
    };
  }, [open, initialFocus]);

  return ref;
}
