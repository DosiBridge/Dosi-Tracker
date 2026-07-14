"use client";

import { useEffect, useRef, useState } from "react";
import { Calendar, Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { rangeForKey, rangePresets, type RangeKey } from "@/lib/reports-data";

export interface ResolvedRange {
  key: RangeKey;
  from: Date;
  to: Date;
  label: string;
}

export function DateRangePicker({
  value,
  onChange,
}: {
  value: RangeKey;
  onChange: (r: ResolvedRange) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const current = rangePresets.find((r) => r.key === value) ?? rangePresets[0];

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  function choose(key: RangeKey) {
    const r = rangeForKey(key);
    onChange({ key, ...r });
    if (key !== "custom") setOpen(false);
  }

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-10 items-center gap-2 rounded-lg border border-border bg-background px-3 text-sm hover:bg-muted"
      >
        <Calendar className="h-4 w-4 text-muted-foreground" />
        {current.label}
        <ChevronDown className="h-4 w-4 text-muted-foreground" />
      </button>
      {open && (
        <div className="absolute right-0 z-30 mt-2 w-56 overflow-hidden rounded-xl border border-border bg-card p-1 card-elev-lg animate-scale-in">
          {rangePresets.map((r) => (
            <button
              key={r.key}
              onClick={() => choose(r.key)}
              className={cn(
                "flex w-full items-center justify-between rounded-lg px-3 py-2 text-sm hover:bg-muted",
                value === r.key && "text-primary"
              )}
            >
              {r.label}
              {value === r.key && <Check className="h-4 w-4" />}
            </button>
          ))}
          {value === "custom" && (
            <div className="space-y-2 border-t border-border p-2">
              <input type="date" className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm" />
              <input type="date" className="h-9 w-full rounded-lg border border-input bg-background px-2 text-sm" />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
