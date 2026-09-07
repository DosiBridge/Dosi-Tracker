// @vitest-environment node
/**
 * ARCHITECTURE + SECURITY constraints — an executable spec over the raw source tree.
 *
 * Scope: everything dependency-cruiser CANNOT see. `.dependency-cruiser.cjs`
 * already enforces layering, circular imports, test-code leakage and orphans —
 * none of that is duplicated here. This file pins down *content* invariants:
 * where persistence lives, which API routes are real, where tenant ids may be
 * hardcoded, and a handful of security footguns.
 *
 * Every rule encodes the CURRENT, reviewed truth of the codebase as an explicit
 * allowlist constant. If a test fails because you added a file/usage:
 * that is the review trigger — either fix the code, or (after review) extend
 * the allowlist in the same PR with a justification comment.
 */
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { navAccess } from "@/lib/roles";
import contract from "@/test/api-contract.json";

/* ─────────────────────────── source-tree scanner ─────────────────────────── */

// This spec lives in src/test/ → one level up is src/.
const SRC_DIR = path.resolve(fileURLToPath(new URL(".", import.meta.url)), "..");

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

/** Repo-relative posix path, e.g. "src/hooks/useApi.ts". */
function rel(abs: string): string {
  return "src/" + path.relative(SRC_DIR, abs).split(path.sep).join("/");
}

const CODE_EXT = /\.(ts|tsx|js|jsx)$/;

/** Production source: all code under src/ EXCEPT src/test/** and *.test.* / *.spec.* / *.d.ts. */
const prodFiles = new Map<string, string>(); // rel path -> raw content
for (const abs of walk(SRC_DIR)) {
  const r = rel(abs);
  if (!CODE_EXT.test(r)) continue;
  if (r.endsWith(".d.ts")) continue;
  if (r.startsWith("src/test/")) continue;
  if (/\.(test|spec)\.(ts|tsx|js|jsx)$/.test(r)) continue;
  prodFiles.set(r, readFileSync(abs, "utf8"));
}

/* ────────────────────────── string-literal lexer ──────────────────────────
 * Rules 3/4/8 reason about *string literals*, so grepping raw lines would
 * false-positive on comments and code. This is a small single-pass lexer that
 * understands line/block comments, ' " ` strings (with escapes), template
 * `${…}` nesting (each placeholder becomes "\u0000" in the extracted value)
 * and regex literals (needed: src/lib/export.ts has regexes containing quote
 * characters). It is deliberately heuristic about `/` (regex vs division):
 * a regex may start only after a punctuator/keyword — which keeps JSX
 * closing tags (`</div>`) out of regex mode.
 */
interface Extracted {
  value: string;
  line: number;
}

type Frame =
  | { kind: "code"; fromTemplate: boolean; depth: number }
  | { kind: "str"; quote: "'" | '"'; buf: string; line: number }
  | { kind: "tpl"; buf: string; line: number };

const REGEX_PRECEDING_KEYWORDS = new Set([
  "return", "typeof", "case", "in", "of", "do", "else", "void", "new", "delete", "instanceof", "yield", "await",
]);

