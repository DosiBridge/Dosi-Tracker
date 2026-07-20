"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

/** Screenshots live under Activity (Screens view). Keep URL for old links. */
export default function ScreenshotsRedirectPage() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/activities?view=screens");
  }, [router]);
  return (
    <div className="flex items-center justify-center py-24 text-sm text-muted-foreground">
      Opening Activity · Screens…
    </div>
  );
}
