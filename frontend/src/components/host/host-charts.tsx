"use client";

import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { GrowthPoint } from "@/lib/host-data";
import { useIsMdUp } from "@/hooks/use-media-query";
import { cn } from "@/lib/utils";

const axisColor = "#94a3b8";
const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

function ChartFrame({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("w-full min-w-0", className)}>
      <ResponsiveContainer width="100%" height="100%">
        {children as React.ReactElement}
      </ResponsiveContainer>
    </div>
  );
}

export function MrrChart({ data }: { data: GrowthPoint[] }) {
  const md = useIsMdUp();
  return (
    <ChartFrame className="h-[200px] sm:h-[240px] lg:h-[260px]">
      <AreaChart data={data} margin={{ top: 10, right: 8, left: md ? -4 : -12, bottom: 0 }}>
        <defs>
          <linearGradient id="gMrr" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#0d9488" stopOpacity={0.4} />
            <stop offset="100%" stopColor="#0d9488" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis dataKey="month" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: md ? 12 : 10 }} />
        <YAxis
          tickLine={false}
          axisLine={false}
          width={md ? 44 : 32}
          tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
          tickFormatter={(v) => `$${Number(v) >= 1000 ? `${Math.round(Number(v) / 1000)}k` : v}`}
        />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => [`$${Number(v).toLocaleString()}`, "MRR"]} />
        <Area type="monotone" dataKey="mrr" stroke="#0d9488" strokeWidth={2.5} fill="url(#gMrr)" name="MRR" />
      </AreaChart>
    </ChartFrame>
  );
}

export function TenantsChart({ data }: { data: GrowthPoint[] }) {
  const md = useIsMdUp();
  return (
    <ChartFrame className="h-[180px] sm:h-[220px]">
      <BarChart data={data} margin={{ top: 4, right: 4, left: md ? -8 : -16, bottom: 0 }}>
        <XAxis dataKey="month" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: md ? 11 : 10 }} />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 11 }} allowDecimals={false} width={md ? 32 : 24} />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => [`${v}`, "Tenants"]} />
        <Bar dataKey="tenants" radius={[4, 4, 0, 0]} fill="#0284c7" barSize={md ? 16 : 12}>
          {data.map((d, i) => (
            <Cell key={i} fill={i === data.length - 1 ? "#0d9488" : "#0284c7"} />
          ))}
        </Bar>
      </BarChart>
    </ChartFrame>
  );
}
