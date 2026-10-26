"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Loader2, RotateCcw, Send, Sparkles, Square } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Drawer } from "@/components/ui/drawer";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { toast } from "@/components/toast";
import { AiAnswer } from "@/components/ai/ai-answer";
import { useAiAssistant, useAiPageContextValue } from "@/components/ai/ai-provider";
import { AiUnavailableNote } from "@/components/ai/ai-unavailable-note";
import { useAiChat } from "@/hooks/use-ai-chat";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import type { AiPageContextDto } from "@/lib/ai-contract";
import {
  AI_ANSWER_LABEL,
  AI_PROMPT_CHIPS,
  AI_QUESTION_MAX_LENGTH,
  AI_SETUP_HELP_CHIPS,
  aiAskScopeLabel,
  aiAssistantMode,
  aiPageForPath,
  quotaNote,
  type AiPromptChip,
} from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

/** Under the drawer's heading in a demo: its answers are samples, not AI output. */
const DEMO_HEADER_NOTE = "Sample answers from the demo data";

/** The demo composer's explanation (§18.7). */
const DEMO_COMPOSER_NOTE = "In the demo, Ask AI answers the suggested questions above; typing your own question needs a live workspace.";

const chipClass =
  "rounded-full border border-border px-3 py-1 text-left text-xs font-medium transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50";

/**
 * Ask AI (AI assist spec §9.2, §18.4): a drawer to ask questions about the
 * workspace's tracked time, with what the page shows sent along so "this
 * week" or "this member" mean what the person is looking at. Non-streaming:
 * each answer arrives whole, is announced politely, and never takes focus.
 *
 * It holds the scope chip (whose activity the assistant can look up), the
 * page's suggested questions (or the setup assistant's), the transcript, a
 * composer (1000 characters) with Send, Stop and New chat, and "What is
 * shared?". For the owner of a workspace that hasn't opted in, it explains
 * AI assist and offers "Review and turn on" instead. A demo answers the
 * suggested questions from the demo data and keeps the composer off.
 *
 * Loaded on demand (components/ai/ai-assistant-mount) the first time it
 * opens, and remounted per workspace, so a conversation never crosses one.
 */