function extractStrings(src: string): Extracted[] {
  const out: Extracted[] = [];
  const stack: Frame[] = [{ kind: "code", fromTemplate: false, depth: 0 }];
  let line = 1;
  let i = 0;

  const regexCanStart = (pos: number): boolean => {
    let j = pos - 1;
    while (j >= 0 && /\s/.test(src[j])) j--;
    if (j < 0) return true;
    if (/[A-Za-z0-9_$]/.test(src[j])) {
      let k = j;
      while (k >= 0 && /[A-Za-z0-9_$]/.test(src[k])) k--;
      return REGEX_PRECEDING_KEYWORDS.has(src.slice(k + 1, j + 1));
    }
    return "(,=:[!&|?{};+-*%^~".includes(src[j]);
  };

  while (i < src.length) {
    const top = stack[stack.length - 1];
    const c = src[i];
    if (c === "\n") line++;

    if (top.kind === "code") {
      if (c === "/" && src[i + 1] === "/") {
        while (i < src.length && src[i] !== "\n") i++;
        continue;
      }
      if (c === "/" && src[i + 1] === "*") {
        i += 2;
        while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) {
          if (src[i] === "\n") line++;
          i++;
        }
        i += 2;
        continue;
      }
      if (c === "/" && regexCanStart(i)) {
        // Try to consume a single-line regex literal; fall through on failure.
        let j = i + 1;
        let inClass = false;
        while (j < src.length && src[j] !== "\n") {
          if (src[j] === "\\") {
            j += 2;
            continue;
          }
          if (src[j] === "[") inClass = true;
          else if (src[j] === "]") inClass = false;
          else if (src[j] === "/" && !inClass) break;
          j++;
        }
        if (j < src.length && src[j] === "/") {
          i = j + 1;
          while (i < src.length && /[a-z]/i.test(src[i])) i++; // flags
          continue;
        }
      }
      if (c === "'" || c === '"') {
        stack.push({ kind: "str", quote: c, buf: "", line });
        i++;
        continue;
      }
      if (c === "`") {
        stack.push({ kind: "tpl", buf: "", line });
        i++;
        continue;
      }
      if (c === "{") {
        top.depth++;
        i++;
        continue;
      }
      if (c === "}") {
        if (top.fromTemplate && top.depth === 0) {
          stack.pop();
          i++;
          continue;
        }
        if (top.depth > 0) top.depth--;
        i++;
        continue;
      }
      i++;
      continue;
    }

    if (top.kind === "str") {
      if (c === "\\") {
        top.buf += src[i + 1] ?? "";
        i += 2;
        continue;
      }
      if (c === top.quote) {
        out.push({ value: top.buf, line: top.line });
        stack.pop();
        i++;
        continue;
      }
      if (c === "\n") {
        stack.pop(); // unterminated (JSX apostrophe noise) — bail, never span lines
        i++;
        continue;
      }
      top.buf += c;
      i++;
      continue;
    }

    // template literal
    if (c === "\\") {
      top.buf += src[i + 1] ?? "";
      i += 2;
      continue;
    }
    if (c === "`") {
      out.push({ value: top.buf, line: top.line });
      stack.pop();
      i++;
      continue;
    }
    if (c === "$" && src[i + 1] === "{") {
      top.buf += "\u0000"; // placeholder marker for `${…}`
      stack.push({ kind: "code", fromTemplate: true, depth: 0 });
      i += 2;
      continue;
    }
    top.buf += c;
    i++;
  }
  return out;
}

const stringCache = new Map<string, Extracted[]>();
function stringsOf(file: string): Extracted[] {
  let cached = stringCache.get(file);
  if (!cached) {
    cached = extractStrings(prodFiles.get(file) ?? "");
    stringCache.set(file, cached);
  }
  return cached;
}

/** Files matching a raw-content regex (comments included — used where any textual occurrence counts). */
function filesMatching(re: RegExp): string[] {
  return [...prodFiles.keys()].filter((f) => re.test(prodFiles.get(f) as string)).sort();
}

function diff(actual: string[], allowed: ReadonlySet<string>): { added: string[]; stale: string[] } {
  return {
    added: actual.filter((f) => !allowed.has(f)),
    stale: [...allowed].filter((f) => !actual.includes(f)),
  };
}

/* ──────────────────────────────── the spec ──────────────────────────────── */

describe("architecture spec: scanner sanity", () => {
  it("sees the production source tree (a broken walk must not vacuously pass every rule)", () => {
    expect(prodFiles.size).toBeGreaterThan(40);
    expect(prodFiles.has("src/hooks/useApi.ts")).toBe(true);
    expect(prodFiles.has("src/lib/roles.ts")).toBe(true);
    expect([...prodFiles.keys()].some((f) => f.startsWith("src/test/"))).toBe(false);
  });
});

