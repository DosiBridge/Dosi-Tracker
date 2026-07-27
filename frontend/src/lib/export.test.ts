import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { downloadCSV, exportRecords } from "./export";

// Capture the CSV handed to the browser download without actually navigating. jsdom implements
// neither URL.createObjectURL nor a readable Blob, so we stub both: a FakeBlob remembers its parts
// and createObjectURL captures the instance.
let lastText: string | null = null;

class FakeBlob {
  readonly content: string;
  readonly type: string;
  constructor(parts: unknown[], opts?: { type?: string }) {
    this.content = parts.map(String).join("");
    this.type = opts?.type ?? "";
  }
}

beforeEach(() => {
  lastText = null;
  vi.stubGlobal("Blob", FakeBlob);
  (URL as unknown as { createObjectURL: unknown }).createObjectURL = vi.fn((blob: FakeBlob) => {
    lastText = blob.content;
    return "blob:mock";
  });
  (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = vi.fn();
  // jsdom anchors would try to navigate on click(); make it a no-op.
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function csvText(): string {
  expect(lastText).not.toBeNull();
  return lastText!;
}

describe("downloadCSV", () => {
  it("writes a header row then data rows", async () => {
    downloadCSV("out.csv", ["Name", "Score"], [["Ada", 90], ["Bob", 80]]);
    expect(await csvText()).toBe("Name,Score\nAda,90\nBob,80");
  });

  it("quotes and escapes cells containing commas, quotes, or newlines", async () => {
    downloadCSV("out", ["A"], [['he said "hi", loudly'], ["line1\nline2"]]);
    const text = await csvText();
    expect(text).toContain('"he said ""hi"", loudly"');
    expect(text).toContain('"line1\nline2"');
  });

  it("renders null/undefined cells as empty strings", async () => {
    downloadCSV("out", ["A", "B"], [[null, undefined]]);
    expect(await csvText()).toBe("A,B\n,");
  });

  it("appends a .csv extension only when missing", () => {
    const a = vi.spyOn(document.body, "appendChild");
    downloadCSV("nodotcsv", ["A"], []);
    const el1 = a.mock.calls[0][0] as HTMLAnchorElement;
    expect(el1.download).toBe("nodotcsv.csv");
    downloadCSV("already.csv", ["A"], []);
    const el2 = a.mock.calls[1][0] as HTMLAnchorElement;
    expect(el2.download).toBe("already.csv");
  });
});

describe("exportRecords", () => {
  it("projects records through the column value functions", async () => {
    const rows = [
      { first: "Ada", last: "Lovelace", n: 1 },
      { first: "Bob", last: "Ross", n: 2 },
    ];
    exportRecords("people", rows, [
      { header: "Full Name", value: (r) => `${r.first} ${r.last}` },
      { header: "N", value: (r) => r.n },
    ]);
    expect(await csvText()).toBe("Full Name,N\nAda Lovelace,1\nBob Ross,2");
  });
});
