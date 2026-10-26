"use client";

import { useState } from "react";
import dynamic from "next/dynamic";
import { useAiAssistant } from "@/components/ai/ai-provider";

// The drawer, its transcript and the chat hook download the first time Ask AI opens (§18.1).
const AiAssistantDrawer = dynamic(() => import("@/components/ai/ai-assistant-drawer").then((m) => m.AiAssistantDrawer), { ssr: false });

/**
 * Where the Ask AI drawer lives, mounted once by the dashboard layout.
 * Nothing loads until the drawer is first opened; after that it stays
 * mounted (closed), so the conversation is still there when it opens again.
 * A workspace switch mounts a fresh one: a conversation never crosses
 * workspaces.
 */
export function AiAssistantMount() {
  const { drawerOpen, workspaceId } = useAiAssistant();
  const [wanted, setWanted] = useState(drawerOpen);
  if (drawerOpen && !wanted) setWanted(true);
  return wanted ? <AiAssistantDrawer key={workspaceId} /> : null;
}
