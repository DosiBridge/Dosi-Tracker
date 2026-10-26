"use client";

import { useAiAssistant } from "@/components/ai/ai-provider";
import { useAiCards } from "@/components/ai/use-ai-cards";
import { aiAvailability } from "@/lib/ai-assist";

/**
 * "Suggest" in a block's note editor (note-suggestions, AI assist spec §3.1,
 * §18.4): up to three short notes drawn from the block's apps, project and
 * the member's previous note. Choosing one only fills the draft — the
 * person edits it, and only the editor's own "Save note" stores anything
 * (PUT /api/app/activity/{id}/note, with its usual rules).
 *
 * Hidden unless the server allows this caller note suggestions; the editor
 * renders it only while the viewer may edit the note. The suggestions
 * themselves download only once it would show.
 */
export function NoteSuggestions({ activityId, onPick, disabled = false }: { activityId: string; onPick: (text: string) => void; disabled?: boolean }) {
  const { status } = useAiAssistant();
  const availability = aiAvailability(status, "note-suggestions");
  const cards = useAiCards(availability.visible);
  if (!availability.visible || !cards) return null;
  return <cards.NoteSuggestionsBody activityId={activityId} onPick={onPick} disabled={disabled} />;
}