export function AiAssistantDrawer() {
  const { status, live, role, userId, workspaceId, drawerOpen, setDrawerOpen, drawerRequest } = useAiAssistant();
  const pathname = usePathname();
  const registered = useAiPageContextValue();
  const timeZone = useBrowserTimeZone();
  const mode = aiAssistantMode(status, role);
  const page = registered?.page ?? aiPageForPath(pathname);
  const context = useMemo<AiPageContextDto>(() => registered ?? { page, timeZone }, [registered, page, timeZone]);
  const chips: readonly AiPromptChip[] = drawerRequest.chips === "setup" ? AI_SETUP_HELP_CHIPS : AI_PROMPT_CHIPS[page];
  const onRestarted = useCallback(
    () => toast({ title: "Started a new conversation", description: "The previous one had ended, so your question was asked again." }),
    [],
  );
  const chat = useAiChat({ live, workspaceId, demoContext: { userId, timeZone }, timeZone, onRestarted });
  const [draft, setDraft] = useState("");
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const firstChipRef = useRef<HTMLButtonElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const handledRef = useRef(0);
  const ids = useId();
  const close = useCallback(() => setDrawerOpen(false), [setDrawerOpen]);

  const usable = mode === "ready" || mode === "quota";
  const sending = chat.phase === "sending";

  // A suggested question the drawer was opened with (the command palette) is asked once.
  useEffect(() => {
    if (!drawerOpen || drawerRequest.seq === handledRef.current) return;
    handledRef.current = drawerRequest.seq;
    if (usable && drawerRequest.prompt) chat.send(drawerRequest.prompt.label, context, drawerRequest.prompt.id);
  }, [drawerOpen, drawerRequest, usable, chat, context]);

  // Keep the newest turn in view (scrolling only: focus stays where it is).
  const turnCount = chat.turns.length;
  useEffect(() => {
    if (turnCount > 0) endRef.current?.scrollIntoView({ block: "nearest" });
  }, [turnCount]);

  if (!mode) return null;

  const ask = (chip: AiPromptChip) => chat.send(chip.label, context, chip.id);
  const submit = () => {
    if (!live || !draft.trim()) return;
    chat.send(draft, context);
    setDraft("");
  };
  const announcement = sending ? "Ask AI is answering" : chat.phase === "done" ? "Answer ready" : "";
  const composerId = `${ids}composer`;
  const counterId = `${ids}counter`;
  const demoNoteId = `${ids}demo`;

  return (
    <Drawer open={drawerOpen} onClose={close} title="Ask AI" initialFocus={usable ? (live ? composerRef : firstChipRef) : undefined}>
      {mode === "turn-on" ? (
        <div className="space-y-4 text-sm">
          <p className="text-muted-foreground">
            AI assist is off in this workspace. Once you turn it on, Ask AI answers questions about your team&apos;s tracked time
            from the tracker&apos;s own figures. You&apos;ll see exactly what is sent to the AI provider before anything is.
          </p>
          <Link href="/settings?tab=workspace" onClick={close} className={linkButtonClass("primary", "md")}>
            <Sparkles aria-hidden className="h-4 w-4" /> Review and turn on
          </Link>
        </div>
      ) : (
        <div className="space-y-5">
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{live ? AI_ANSWER_LABEL : DEMO_HEADER_NOTE}</span>
            <Badge tone="muted">
              <span className="sr-only">Looks at: </span>
              {aiAskScopeLabel(status, role)}
            </Badge>
          </div>
          {mode === "quota" && <AiUnavailableNote message={quotaNote(status?.quota?.resetsAt, timeZone)} />}

          <div role="group" aria-label="Suggested questions" className="flex flex-wrap gap-2">
            {chips.map((chip, i) => (
              <button
                key={chip.id}
                ref={i === 0 ? firstChipRef : undefined}
                type="button"
                className={chipClass}
                disabled={sending}
                onClick={() => ask(chip)}
              >
                {chip.label}
              </button>
            ))}
          </div>

          <div role="log" aria-label="Conversation" aria-live="polite" className="space-y-4">
            {chat.turns.map((turn) =>
              turn.role === "user" ? (
                <div key={turn.id} className="ml-8 rounded-2xl bg-accent px-3 py-2 text-sm text-accent-foreground">
                  <span className="sr-only">You asked: </span>
                  {turn.text}
                </div>
              ) : (
                <div key={turn.id} className="rounded-2xl border border-border p-3">
                  <span className="sr-only">Ask AI answered: </span>
                  <AiAnswer text={turn.text} sources={turn.sources} facts={turn.facts} isClarification={turn.isClarification} meta={turn.meta} />
                </div>
              ),
            )}
          </div>
          {sending && (
            <p aria-hidden className="flex items-center gap-2 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 motion-safe:animate-spin" /> Writing an answer…
            </p>
          )}
          <p role="status" className="sr-only">
            {announcement}
          </p>
          {chat.phase === "error" && chat.error && (
            <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-xl bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
              <span>{chat.error.message}</span>
              {chat.error.retryable && (
                <Button type="button" size="sm" variant="outline" onClick={chat.retry}>
                  Retry
                </Button>
              )}
            </div>
          )}
          <div ref={endRef} />

          <form
            className="space-y-2"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <label htmlFor={composerId} className="text-xs font-medium text-muted-foreground">
              Your question
            </label>
            <textarea
              ref={composerRef}
              id={composerId}
              value={draft}
              rows={3}
              maxLength={AI_QUESTION_MAX_LENGTH}
              disabled={!live}
              aria-describedby={live ? counterId : demoNoteId}
              placeholder={live ? "Ask about tracked time, projects or the agent…" : undefined}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  submit();
                }
              }}
              className={cn(
                "block w-full resize-y rounded-xl border border-input bg-card px-3 py-2 text-sm",
                "placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring",
                "disabled:cursor-not-allowed disabled:opacity-60",
              )}
            />
            {live ? (
              <p id={counterId} className="text-[11px] text-muted-foreground">
                {draft.length} / {AI_QUESTION_MAX_LENGTH} · Enter to send, Shift+Enter for a new line
              </p>
            ) : (
              <p id={demoNoteId} className="text-[11px] text-muted-foreground">
                {DEMO_COMPOSER_NOTE}
              </p>
            )}
            <div className="flex flex-wrap items-center justify-end gap-2">
              <Button type="button" variant="ghost" size="sm" onClick={chat.newChat} disabled={chat.turns.length === 0 && chat.phase === "idle"}>
                <RotateCcw aria-hidden className="h-3.5 w-3.5" /> New chat
              </Button>
              {sending ? (
                <Button type="button" variant="outline" size="sm" onClick={chat.stop}>
                  <Square aria-hidden className="h-3.5 w-3.5" /> Stop
                </Button>
              ) : (
                <Button type="submit" size="sm" disabled={!live || !draft.trim()}>
                  <Send aria-hidden className="h-3.5 w-3.5" /> Send
                </Button>
              )}
            </div>
          </form>

          <p className="border-t border-border pt-3 text-xs text-muted-foreground">
            <Link href="/settings?tab=privacy" onClick={close} className="font-medium text-primary-strong hover:underline">
              What is shared?
            </Link>
          </p>
        </div>
      )}
    </Drawer>
  );
}
