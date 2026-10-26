/* ============================================================================
 * OWNER SELF-SERVICE BILLING (E2E-08), while the deployment bills:
 *  - an owner moves the workspace to a FREE plan whose seat limit the team
 *    fits (POST /api/app/billing/change-plan); a paid plan is assigned by the
 *    platform (Tracker:PlanChangeNotAllowed), so the page asks support for it;
 *  - an owner cancels a PAID subscription (POST /api/app/billing/cancel): no
 *    more invoices, and tracking pauses until a free plan or the platform
 *    activates it again. Issued invoices stay owed.
 * Pure rules over the server's plan table; the page does the requests.
 * ========================================================================== */

/** The fields of a plan these rules read (GET /api/app/workspace/available-plans). */
export interface PlanFacts {
  id: string;
  name?: string | null;
  pricePerUser?: number | null;
  /** 0 or less: no limit. */
  maxSeats?: number | null;
}

export function isFreePlan(plan: PlanFacts): boolean {
  return !(Number(plan.pricePerUser) > 0);
}

/** Whether a team of `seatsUsed` fits the plan's seat limit. */
export function teamFits(plan: PlanFacts, seatsUsed: number): boolean {
  const max = Number(plan.maxSeats);
  return !(max > 0) || seatsUsed <= max;
}

/** What an owner can do about one plan card. */
export type PlanAction =
  | { kind: "current" }
  /** A free plan the team fits: switch to it now. */
  | { kind: "switch" }
  /** A free plan the team doesn't fit: say why. */
  | { kind: "too-small"; maxSeats: number }
  /** A paid plan: only the platform assigns it. */
  | { kind: "request" };

export function planAction(plan: PlanFacts, input: { currentPlanId: string | null; seatsUsed: number }): PlanAction {
  if (input.currentPlanId && plan.id === input.currentPlanId) return { kind: "current" };
  if (!isFreePlan(plan)) return { kind: "request" };
  return teamFits(plan, input.seatsUsed) ? { kind: "switch" } : { kind: "too-small", maxSeats: Number(plan.maxSeats) };
}

/** The cheapest free plan the team fits (the trial / lapsed banners offer it); null when there is none. */
export function fittingFreePlan<T extends PlanFacts>(plans: readonly T[], seatsUsed: number, currentPlanId: string | null): T | null {
  return plans.find((p) => p.id !== currentPlanId && isFreePlan(p) && teamFits(p, seatsUsed)) ?? null;
}

/** Whether the owner may cancel: a paid plan that isn't canceled or expired already. */
export function canCancelSubscription(input: { pricePerUser: number | null; status: string }): boolean {
  return Number(input.pricePerUser) > 0 && input.status !== "canceled" && input.status !== "expired";
}

interface ErrorLike {
  code?: unknown;
  serverMessage?: unknown;
}

/** Why a plan change or cancellation was refused, in words. */
export function billingErrorText(err: unknown, fallback: string): string {
  const e = (err ?? {}) as ErrorLike;
  switch (e.code) {
    case "Tracker:SeatLimitReached":
      return "Your team has more members than that plan allows. Remove members from projects first, or pick a bigger plan.";
    case "Tracker:PlanChangeNotAllowed":
      return "Owners can switch to a free plan themselves. Ask support to move you to a paid plan.";
    case "Tracker:BillingDisabled":
      return "Billing is turned off on this server: there is no plan to change.";
    case "Tracker:PlanNotFound":
      return "That plan no longer exists. Reload the page to see the current plans.";
    default:
      return typeof e.serverMessage === "string" && e.serverMessage.trim() ? e.serverMessage.trim() : fallback;
  }
}
