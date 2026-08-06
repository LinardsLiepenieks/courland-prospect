/**
 * Reading SQLite timestamps.
 *
 * Every timestamp the backend writes comes from `datetime('now')`: UTC, formatted
 * "YYYY-MM-DD HH:MM:SS" — a space instead of the ISO "T", and no zone suffix. Left as-is,
 * that string is read as LOCAL time by most engines, so the one conversion that matters
 * (add the "T", add the "Z") lives here once instead of being re-derived per caller.
 */

/** Milliseconds since the epoch for a SQLite UTC timestamp, or `null` if it won't parse.
 *
 *  The explicit `Z` matters: without it these strings parse as LOCAL time, which shifts
 *  every derived age by the viewer's UTC offset — enough to flip a staleness colour a whole
 *  day early or late west of Greenwich. `Date.parse` on the space form is also
 *  implementation-defined, so the ISO `T` isn't optional either.
 *
 *  Callers decide what an unreadable date means — `formatDate` echoes the raw string back,
 *  staleness grading treats it as "fresh" so a bad row can't paint a card red — so this
 *  reports the failure rather than choosing on their behalf. */
export function parseSqliteUtc(sqliteUtc: string): number | null {
  const ms = Date.parse(sqliteUtc.replace(" ", "T") + "Z");
  return Number.isNaN(ms) ? null : ms;
}

/**
 * Format a SQLite `datetime('now')` timestamp (UTC, "YYYY-MM-DD HH:MM:SS")
 * into a short local date like "Jul 10, 2026".
 */
export function formatDate(sqliteUtc: string): string {
  const ms = parseSqliteUtc(sqliteUtc);
  if (ms == null) return sqliteUtc;
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}
