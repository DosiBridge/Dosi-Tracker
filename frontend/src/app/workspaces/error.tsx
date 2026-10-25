"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash in the workspace chooser. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="workspaces" home={{ href: "/workspaces", label: "Back to your workspaces" }} />;
}
