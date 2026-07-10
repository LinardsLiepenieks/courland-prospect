/**
 * Format a SQLite `datetime('now')` timestamp (UTC, "YYYY-MM-DD HH:MM:SS")
 * into a short local date like "Jul 10, 2026".
 */
export function formatDate(sqliteUtc: string): string {
  const iso = sqliteUtc.replace(" ", "T") + "Z";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return sqliteUtc;
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
