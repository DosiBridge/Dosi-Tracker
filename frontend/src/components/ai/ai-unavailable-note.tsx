import { Info } from "lucide-react";
import { AI_COPY } from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

/**
 * A quiet note where an AI surface would be: by default the owner's
 * "AI assist isn't available on this server right now." in Settings, and the
 * quota note on a card. Plain text, never a button that would only fail.
 */
export function AiUnavailableNote({ message = AI_COPY.serverUnavailable, className }: { message?: string; className?: string }) {
  return (
    <p role="note" className={cn("flex items-start gap-2 rounded-xl bg-muted/60 px-3 py-2 text-xs text-muted-foreground", className)}>
      <Info aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span>{message}</span>
    </p>
  );
}
