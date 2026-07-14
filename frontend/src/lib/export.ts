/** Minimal CSV export helpers (no dependency). */

function escapeCell(value: unknown): string {
  const s = value === null || value === undefined ? "" : String(value);
  if (/[",\n]/.test(s)) return `"${s.replace(/"/g, '""')}"`;
  return s;
}

export function downloadCSV(filename: string, headers: string[], rows: unknown[][]) {
  const lines = [headers.map(escapeCell).join(","), ...rows.map((r) => r.map(escapeCell).join(","))];
  const csv = lines.join("\n");
  const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename.endsWith(".csv") ? filename : `${filename}.csv`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
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
