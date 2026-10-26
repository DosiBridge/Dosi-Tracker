/* ============================================================================
 * The demo dataset's frozen clock. Tiny on purpose: live code needs these
 * constants (to tell "the demo's today" from the real one, and to lay out
 * calendars), while the demo data itself (lib/demo-tenants, lib/mock-data) is
 * only loaded, on demand, by a deployment that turns demo mode on.
 * ========================================================================== */

/** Fixed reference time so SSR and client render identically (no hydration drift). */
export const NOW = new Date("2026-07-14T15:30:00.000Z");

export const DAY_MS = 24 * 60 * 60 * 1000;

/** The demo's "today" as a UTC day key — the bucket day totals group by. */
export const TODAY = NOW.toISOString().slice(0, 10);
