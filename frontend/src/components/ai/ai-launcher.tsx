"use client";

import { Sparkles } from "lucide-react";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { aiAssistantMode } from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

/**
 * The topbar's Ask AI button (AI assist spec §18.3/§18.5): shown when the
 * server allows this caller Ask AI, and to the owner of a workspace that
 * hasn't opted in (the drawer then explains AI assist and offers to turn it
 * on). Hidden everywhere else. It adds no keyboard shortcut of its own:
 * Ctrl+J is the browser's Downloads, and the command palette already offers
 * "Ask AI...".
 */
export function AiLauncher({ className }: { className?: string }) {
  const { status, role, drawerOpen, openAssistant } = useAiAssistant();
  if (!aiAssistantMode(status, role)) return null;
  return (
    <button
      type="button"
      aria-label="Ask AI"
      title="Ask AI"
      aria-haspopup="dialog"
      aria-expanded={drawerOpen}
      onClick={() => openAssistant()}
      className={cn(className)}
    >
      <Sparkles aria-hidden className="h-4 w-4" />
    </button>
  );
}
