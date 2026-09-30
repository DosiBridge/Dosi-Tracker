"use client";

import { useEffect, useRef } from "react";

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

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
 */
export function useDialogFocus<T extends HTMLElement>(open: boolean) {
  const ref = useRef<T>(null);

  useEffect(() => {
    if (!open) return;
    const dialog = ref.current;
    if (!dialog) return;

    const restoreTo = document.activeElement as HTMLElement | null;

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

    (focusable()[0] ?? dialog).focus();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
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
      siblings.forEach((el, i) => {
        if (!previouslyInert[i]) el.removeAttribute("inert");
      });
      // Only restore if focus is still somewhere we put it; if the app moved
      // focus deliberately on close, leave it alone.
      if (restoreTo?.isConnected) restoreTo.focus();
    };
  }, [open]);

  return ref;
}
