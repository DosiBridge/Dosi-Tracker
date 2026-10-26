"use client";

import { Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { aiAssistantMode } from "@/lib/ai-assist";

/**
 * "Ask the setup assistant" (AI assist spec §3.1, §18.5): opens Ask AI with
 * the setup questions (installing the agent, what AI assist sends), which the
 * server answers from its curated help articles. Shown wherever the Ask AI
 * launcher is.
 */
export function AiSetupHelpButton({ className }: { className?: string }) {
  const { status, role, openAssistant } = useAiAssistant();
  if (!aiAssistantMode(status, role)) return null;
  return (
    <Button type="button" variant="outline" size="sm" className={className} onClick={() => openAssistant({ chips: "setup" })}>
      <Sparkles aria-hidden className="h-3.5 w-3.5" /> Ask the setup assistant
    </Button>
  );
}
