import { ArrowDownRight, ArrowUpRight } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { brand } from "@/lib/brand";
import { cn } from "@/lib/utils";

export function StatCard({
  label,
  value,
  icon: Icon,
  trend,
  accent = brand.primary,
  sub,
}: {
  label: string;
  value: string;
  icon: LucideIcon;
  trend?: number;
  accent?: string;
  sub?: string;
}) {
  const up = (trend ?? 0) >= 0;
  return (
    <Card className="motion-safe-lift p-5 transition-transform duration-300 hover:-translate-y-0.5">
      <div className="flex items-start justify-between">
        <div
          className="flex h-10 w-10 items-center justify-center rounded-xl transition-transform duration-300 group-hover:scale-105"
          style={{ background: `${accent}18`, color: accent }}
        >
          <Icon className="h-5 w-5" />
        </div>
        {trend !== undefined && (
          <span
            className={cn(
              "inline-flex items-center gap-0.5 rounded-full px-2 py-0.5 text-xs font-medium",
              up ? "bg-success/12 text-success" : "bg-danger/12 text-danger"
            )}
          >
            {up ? <ArrowUpRight className="h-3 w-3" /> : <ArrowDownRight className="h-3 w-3" />}
            {Math.abs(trend)}%
          </span>
        )}
      </div>
      <div className="mt-4">
        <div className="font-display text-2xl font-bold tracking-tight tabular-nums">{value}</div>
        <div className="mt-1 text-sm text-muted-foreground">{label}</div>
        {sub && <div className="mt-0.5 text-xs text-muted-foreground/70">{sub}</div>}
      </div>
    </Card>
  );
}
