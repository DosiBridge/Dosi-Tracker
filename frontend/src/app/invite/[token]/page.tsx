"use client";

import { useParams } from "next/navigation";
import { InviteView } from "@/components/invite-view";

/** /invite/{token}: join a workspace from an invitation link. */
export default function InvitePage() {
  const params = useParams<{ token?: string | string[] }>();
  const raw = params?.token;
  const token = Array.isArray(raw) ? raw[0] : raw;
  return <InviteView token={token ?? ""} />;
}
