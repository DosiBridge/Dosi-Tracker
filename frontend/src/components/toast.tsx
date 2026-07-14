"use client";

import { useEffect, useState } from "react";
import { CheckCircle2, Info, AlertTriangle, XCircle, X } from "lucide-react";
import { cn } from "@/lib/utils";

type ToastTone = "success" | "info" | "warning" | "danger";

interface ToastItem {
  id: number;
  title: string;
  description?: string;
  tone: ToastTone;
}

interface ToastDetail {
  title: string;
  description?: string;
  tone?: ToastTone;
}

const toneMeta: Record<ToastTone, { icon: typeof Info; color: string }> = {
  success: { icon: CheckCircle2, color: "#22c55e" },
  info: { icon: Info, color: "#0ea5e9" },
  warning: { icon: AlertTriangle, color: "#f59e0b" },
  danger: { icon: XCircle, color: "#ef4444" },
};

/** Fire a toast from anywhere (client). */
export function toast(detail: ToastDetail) {
  window.dispatchEvent(new CustomEvent<ToastDetail>("dosi-toast", { detail }));
}

let counter = 0;

export function ToastViewport() {
  const [items, setItems] = useState<ToastItem[]>([]);

  useEffect(() => {
    const onToast = (e: Event) => {
      const d = (e as CustomEvent<ToastDetail>).detail;
      const id = ++counter;
      setItems((prev) => [...prev, { id, title: d.title, description: d.description, tone: d.tone ?? "success" }]);
      setTimeout(() => setItems((prev) => prev.filter((t) => t.id !== id)), 4200);
    };
    window.addEventListener("dosi-toast", onToast);
    return () => window.removeEventListener("dosi-toast", onToast);
  }, []);

  function dismiss(id: number) {
    setItems((prev) => prev.filter((t) => t.id !== id));
  }

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[110] flex w-full max-w-sm flex-col gap-2">
      {items.map((t) => {
        const Icon = toneMeta[t.tone].icon;
        return (
          <div
            key={t.id}
            className="pointer-events-auto flex items-start gap-3 rounded-xl border border-border bg-card p-3.5 card-elev-lg animate-scale-in"
          >
            <Icon className="mt-0.5 h-5 w-5 shrink-0" style={{ color: toneMeta[t.tone].color }} />
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium">{t.title}</div>
              {t.description && <div className="mt-0.5 text-xs text-muted-foreground">{t.description}</div>}
            </div>
            <button onClick={() => dismiss(t.id)} className={cn("text-muted-foreground hover:text-foreground")}>
              <X className="h-4 w-4" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
