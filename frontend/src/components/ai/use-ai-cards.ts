"use client";

import { useCallback, useSyncExternalStore } from "react";

/*
 * The AI insight cards' bodies (components/ai/ai-cards: the card shell, the
 * result body and footer, the request hook), loaded once per page view
 * through ONE dynamic import, like recharts (components/charts/use-recharts).
 * Each page keeps only a small gate in its own bundle — whether the server
 * allows the card, and the page context for Ask AI — so the shell is
 * shipped once instead of once per route, and a viewer who can't use AI
 * assist never downloads it at all.
 *
 * The server and the hydration pass always see "not loaded", so the markup
 * they agree on never depends on how fast the download was.
 */

type AiCards = typeof import("./ai-cards");

let cards: AiCards | null = null;
let pending: Promise<AiCards> | null = null;
const listeners = new Set<() => void>();

/** Load the cards (once); a failed download is tried again by the next caller. */
export function loadAiCards(): Promise<AiCards> {
  if (!pending) {
    pending = import("./ai-cards").then(
      (module) => {
        cards = module;
        listeners.forEach((notify) => notify());
        return module;
      },
      (error: unknown) => {
        pending = null;
        throw error;
      },
    );
  }
  return pending;
}

const currentCards = () => cards;
const notLoadedOnServer = () => null;

/**
 * The card bodies, or null while they download. Pass `wanted: false` while
 * the card is hidden for this viewer: nothing is downloaded until a card
 * would actually show.
 */
export function useAiCards(wanted: boolean): AiCards | null {
  const subscribe = useCallback(
    (onChange: () => void) => {
      listeners.add(onChange);
      if (wanted && !cards) {
        loadAiCards().catch(() => {
          // Offline or a failed deploy: the card stays away; the next mount retries.
        });
      }
      return () => {
        listeners.delete(onChange);
      };
    },
    [wanted],
  );
  const loaded = useSyncExternalStore(subscribe, currentCards, notLoadedOnServer);
  return wanted ? loaded : null;
}
