"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash in a platform console page: the console's navigation stays up around this card. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="host" home={{ href: "/host", label: "Go to the console" }} />;
}
