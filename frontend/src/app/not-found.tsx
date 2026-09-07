import Link from "next/link";
import { Compass } from "lucide-react";
import { Button } from "@/components/ui/button";

/**
 * Branded 404. Unknown URLs previously fell through to Next's bare default
 * page, which offers no way back into the app.
 */
export default function NotFound() {
  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <div className="w-full max-w-md rounded-2xl border border-border bg-card p-8 text-center card-elev">
        <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Compass className="h-7 w-7" />
        </div>
        <h1 className="mt-5 font-display text-2xl font-bold tracking-tight">This page doesn&apos;t exist</h1>
        <p className="mt-2 text-sm text-muted-foreground">
          The link may be out of date, or the page may have been moved. Your workspace and data are unaffected.
        </p>
        <div className="mt-6 flex justify-center gap-2">
          <Link href="/dashboard">
            <Button>Go to dashboard</Button>
          </Link>
          <Link href="/projects">
            <Button variant="outline">Browse projects</Button>
          </Link>
        </div>
      </div>
    </div>
  );
}
