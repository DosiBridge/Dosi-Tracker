"use client";

import { useAiAssistant, useAiPageContext } from "@/components/ai/ai-provider";
import { useAiCards } from "@/components/ai/use-ai-cards";
import { aiAvailability } from "@/lib/ai-assist";

/**
 * "Explain this day": an AI summary of one member's day — what it went to,
 * the longest stretch, idle time, the rhythm — on the Monitor's detail panel
 * (AI assist spec §3.1, §18.5). `date` is the local day (yyyy-MM-dd) of
 * `timeZone`. A member's own day is asked for as their own (the server's
 * "self" scope); anyone else's needs the "member" scope, which only owners
 * and managers hold. Hidden otherwise; generated only on Generate. The card
 * itself downloads only once it would show (use-ai-cards).
 */
export function AiDaySummaryCard({ userId, date, timeZone, className }: { userId: string; date: string; timeZone: string; className?: string }) {
  const { status, userId: viewerId } = useAiAssistant();
  const self = userId === viewerId;
  const availability = aiAvailability(status, "member-day-summary", self ? "self" : "member");
  useAiPageContext(availability.visible ? { page: "monitor", memberId: userId, day: date, timeZone } : null);
  const cards = useAiCards(availability.visible);
  if (!availability.visible || !cards) return null;

  return (
    <cards.DaySummaryBody
      title="Explain this day"
      purpose={
        self
          ? "A short summary of your day: what it went to, the longest stretch and the breaks."
          : "A short summary of this member's day: what it went to, the longest stretch and the breaks."
      }
      // The caller's own day goes as `userId: null` (the contract's "the caller"), never as a member lookup.
      input={{ userId: self ? null : userId, date, timeZone }}
      availability={availability}
      timeZone={timeZone}
      className={className}
    />
  );
}
