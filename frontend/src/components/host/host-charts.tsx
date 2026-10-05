"use client";

import { ChartLoading } from "@/components/charts/chart-loading";
import type { Recharts } from "@/components/charts/recharts-kit";
import { useRecharts } from "@/components/charts/use-recharts";
import type { GrowthPoint } from "@/lib/host-data";
import { useIsMdUp, useMediaQuery } from "@/hooks/use-media-query";
import { cn } from "@/lib/utils";

/** Axis labels: the muted text colour (≥ 4.5:1 on cards in both themes). */
const axisColor = "var(--muted-foreground)";
const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

/** `children` draws the chart once recharts has loaded (use-recharts.ts); a same-size placeholder until then. */
function ChartFrame({ children, className }: { children: (recharts: Recharts) => React.ReactElement; className?: string }) {
  const recharts = useRecharts();
  return (
    <div className={cn("w-full min-w-0", className)}>
      {recharts ? (
        <recharts.ResponsiveContainer width="100%" height="100%">
          {children(recharts)}
        </recharts.ResponsiveContainer>
      ) : (
        <ChartLoading />
      )}
    </div>
  );
}

/** Charts animate only once the browser has said motion is welcome. */
function useChartMotion(): boolean {
  return useMediaQuery("(prefers-reduced-motion: no-preference)");
}

/** "$950", "$12k": a tick on the money axis. */
function moneyTick(value: unknown): string {
  const n = Number(value);
  return `$${n >= 1000 ? `${Math.round(n / 1000)}k` : n}`;
}

export function MrrChart({ data }: { data: GrowthPoint[] }) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  return (
    <ChartFrame className="h-[200px] sm:h-[240px] lg:h-[260px]">
      {({ AreaChart, Area, XAxis, YAxis, Tooltip }) => (
        <AreaChart data={data} margin={{ top: 10, right: 8, left: 0, bottom: 0 }}>
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
            width={md ? 48 : 40}
            tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
            tickFormatter={moneyTick}
          />
          <Tooltip contentStyle={tooltipStyle} formatter={(v) => [`$${Number(v).toLocaleString("en-US")}`, "MRR"]} />
          <Area type="monotone" dataKey="mrr" stroke="#0d9488" strokeWidth={2.5} fill="url(#gMrr)" name="MRR" isAnimationActive={motion} />
        </AreaChart>
      )}
    </ChartFrame>
  );
}

export function TenantsChart({ data }: { data: GrowthPoint[] }) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  return (
    <ChartFrame className="h-[180px] sm:h-[220px]">
      {({ BarChart, Bar, Cell, XAxis, YAxis, Tooltip }) => (
        <BarChart data={data} margin={{ top: 4, right: 4, left: 0, bottom: 0 }}>
          <XAxis dataKey="month" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: md ? 11 : 10 }} />
          <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 11 }} allowDecimals={false} width={md ? 32 : 28} />
          <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => [`${v}`, "Tenants"]} />
          <Bar dataKey="tenants" radius={[4, 4, 0, 0]} fill="#0284c7" barSize={md ? 16 : 12} isAnimationActive={motion}>
            {data.map((d, i) => (
              <Cell key={i} fill={i === data.length - 1 ? "#0d9488" : "#0284c7"} />
            ))}
          </Bar>
        </BarChart>
      )}
    </ChartFrame>
  );
}
