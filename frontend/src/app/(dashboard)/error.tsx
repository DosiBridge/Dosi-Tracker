"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash in a workspace page: the sidebar and top bar stay up around this card. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="dashboard" home={{ href: "/dashboard", label: "Go to dashboard" }} />;
}
