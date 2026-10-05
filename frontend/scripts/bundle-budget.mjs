// Bundle budget gate — run AFTER `next build`.
//
// Guards against the two regressions that actually happen: a single chunk
// ballooning (an accidental heavyweight dependency) and total JS creep.
// Budgets are a ratchet: set just above the measured baseline; tighten as the
// bundle shrinks, never loosen without a written justification in the PR.
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";

// Ratchet set against the measured baseline (35 chunks, 1552 KB total, 390 KB
// largest) with roughly 20% headroom for normal feature work. A breach means
// either a heavyweight dependency slipped in or a chunk stopped splitting —
// investigate before raising these numbers.
const CHUNK_DIR = join(process.cwd(), ".next", "static", "chunks");
const MAX_SINGLE_CHUNK_KB = Number(process.env.BUDGET_MAX_CHUNK_KB ?? 480);
//
// Raised 1900 → 2400 KB (2026-10, contract v2 round), with this justification:
// the round measured 1879 KB before it began and ~2285 KB after. The growth is
// feature code (presence, devices, payroll/hourly reports, session renewal and
// sign-in recovery, team/project/host management, timelines, popover/menu
// primitives); recharts was moved behind ONE dynamic import (components/charts)
// so its ~420 KB is shipped once instead of once per route group, and about
// 300 KB of the total are Turbopack's per-route copies of small shared modules
// (page header, data table…), which no config option controls. 2400 KB keeps
// ~5% headroom; the largest-chunk limit is unchanged.
const MAX_TOTAL_KB = Number(process.env.BUDGET_MAX_TOTAL_KB ?? 2400);

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(p));
    else if (entry.name.endsWith(".js")) out.push({ path: p, kb: statSync(p).size / 1024 });
  }
  return out;
}

let chunks;
try {
  chunks = walk(CHUNK_DIR);
} catch {
  console.error(`bundle-budget: ${CHUNK_DIR} not found — run \`next build\` first.`);
  process.exit(2);
}

chunks.sort((a, b) => b.kb - a.kb);
const totalKb = chunks.reduce((s, c) => s + c.kb, 0);

console.log(`bundle-budget: ${chunks.length} chunks, total ${Math.round(totalKb)} KB (budget ${MAX_TOTAL_KB} KB)`);
console.log("largest chunks:");
for (const c of chunks.slice(0, 5)) {
  console.log(`  ${Math.round(c.kb).toString().padStart(6)} KB  ${c.path.split("chunks")[1] ?? c.path}`);
}

const oversized = chunks.filter((c) => c.kb > MAX_SINGLE_CHUNK_KB);
let failed = false;
if (oversized.length) {
  console.error(`FAIL: ${oversized.length} chunk(s) exceed ${MAX_SINGLE_CHUNK_KB} KB`);
  failed = true;
}
if (totalKb > MAX_TOTAL_KB) {
  console.error(`FAIL: total chunk size ${Math.round(totalKb)} KB exceeds budget ${MAX_TOTAL_KB} KB`);
  failed = true;
}
process.exit(failed ? 1 : 0);
