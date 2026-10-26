"use client";

import { useEffect, useState } from "react";
import { Select } from "@/components/ui/input";
import { getAvailablePlansApi } from "@/hooks/useApi";
import { useBillingMode } from "@/hooks/use-billing-mode";
import type { PlanDto } from "@/lib/api-contract";

/** "Free — $0 · up to 3 seats", "Starter — $6/user/mo · 14-day trial": what the server's plan table says, nothing more. */
export function planOptionLabel(plan: {
  name?: string | null;
  pricePerUser?: number | null;
  maxSeats?: number | null;
  trialDays?: number | null;
}): string {
  const price = Number(plan.pricePerUser) > 0 ? `$${plan.pricePerUser}/user/mo` : "$0";
  const extra =
    Number(plan.trialDays) > 0 ? ` · ${plan.trialDays}-day trial` : Number(plan.maxSeats) > 0 ? ` · up to ${plan.maxSeats} seats` : "";
  return `${plan.name || "Plan"} — ${price}${extra}`;
}

/**
 * The plan a new workspace starts on, from the server's own plan table
 * (GET /api/app/workspace/available-plans, FE-12) — never prices or trials the
 * server doesn't offer. Nothing is shown while this deployment doesn't bill
 * (every workspace is active, the server ignores the plan) or when the plans
 * can't be read (the server then starts the workspace on its free plan).
 * Reports the chosen plan id, or null for none.
 */
export function SignupPlanPicker({ onChange }: { onChange: (planId: string | null) => void }) {
  const billing = useBillingMode();
  const [plans, setPlans] = useState<PlanDto[] | null>(null);
  const [planId, setPlanId] = useState("");

  useEffect(() => {
    if (!billing.ready || billing.enabled === false) {
      onChange(null);
      return;
    }
    let alive = true;
    getAvailablePlansApi().then(
      (list) => {
        if (!alive) return;
        setPlans(list);
        const first = list[0]?.id ?? "";
        setPlanId(first);
        onChange(first || null);
      },
      () => alive && setPlans([]),
    );
    return () => {
      alive = false;
    };
    // onChange is the parent's state setter: stable for the picker's lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [billing.ready, billing.enabled]);

  if (!plans || plans.length === 0 || billing.enabled === false) return null;
  return (
    <div className="space-y-1.5">
      <label htmlFor="signup-plan" className="text-sm font-medium">
        Choose a plan
      </label>
      <Select
        id="signup-plan"
        value={planId}
        onChange={(e) => {
          setPlanId(e.target.value);
          onChange(e.target.value || null);
        }}
      >
        {plans.map((p) => (
          <option key={p.id} value={p.id}>
            {planOptionLabel(p)}
          </option>
        ))}
      </Select>
    </div>
  );
}
