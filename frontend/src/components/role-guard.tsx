"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { Lock } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useSession } from "@/components/session-provider";
import { canAccess, roleLabels } from "@/lib/roles";

export function RoleGuard({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const { user } = useSession();

  if (!canAccess(user.role, pathname)) {
    return (
      <Card className="mx-auto flex max-w-md flex-col items-center justify-center gap-4 py-16 text-center">
        <div className="flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Lock className="h-7 w-7" />
        </div>
        <div>
          <h2 className="text-lg font-semibold">Restricted area</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Your role ({roleLabels[user.role]}) doesn&apos;t have access to this page.
          </p>
        </div>
        <Link href="/dashboard">
          <Button>Back to dashboard</Button>
        </Link>
      </Card>
    );
  }

  return <>{children}</>;
}
