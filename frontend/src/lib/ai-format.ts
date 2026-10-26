/* ============================================================================
 * AI assist's figures as people read them: how much of an allowance is used,
 * an estimated cost, a rate. Pure, and free of the rest of AI assist's
 * modules, so the host console's AI card can use it on its own.
 * ========================================================================== */

/* ----------------------------- Usage meters ----------------------------- */

export interface AiMeter {
  /** 0–100, for a progress bar; null when there is no limit to measure against. */
  percent: number | null;
  /** "42 of 2,000", or "42 (no monthly limit)". */
  text: string;
}

const count = (n: number) => Math.max(0, Math.round(n)).toLocaleString("en-US");

/** How much of an allowance is used: `used` of `limit` (null limit = none set). */
export function aiMeter(used: number, limit: number | null | undefined): AiMeter {
  const safeUsed = Number.isFinite(used) ? Math.max(0, used) : 0;
  if (typeof limit !== "number" || !Number.isFinite(limit) || limit <= 0) {
    return { percent: null, text: `${count(safeUsed)} (no monthly limit)` };
  }
  return { percent: Math.min(100, Math.round((safeUsed / limit) * 100)), text: `${count(safeUsed)} of ${count(limit)}` };
}

/** "$12.40"; "—" when the server couldn't price it (an unknown model). */
export function formatUsd(amount: number | null | undefined): string {
  if (typeof amount !== "number" || !Number.isFinite(amount)) return "—";
  return `$${amount.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** "13%", "2%", "0.4%" for a 0–1 rate; "—" when unreadable. */
export function formatRate(rate: number | null | undefined): string {
  if (typeof rate !== "number" || !Number.isFinite(rate)) return "—";
  const pct = Math.max(0, rate) * 100;
  // Whole percent from 10% up; one decimal below, without a trailing ".0" ("2%", "0.4%").
  return `${pct >= 10 ? Math.round(pct) : Number(pct.toFixed(1))}%`;
}
