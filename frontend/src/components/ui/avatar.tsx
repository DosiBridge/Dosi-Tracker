import { cn, colorFromString, initials } from "@/lib/utils";
import type { UserStatus } from "@/lib/types";

const sizeMap = {
  sm: "h-8 w-8 text-xs",
  md: "h-10 w-10 text-sm",
  lg: "h-12 w-12 text-base",
};

const statusColor: Record<UserStatus, string> = {
  active: "bg-success",
  idle: "bg-warning",
  offline: "bg-muted-foreground",
};

export function Avatar({
  name,
  size = "md",
  status,
  className,
}: {
  name: string;
  size?: keyof typeof sizeMap;
  status?: UserStatus;
  className?: string;
}) {
  const bg = colorFromString(name);
  return (
    <div className={cn("relative shrink-0", className)}>
      <div
        className={cn(
          "flex items-center justify-center rounded-full font-semibold text-white ring-2 ring-card",
          sizeMap[size]
        )}
        style={{ background: `linear-gradient(135deg, ${bg}, ${colorFromString(name + "x", 65, 45)})` }}
        aria-hidden
      >
        {initials(name)}
      </div>
      {status && (
        <span
          className={cn(
            "absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full ring-2 ring-card",
            statusColor[status],
            status === "active" && "live-dot"
          )}
          title={status}
        />
      )}
    </div>
  );
}
