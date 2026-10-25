"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash in an installation guide: the guides' header and navigation stay up. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="install" home={{ href: "/install", label: "All installation guides" }} />;
}