describe("architecture spec: persistence boundaries", () => {
  /**
   * RULE 1 — web storage is a persistence concern; every reader/writer below has
   * been reviewed. Additions require review — session state belongs in
   * SessionProvider / useApi, not sprinkled through pages.
   *
   * Reviewed groups (as of 2026-09):
   *  - Designed persistence layer: session-provider (dosi-user/-workspace/…),
   *    useApi (dosi-token/-tenant), tenant-data (dosi-projects-created-*),
   *    theme-provider + the static pre-hydration theme script in app/layout.
   *  - KNOWN TECH DEBT: 22 pages/dashboards/report widgets re-implement the
   *    same `localStorage.getItem("dosi-token")` live-mode probe inline. Do NOT
   *    add a 23rd — extract a shared `hasLiveSession()` helper instead, and
   *    shrink this list as callers migrate.
   */
  const STORAGE_ALLOWLIST = new Set<string>([
    // designed persistence layer
    "src/components/session-provider.tsx",
    "src/components/theme-provider.tsx",
    "src/hooks/useApi.ts",
    "src/lib/tenant-data.ts",
    "src/app/layout.tsx", // static theme script (pre-hydration, reads dosi-theme)
    // tech debt: inline dosi-token live-mode probes (shrink, never grow)
    "src/app/(dashboard)/activities/page.tsx",
    "src/app/(dashboard)/billing/page.tsx",
    "src/app/(dashboard)/monitor/page.tsx",
    "src/app/(dashboard)/projects/[id]/page.tsx",
    "src/app/(dashboard)/projects/page.tsx",
    "src/app/(dashboard)/settings/page.tsx",
    "src/app/(dashboard)/team/page.tsx",
    "src/app/(dashboard)/timesheet/page.tsx",
    "src/app/(host)/host/billing/page.tsx",
    "src/app/(host)/host/page.tsx",
    "src/app/(host)/host/plans/page.tsx",
    "src/app/(host)/host/tenants/page.tsx",
    "src/app/(host)/host/users/page.tsx",
    "src/components/dashboard/admin-dashboard.tsx",
    "src/components/dashboard/worker-dashboard.tsx",
    "src/components/reports/apps-report.tsx",
    "src/components/reports/attendance-report.tsx",
    "src/components/reports/payroll-report.tsx",
    "src/components/reports/productivity-report.tsx",
    "src/components/reports/projects-report.tsx",
    "src/components/reports/time-activity-report.tsx",
    "src/components/reports/weekly-report.tsx",
  ]);

  it("confines localStorage/sessionStorage access to the reviewed persistence allowlist", () => {
    const offenders = filesMatching(/(localStorage|sessionStorage)\s*[.[]/);
    const { added, stale } = diff(offenders, STORAGE_ALLOWLIST);
    expect(
      added,
      "New file(s) touch web storage. Session/persistence state belongs in the designed layer — do not add inline storage access. If reviewed and truly necessary, add to STORAGE_ALLOWLIST with a justification.",
    ).toEqual([]);
    expect(
      stale,
      "File(s) in STORAGE_ALLOWLIST no longer touch storage — remove them so the list stays true (it must only shrink).",
    ).toEqual([]);
  });
});

describe("architecture spec: XSS surface", () => {
  it("allows dangerouslySetInnerHTML only in the root layout's static theme script", () => {
    /**
     * RULE 2 — the single sanctioned use is app/layout.tsx: a build-time-constant
     * script that applies the stored theme before hydration (no user input can
     * reach it). Any other use is an XSS surface and fails review by default.
     */
    const offenders = filesMatching(/dangerouslySetInnerHTML/);
    expect(offenders).toEqual(["src/app/layout.tsx"]);
  });

  it('gives every target="_blank" link rel="noopener" (or noreferrer)', () => {
    // RULE 7 — without rel, the opened page gets window.opener and can navigate
    // us (reverse tabnabbing). Currently ZERO target="_blank" links exist; the
    // rule is here so the first one added ships safe.
    const violations: string[] = [];
    for (const [file, content] of prodFiles) {
      let idx = content.indexOf("_blank");
      while (idx !== -1) {
        const tagStart = content.lastIndexOf("<", idx);
        const tagEnd = content.indexOf(">", idx);
        const tag = tagStart !== -1 && tagEnd !== -1 ? content.slice(tagStart, tagEnd + 1) : "";
        if (!/rel=["'{][^"'}]*(noopener|noreferrer)/.test(tag)) {
          const line = content.slice(0, idx).split("\n").length;
          violations.push(`${file}:${line}`);
        }
        idx = content.indexOf("_blank", idx + 1);
      }
    }
    expect(violations, 'target="_blank" without rel="noopener"/"noreferrer" enables reverse tabnabbing.').toEqual([]);
  });
});

describe("architecture spec: API contract", () => {
  it("only calls backend routes declared in src/test/api-contract.json", () => {
    /**
     * RULE 3 — a "/api/…" or "/connect/…" literal that the backend does not
     * serve is dead-on-arrival (guaranteed 404). Every endpoint reference in
     * production src must prefix-match a contract entry; update the contract in
     * the same PR as the backend route.
     *
     * A literal counts as an endpoint reference when the path BEGINS the string
     * (optionally after a `${API_BASE_URL}`-style placeholder). Prose that
     * merely mentions a path — e.g. the fake browser window title
     * "POST /api/activities" in src/lib/monitor-data.ts demo data — is not a
     * call site and is deliberately ignored.
     */
    const endpoints: string[] = contract.endpoints;
    expect(endpoints.length).toBeGreaterThan(0);

    const violations: string[] = [];
    let refsSeen = 0;
    for (const file of prodFiles.keys()) {
      for (const s of stringsOf(file)) {
        let start = -1;
        if (s.value.startsWith("/api/") || s.value.startsWith("/connect/")) {
          start = 0;
        } else {
          for (const marker of ["\u0000/api/", "\u0000/connect/"]) {
            const at = s.value.indexOf(marker);
            if (at !== -1 && (start === -1 || at + 1 < start)) start = at + 1;
          }
        }
        if (start === -1) continue;
        refsSeen++;
        const called = s.value
          .slice(start)
          .split("?")[0]
          .split("\u0000")[0]
          .replace(/\/+$/, "");
        const served = endpoints.some((e) => called === e || called.startsWith(e + "/"));
        if (!served) violations.push(`${file}:${s.line} calls "${called}"`);
      }
    }
    expect(
      violations,
      "Frontend calls route(s) the backend contract does not serve — fix the path or update src/test/api-contract.json together with the backend.",
    ).toEqual([]);
    // Anti-vacuity: the app is known to make dozens of API calls. If the lexer
    // ever broke and extracted nothing, this must fail rather than pass silently.
    expect(refsSeen).toBeGreaterThanOrEqual(30);
  });
});

describe("architecture spec: tenant isolation", () => {
  it("hardcodes tenant ids (w1/w2/w3) only in the data layer", () => {
    /**
     * RULE 4 — demo tenant ids are DEFINED in src/lib (tenant-data.ts,
     * saas-data.ts); src/test seeds them via the harness. Production
     * components/pages/hooks must stay tenant-agnostic — even SessionProvider
     * derives its default workspace from the data layer rather than a literal.
     * Current count of exceptions outside src/lib: ZERO. Keep it that way;
     * any addition requires review.
     */
    const TENANT_ID_ALLOWLIST = new Set<string>([]);
    const violations: string[] = [];
    for (const file of prodFiles.keys()) {
      if (file.startsWith("src/lib/")) continue; // the data layer defines these ids
      if (TENANT_ID_ALLOWLIST.has(file)) continue;
      for (const s of stringsOf(file)) {
        if (s.value === "w1" || s.value === "w2" || s.value === "w3") {
          violations.push(`${file}:${s.line} "${s.value}"`);
        }
      }
    }
    expect(
      violations,
      "Tenant id literal outside src/lib — thread the workspace id through props/context/session instead.",
    ).toEqual([]);
  });
});

describe("architecture spec: hygiene", () => {
  it("keeps console.log out of production code (shrinking allowlist, target: zero)", () => {
    /**
     * RULE 5 — console.warn/error are fine; console.log is debug residue.
     * TODO(to-zero): src/components/session-provider.tsx:~137 logs a hydration
     * success message — drop it (or demote behind a debug flag) and empty this
     * list. It must never grow.
     */
    const CONSOLE_LOG_ALLOWLIST = new Set<string>(["src/components/session-provider.tsx"]);
    const offenders = filesMatching(/console\.log\s*\(/);
    const { added, stale } = diff(offenders, CONSOLE_LOG_ALLOWLIST);
    expect(added, "New console.log in production code — remove it (console.warn/error are allowed).").toEqual([]);
    expect(stale, "console.log allowlist entry no longer needed — delete it so the list keeps shrinking.").toEqual([]);
  });

  it("carries no TODO/FIXME/HACK markers in production source", () => {
    // RULE 6 — currently zero. Track work in issues, not in shipped code.
    const violations: string[] = [];
    for (const [file, content] of prodFiles) {
      const lines = content.split("\n");
      for (let n = 0; n < lines.length; n++) {
        if (/\b(TODO|FIXME|HACK)\b/.test(lines[n])) violations.push(`${file}:${n + 1}`);
      }
    }
    expect(violations, "Marker comments rot in place — file an issue instead.").toEqual([]);
  });
});

describe("architecture spec: secret scanning", () => {
  it("contains no secret-looking string literals beyond documented safe uses", () => {
    /**
     * RULE 8 — scan every string literal for credential-shaped content.
     * Each allowlisted VALUE below was inspected and is safe because it is a
     * word ABOUT authentication, never a credential itself:
     *  - "password"        → HTML input type="password", the OAuth2
     *                        grant_type/form-field name in useApi.ts's
     *                        resource-owner flow, and login form state. A
     *                        fixed protocol/DOM token, not a secret value.
     *  - "Invalid username, password or workspace"
     *                      → user-facing login error copy (login/page.tsx).
     *  - "1Password"       → a password-manager PRODUCT name inside the demo
     *                        app-usage data (mock-data.ts / tenant-data.ts).
     * Note the `Bearer ${token}` header templates do NOT trip the scan: the
     * pattern only flags "bearer" followed by literal token characters, i.e.
     * an inlined credential.
     */
    const SECRET_PATTERN = /(password|secret|api[_-]?key|bearer\s+[a-z0-9])/i;
    // Each entry was read in context and is benign: UI copy, an <input type>,
    // a demo app name, or a DOM id/aria-label for a password FIELD (a label
    // for the box, never a value that goes in it).
    const SAFE_LITERALS = new Set<string>([
      "password", // <input type="password">
      "Invalid username, password or workspace", // sign-in error copy
      "1Password", // demo row: a tracked application's name
      "login-password", // id/htmlFor pair, sign-in form
      "signup-password", // id/htmlFor pair, workspace-creation form
      "Show password", // aria-label on the reveal toggle
      "Hide password", // aria-label on the reveal toggle
      "Forgot password?", // link copy
    ]);
    const violations: string[] = [];
    let hitsSeen = 0;
    for (const file of prodFiles.keys()) {
      for (const s of stringsOf(file)) {
        if (!SECRET_PATTERN.test(s.value)) continue;
        hitsSeen++;
        if (SAFE_LITERALS.has(s.value)) continue;
        violations.push(`${file}:${s.line} "${s.value}"`);
      }
    }
    expect(
      violations,
      "Secret-looking literal found. If it is a real credential: rotate it NOW and move it to env config. If it is benign copy, add the exact literal to SAFE_LITERALS with a reason.",
    ).toEqual([]);
    // Anti-vacuity: the reviewed safe literals above EXIST in the codebase
    // (grant_type 'password', input types, '1Password' demo rows). Zero hits
    // would mean the lexer broke, not that the code got cleaner.
    expect(hitsSeen).toBeGreaterThanOrEqual(4);
  });
});

describe("architecture spec: route-permission consistency", () => {
  // Map every src/app/**/page.tsx to its URL (route groups "(dashboard)"/"(host)"
  // vanish from URLs; dynamic segments stay as-is and are matched by prefix).
  const pageUrls = [...prodFiles.keys()]
    .filter((f) => f.startsWith("src/app/") && f.endsWith("/page.tsx"))
    .map((f) => {
      const segs = f.slice("src/app/".length).split("/");
      segs.pop(); // page.tsx
      return "/" + segs.filter((seg) => !(seg.startsWith("(") && seg.endsWith(")"))).join("/");
    });

  /**
   * Intentionally outside the navAccess matrix:
   *  - "/"       → redirect stub straight to /dashboard (which IS guarded).
   *  - "/login"  → the public sign-in page; it must be reachable logged-out.
   */
  const PUBLIC_ROUTES = new Set<string>(["/", "/login"]);

  it("has a real page behind every navAccess permission (no phantom permissions)", () => {
    // RULE 9a — a navAccess key with no page is dead configuration that
    // silently rots (e.g. a renamed module leaving its old guard behind).
    const phantom = Object.keys(navAccess).filter(
      (key) => !pageUrls.some((u) => u === key || u.startsWith(key + "/")),
    );
    expect(phantom, "navAccess key(s) guard route(s) that do not exist under src/app — delete or rename them.").toEqual([]);
  });

  it("covers every app route with a navAccess permission or a documented public exception (no unguarded modules)", () => {
    // RULE 9b — every page must resolve to a role guard. A new top-level module
    // without a navAccess entry would render for EVERY role (canAccess returns
    // true for unknown prefixes) — that is an access-control hole, not a default.
    const keys = Object.keys(navAccess);
    const unguarded = pageUrls.filter(
      (u) => !PUBLIC_ROUTES.has(u) && !keys.some((k) => u === k || u.startsWith(k + "/")),
    );
    expect(
      unguarded,
      "Route(s) exist with no navAccess entry — add the module to navAccess in src/lib/roles.ts (or, only for genuinely public pages, to PUBLIC_ROUTES here with a justification).",
    ).toEqual([]);
  });

  it("keeps the demo page inventory itself honest", () => {
    // Guards the two tests above against a silently-broken glob: the app tree
    // is known to contain the login page, the host console and the dashboard.
    expect(pageUrls).toContain("/login");
    expect(pageUrls).toContain("/host");
    expect(pageUrls).toContain("/dashboard");
    expect(pageUrls.length).toBeGreaterThanOrEqual(15);
  });
});
