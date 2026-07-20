"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

/** Insights demoted — surface key chips on Dashboard instead of a thin peer page. */
export default function InsightsRedirectPage() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/dashboard");
  }, [router]);
  return (
    <div className="flex items-center justify-center py-24 text-sm text-muted-foreground">
      Redirecting to Dashboard…
    </div>
  );
}
