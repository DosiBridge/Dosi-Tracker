/** Brand accents for charts / inline styles (mirrors CSS --primary). */
export const brand = {
  primary: "#0d9488",
  primaryMuted: "#14b8a6",
  ink: "#0c1524",
  info: "#0284c7",
  success: "#16a34a",
  warning: "#d97706",
  danger: "#dc2626",
  pink: "#db2777",
} as const;

export function greeting(name: string, hour = new Date().getHours()): string {
  const first = name.split(" ")[0] ?? name;
  if (hour < 12) return `Good morning, ${first}`;
  if (hour < 17) return `Good afternoon, ${first}`;
  return `Good evening, ${first}`;
}
