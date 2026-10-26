"use client";

import Link from "next/link";
import { Popover } from "@/components/ui/popover";
import { AI_COPY, WHATS_SENT, dataCategoryLabel } from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

function SentList({ label, items }: { label: string; items: readonly string[] }) {
  return (
    <div>
      <p className="font-semibold text-foreground">{label}</p>
      <ul className="mt-1 list-disc space-y-0.5 pl-4">
        {items.map((text) => (
          <li key={text}>{text}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * "What's sent?": what AI assist sends to the AI provider, and what it never
 * sends, plus — for a result — the kinds of data that result used. The
 * workspace's full disclosure lives in Settings > Privacy.
 */
export function AiDisclosurePopover({ dataCategories, className }: { dataCategories?: readonly string[]; className?: string }) {
  return (
    <Popover
      label="What AI assist sends"
      className="w-80 max-w-[calc(100vw-2rem)] p-4"
      trigger={(props) => (
        <button
          {...props}
          className={cn(
            "rounded text-xs font-medium text-primary underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
        >
          {AI_COPY.whatsSent}
        </button>
      )}
    >
      <div className="space-y-3 text-xs text-muted-foreground">
        {dataCategories && dataCategories.length > 0 && <SentList label="This result used" items={dataCategories.map(dataCategoryLabel)} />}
        <SentList label="Sent" items={WHATS_SENT.always} />
        <SentList label="Sent only when the owner allows it" items={WHATS_SENT.whenAllowed} />
        <SentList label="Never sent" items={WHATS_SENT.never} />
        <p>Names of members are replaced with aliases before anything leaves the server.</p>
        <Link href="/settings?tab=privacy" className="inline-block font-medium text-primary underline-offset-2 hover:underline">
          Privacy details
        </Link>
      </div>
    </Popover>
  );
}
