/* ============================================================================
 * CSV export (no dependency) — files that open safely and correctly in Excel,
 * Google Sheets, Numbers and LibreOffice.
 *
 *  - Formula injection: a text cell starting with = + - @ tab or carriage
 *    return would run as a formula when the file is opened (a member could
 *    name themselves `=HYPERLINK(...)`), so it is prefixed with an apostrophe
 *    and reads as text. Numbers are written as they are — a negative number is
 *    data, not a formula — and so is text that is just a plain number.
 *  - Quoting (RFC 4180): a cell holding a comma, a double quote or a line break
 *    is wrapped in double quotes, with inner quotes doubled.
 *  - Rows end with CRLF, and the file starts with a UTF-8 byte order mark so
 *    Excel reads names such as "José" or Bengali text correctly.
 * ========================================================================== */

/** The byte order mark that tells Excel the file is UTF-8. */
export const CSV_BOM = String.fromCharCode(0xfeff);

const FORMULA_START = /^[=+\-@\t\r]/;
const PLAIN_NUMBER = /^-?\d+(\.\d+)?$/;
const NEEDS_QUOTES = /[",\r\n]/;

/** One cell, neutralised and quoted as needed. null and undefined are empty cells. */
export function csvCell(value: unknown): string {
  if (value === null || value === undefined) return "";
  let s = String(value);
  if (typeof value !== "number" && typeof value !== "bigint" && FORMULA_START.test(s) && !PLAIN_NUMBER.test(s)) {
    s = `'${s}`;
  }
  return NEEDS_QUOTES.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** The whole file: BOM, header row, then one CRLF-separated line per row. */
export function toCsv(headers: readonly unknown[], rows: readonly (readonly unknown[])[]): string {
  const lines = [headers, ...rows].map((row) => row.map(csvCell).join(","));
  return CSV_BOM + lines.join("\r\n");
}

/** A file name every OS accepts (no path separators, reserved or control characters), ending in ".csv". */
function csvFileName(name: string): string {
  let cleaned = "";
  for (const ch of name) {
    const reserved = (ch.codePointAt(0) ?? 0) < 0x20 || '<>:"/\\|?*'.includes(ch);
    cleaned += reserved ? "-" : ch;
  }
  const safe = cleaned.trim() || "export";
  return safe.toLowerCase().endsWith(".csv") ? safe : `${safe}.csv`;
}

export function downloadCSV(filename: string, headers: string[], rows: unknown[][]) {
  const blob = new Blob([toCsv(headers, rows)], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = csvFileName(filename);
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  // Released a moment later: some browsers still read the blob after click() returns.
  setTimeout(() => {
    try {
      URL.revokeObjectURL(url);
    } catch {
      // Nothing to release where object URLs aren't supported.
    }
  }, 1000);
}

/** Export an array of records using a column definition. */
export function exportRecords<T>(
  filename: string,
  records: T[],
  columns: { header: string; value: (row: T) => unknown }[]
) {
  downloadCSV(
    filename,
    columns.map((c) => c.header),
    records.map((rec) => columns.map((c) => c.value(rec)))
  );
}
