"use client";

import { useEffect, useRef, useState } from "react";
import { askAi } from "./ai-api";
import type { AiAnswerDto, AiAnswerSourceDto, AiFactRefDto, AiPageContextDto, AiResultMetaDto } from "@/lib/ai-contract";
import { AI_QUESTION_MAX_LENGTH, aiErrorState, type AiErrorState } from "@/lib/ai-assist";
import type { DemoAiContext } from "@/lib/ai-demo";

/* ============================================================================
 * One Ask AI conversation (AI assist spec §9.2, §18.2). Non-streaming:
 *
 *   idle → sending → done | error
 *
 * `send(message, context)` asks one question with what the page shows, in the
 * conversation the server started (its conversationId is threaded through
 * every follow-up). `stop()` abandons the question in flight, `newChat()`
 * starts over, `retry()` asks the last question again after an error. A
 * response that arrives after any of those — or after the component left —
 * is ignored.
 *
 * The server keeps a conversation for a limited time (and drops it when the
 * workspace's AI consent changes): Tracker:AiConversationExpired starts a new
 * conversation with the same question, once, and `onRestarted` tells the
 * person so.
 *
 * The transcript lives in React state only: never web storage. A demo
 * session never calls the API: it answers the suggested questions (by chip
 * id) from the demo data (lib/ai-demo, loaded on demand), and anything else
 * with a clarification listing them.
 * ========================================================================== */

export interface AiChatTurn {
  id: number;
  role: "user" | "assistant";
  /** The question as asked, or the answer's plain text. */
  text: string;
  sources: AiAnswerSourceDto[];
  facts: AiFactRefDto[];
  isClarification: boolean;
  /** The answer's result record (null for a question). */
  meta: AiResultMetaDto | null;
}

export type AiChatPhase = "idle" | "sending" | "done" | "error";

export interface UseAiChatOptions {
  /** The session is live: ask the server. Otherwise the demo's sample answers, with no network. */
  live: boolean;
  /** The workspace the conversation belongs to (its owner remounts the hook on a switch). */
  workspaceId: string;
  /** Demo only: the viewer and their zone. */
  demoContext?: DemoAiContext;
  /** The zone an error's dates are read in. */
  timeZone?: string;
  /** An expired conversation was replaced by a new one, with the question asked again. */
  onRestarted?: () => void;
}

export interface AiChat {
  turns: AiChatTurn[];
  phase: AiChatPhase;
  error: AiErrorState | null;
  conversationId: string | null;
  /** Ask a question (1-1000 characters); `chipId` names the suggested question it came from. Ignored while one is in flight. */
  send: (message: string, context?: AiPageContextDto | null, chipId?: string | null) => void;
  /** Ask the last question again, after an error. */
  retry: () => void;
  /** Abandon the question in flight. */
  stop: () => void;
  /** Forget the conversation and start over. */
  newChat: () => void;
}

interface Question {
  text: string;
  context: AiPageContextDto | null;
  chipId: string | null;
}

const NO_ANSWER = "Couldn't get an answer.";

export function useAiChat({ live, workspaceId, demoContext, timeZone, onRestarted }: UseAiChatOptions): AiChat {
  const [turns, setTurns] = useState<AiChatTurn[]>([]);
  const [phase, setPhase] = useState<AiChatPhase>("idle");
  const [error, setError] = useState<AiErrorState | null>(null);
  const [conversationId, setConversationId] = useState<string | null>(null);
  const controllerRef = useRef<AbortController | null>(null);
  /** Bumped by every question, stop, new chat and unmount: only the newest question may settle. */
  const runRef = useRef(0);
  const nextIdRef = useRef(0);
  const conversationRef = useRef<string | null>(null);
  const lastQuestionRef = useRef<Question | null>(null);

  const abandon = () => {
    runRef.current++;
    controllerRef.current?.abort();
    controllerRef.current = null;
  };

  const reset = () => {
    abandon();
    conversationRef.current = null;
    lastQuestionRef.current = null;
    setTurns([]);
    setPhase("idle");
    setError(null);
    setConversationId(null);
  };

  // Leaving abandons whatever is in flight.
  useEffect(
    () => () => {
      runRef.current++;
      controllerRef.current?.abort();
    },
    [],
  );

  const turnOf = (role: AiChatTurn["role"], text: string, answer?: AiAnswerDto): AiChatTurn => ({
    id: ++nextIdRef.current,
    role,
    text,
    sources: answer?.sources ?? [],
    facts: answer?.facts ?? [],
    isClarification: answer?.isClarification ?? false,
    meta: answer?.meta ?? null,
  });

  // Plain functions, rebuilt every render: each one reads this render's state.
  const ask = (question: Question, askedTurn: AiChatTurn) => {
    abandon();
    const ticket = ++runRef.current;
    lastQuestionRef.current = question;
    setPhase("sending");
    setError(null);

    const answered = (answer: AiAnswerDto) => {
      if (runRef.current !== ticket) return;
      conversationRef.current = answer.conversationId;
      setConversationId(answer.conversationId);
      setTurns((list) => [...list, turnOf("assistant", answer.answer, answer)]);
      setPhase("done");
    };
    const failed = (state: AiErrorState) => {
      if (runRef.current !== ticket) return;
      setError(state);
      setPhase("error");
    };

    if (!live) {
      import("@/lib/ai-demo")
        .then(({ demoAnswer }) => answered(demoAnswer(question.chipId ?? question.text, question.context?.page ?? "other", workspaceId, demoContext)))
        .catch(() => failed(aiErrorState(null, { fallback: NO_ANSWER })));
      return;
    }

    const attempt = (conversation: string | null, restarted: boolean) => {
      const controller = new AbortController();
      controllerRef.current = controller;
      askAi({ conversationId: conversation, message: question.text, context: question.context }, { signal: controller.signal }).then(
        (answer) => {
          if (!controller.signal.aborted) answered(answer);
        },
        (err: unknown) => {
          if (controller.signal.aborted || runRef.current !== ticket) return;
          const state = aiErrorState(err, { timeZone, fallback: NO_ANSWER });
          if (state.kind === "conversation-expired" && !restarted) {
            // The server forgot this conversation: start a new one with this question, once.
            conversationRef.current = null;
            setConversationId(null);
            setTurns([askedTurn]);
            onRestarted?.();
            attempt(null, true);
            return;
          }
          failed(state);
        },
      );
    };
    attempt(conversationRef.current, false);
  };

  const send = (message: string, context: AiPageContextDto | null = null, chipId: string | null = null) => {
    const text = message.trim();
    if (!text || text.length > AI_QUESTION_MAX_LENGTH || phase === "sending") return;
    const asked = turnOf("user", text);
    setTurns((list) => [...list, asked]);
    ask({ text, context, chipId }, asked);
  };

  const retry = () => {
    const question = lastQuestionRef.current;
    if (!question || phase !== "error") return;
    const asked = [...turns].reverse().find((t) => t.role === "user") ?? turnOf("user", question.text);
    ask(question, asked);
  };

  const stop = () => {
    if (phase !== "sending") return;
    abandon();
    setPhase("idle");
  };

  return { turns, phase, error, conversationId, send, retry, stop, newChat: reset };
}
