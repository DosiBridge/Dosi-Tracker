import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import fc from "fast-check";
import { downloadCSV, exportRecords } from "./export";

/* ----------------------------------------------------------------------------
 * Capture the CSV text handed to the browser download. jsdom implements neither
 * URL.createObjectURL nor a readable Blob, so both are stubbed (mirrors the
 * technique in export.test.ts — that file owns the example-based cases; this
 * file owns the property-based roundtrip guarantees).
 * -------------------------------------------------------------------------- */

let lastText: string | null = null;

class CapturingBlob {
  readonly content: string;
  constructor(parts: unknown[]) {
    this.content = parts.map(String).join("");
  }
}

beforeEach(() => {
  lastText = null;
  vi.stubGlobal("Blob", CapturingBlob);
  (URL as unknown as { createObjectURL: unknown }).createObjectURL = vi.fn((blob: CapturingBlob) => {
    lastText = blob.content;
    return "blob:mock";
  });
  (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = vi.fn();
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function captured(): string {
  expect(lastText).not.toBeNull();
  return lastText!;
}

/**
 * A strict RFC-4180-style parser: cells split on commas, rows on newlines,
 * BOTH only outside double quotes; "" inside a quoted cell is a literal quote.
 * Written independently of export.ts so the two cannot share a bug.
 */
function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let inQuotes = false;
  let cellStarted = false;
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          cell += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i++;
        continue;
      }
      cell += ch;
      i++;
      continue;
    }
    if (ch === '"' && !cellStarted) {
      inQuotes = true;
      cellStarted = true;
      i++;
      continue;
    }
    if (ch === ",") {
      row.push(cell);
      cell = "";
      cellStarted = false;
      i++;
      continue;
    }
    if (ch === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
      cellStarted = false;
      i++;
      continue;
    }
    cell += ch;
    cellStarted = true;
    i++;
  }
  row.push(cell);
  rows.push(row);
  return rows;
}

/* ----------------------------- Arbitraries ----------------------------- */

// Hostile cell content: delimiters, quotes, newlines, Bangla, emoji, very long text.
const nastyCell = fc.oneof(
  { weight: 4, arbitrary: fc.string() },
  {
    weight: 4,
    arbitrary: fc.constantFrom(
      "রহিম করিম", // Bangla
      "সাপ্তাহিক রিপোর্ট, ৯৫%", // Bangla with a comma
      "👩‍💻 debugging 🔥",
      'he said "hi", loudly',
      "a,b,c",
      "line1\nline2",
      'mix: "q", comma, \n newline',
      "\rcarriage-return-only",
      "",
      ","
    ),
  },
  { weight: 1, arbitrary: fc.string().map((s) => `"${s}"`) }, // fully-quoted-looking cells
  { weight: 1, arbitrary: fc.constant("x".repeat(10000)) } // very long cell
);

// A rectangular table: 1..5 columns, 0..8 data rows, header always present.
const tableArb = fc.integer({ min: 1, max: 5 }).chain((cols) =>
  fc.record({
    headers: fc.array(nastyCell, { minLength: cols, maxLength: cols }),
    rows: fc.array(fc.array(nastyCell, { minLength: cols, maxLength: cols }), { maxLength: 8 }),
  })
);

/* ----------------------------- Properties ----------------------------- */

describe("CSV export roundtrip (property-based)", () => {
  it("any table of hostile cells survives a quote-respecting CSV parse unchanged", () => {
    fc.assert(
      fc.property(tableArb, ({ headers, rows }) => {
        downloadCSV("prop", headers, rows);
        expect(parseCsv(captured())).toEqual([headers, ...rows]);
      }),
      { numRuns: 60 }
    );
  });

  it("the header row always parses back as the first row, column-for-column", () => {
    fc.assert(
      fc.property(fc.array(nastyCell, { minLength: 1, maxLength: 5 }), (headers) => {
        downloadCSV("prop", headers, []);
        const parsed = parseCsv(captured());
        expect(parsed[0]).toEqual(headers);
      }),
      { numRuns: 60 }
    );
  });

  it("numeric cells export as exact decimal strings — never NaN, never scientific garbage", () => {
    const numericRow = fc.array(
      fc.oneof(
        fc.integer(),
        fc.double({ noNaN: true, noDefaultInfinity: true, min: -1e12, max: 1e12 })
      ),
      { minLength: 1, maxLength: 4 }
    );
    fc.assert(
      fc.property(fc.array(numericRow, { minLength: 1, maxLength: 6 }), (rows) => {
        const cols = Math.max(...rows.map((r) => r.length));
        const headers = Array.from({ length: cols }, (_, i) => `c${i}`);
        const padded = rows.map((r) => [...r, ...Array.from({ length: cols - r.length }, () => 0)]);
        downloadCSV("nums", headers, padded);
        const text = captured();
        expect(text).not.toContain("NaN");
        expect(text).not.toContain("undefined");
        const parsed = parseCsv(text);
        expect(parsed.slice(1)).toEqual(padded.map((r) => r.map((n) => String(n))));
      })
    );
  });

  it("null and undefined cells become empty strings, never the literal words", () => {
    fc.assert(
      fc.property(
        fc.array(fc.array(fc.constantFrom<null | undefined>(null, undefined), { minLength: 2, maxLength: 2 }), {
          minLength: 1,
          maxLength: 5,
        }),
        (rows) => {
          downloadCSV("empties", ["a", "b"], rows);
          const text = captured();
          expect(text).not.toContain("undefined");
          expect(text).not.toContain("null");
          expect(parseCsv(text).slice(1)).toEqual(rows.map(() => ["", ""]));
        }
      ),
      { numRuns: 40 }
    );
  });

  it("exportRecords projects records through columns and the result roundtrips", () => {
    const recordArb = fc.record({ name: nastyCell, minutes: fc.integer({ min: 0, max: 100000 }) });
    fc.assert(
      fc.property(fc.array(recordArb, { maxLength: 8 }), (records) => {
        exportRecords("records", records, [
          { header: "Member, name", value: (r) => r.name },
          { header: "Minutes", value: (r) => r.minutes },
        ]);
        const parsed = parseCsv(captured());
        expect(parsed[0]).toEqual(["Member, name", "Minutes"]);
        expect(parsed.slice(1)).toEqual(records.map((r) => [r.name, String(r.minutes)]));
      }),
      { numRuns: 60 }
    );
  });
});
