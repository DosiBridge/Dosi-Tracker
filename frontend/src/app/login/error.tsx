"use client";

import { ErrorFallback } from "@/components/error-fallback";

/** A crash on the sign-in page. */
export default function SegmentError({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorFallback error={error} retry={retry} boundary="login" home={{ href: "/login", label: "Reload sign-in" }} />;
}
