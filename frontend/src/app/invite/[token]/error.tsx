"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash while opening an invitation. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="invite" home={{ href: "/login", label: "Go to sign-in" }} />;
}
