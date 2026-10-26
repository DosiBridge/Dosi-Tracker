"use client";

import { useEffect } from "react";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { loadAiCards } from "@/components/ai/use-ai-cards";

/**
 * Starts downloading the AI insight cards (use-ai-cards) once the status says
 * AI assist is usable in this workspace. Mounted by the dashboard layout, so
 * the cards' one dynamic import sits in the layout's bundle and the build
 * keeps ONE copy of them for every page, instead of one per route. A viewer
 * without AI assist never downloads them. Renders nothing.
 */
export function AiCardsPreload() {
  const { status } = useAiAssistant();
  const usable = !!status && (status.available || status.reason === "quota-exhausted");
  useEffect(() => {
    if (!usable) return;
    loadAiCards().catch(() => {
      // Offline or a failed deploy: the cards stay away; the next card that mounts retries.
    });
  }, [usable]);
  return null;
}
