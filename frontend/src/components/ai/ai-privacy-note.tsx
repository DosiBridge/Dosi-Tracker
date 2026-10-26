"use client";

import { useId } from "react";
import Link from "next/link";
import { Sparkles } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { WHATS_SENT, aiStatusLine, allowedAiFeatures } from "@/lib/ai-assist";

function SentList({ label, items }: { label: string; items: readonly string[] }) {
  return (
    <div>
      <h3 className="text-sm font-semibold">{label}</h3>
      <ul className="mt-1 list-disc space-y-0.5 pl-5 text-xs leading-relaxed text-muted-foreground">
        {items.map((text) => (
          <li key={text}>{text}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * AI assist on the Privacy tab (owner, manager, member; AI assist spec
 * §18.5): whether it is on here, who can use it and what this viewer can,
 * what of their data may be sent, what never is, and — for the owner — the
 * way to the full disclosure and its switches. Read-only text: nothing here
 * changes a setting.
 */
export function AiPrivacyNote() {
  const headingId = useId();
  const { status, statusLoading, live, role } = useAiAssistant();
  const checking = live && statusLoading && !status;
  const line = aiStatusLine(status, { demo: !live });
  const usable = !!status && (status.available || status.reason === "quota-exhausted");
  const mine = usable ? allowedAiFeatures(status) : [];
  const included = (on: boolean | undefined) => (status ? (on ? " (included in this workspace)" : " (not included in this workspace)") : "");
  const whenAllowed = [`${WHATS_SENT.whenAllowed[0]}${included(status?.allowWindowTitles)}`, `${WHATS_SENT.whenAllowed[1]}${included(status?.allowNotes)}`];

  return (
    <Card>
      <section aria-labelledby={headingId}>
        <CardHeader>
          <div className="flex flex-wrap items-center gap-2">
            <Sparkles aria-hidden className="h-4 w-4 shrink-0 text-primary" />
            <CardTitle id={headingId}>AI assist</CardTitle>
            {!checking && <Badge tone={line.tone}>{line.label}</Badge>}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {checking ? (
            <p role="status" className="text-sm text-muted-foreground">
              Checking AI assist…
            </p>
          ) : (
            <p className="text-sm text-muted-foreground">{line.text}</p>
          )}
          <div className="space-y-1">
            <h3 className="text-sm font-semibold">Who can use it</h3>
            <p className="text-xs leading-relaxed text-muted-foreground">
              The workspace owner decides whether AI assist is on. When it is, managers can use it if the owner allows, and members only on their own
              data. Nothing is written until someone presses Generate.
            </p>
            {usable && (
              <p className="text-xs text-muted-foreground">
                {mine.length > 0 ? (
                  <>
                    You can use: <span className="font-medium text-foreground">{mine.join(", ")}</span>.
                  </>
                ) : (
                  "No AI feature is available to you in this workspace."
                )}
              </p>
            )}
          </div>
          <SentList label="What may be sent" items={WHATS_SENT.always} />
          <SentList label="Sent only when the owner allows it" items={whenAllowed} />
          <SentList label="Never sent" items={WHATS_SENT.never} />
          <p className="text-xs text-muted-foreground">
            Names of members are replaced with aliases before anything leaves the server. AI output is labelled and can be wrong; it is never meant
            for decisions about people on its own.
          </p>
          {role === "owner" && (
            <Link href="/settings?tab=workspace" className="inline-block text-xs font-medium text-primary-strong underline underline-offset-2">
              Read the full AI disclosure and change the AI settings
            </Link>
          )}
        </CardContent>
      </section>
    </Card>
  );
}
