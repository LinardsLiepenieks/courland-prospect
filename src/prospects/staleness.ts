import type { Prospect } from "../api/prospects";
import type { Stage } from "../api/stages";
import { parseSqliteUtc } from "../lib/date";

/**
 * How long a prospect has been waiting on you, graded against the thresholds of
 * the stage they sit in.
 *
 * The clock is YOUR outreach, not activity in general: a thread where they
 * replied a week ago and you never answered is exactly the case this is meant to
 * surface, so their message must not reset it. `last_outreach_at` is derived
 * backend-side from captured outgoing messages only.
 */
export type Staleness = "fresh" | "warn" | "stale";

/** A prospect's staleness plus the age it was derived from, so the caller can
 *  render both the color and the "9d" label from one computation. */
export interface StalenessReading {
  level: Staleness;
  /** Whole days since the clock started. Never negative. */
  days: number;
  /** False when we've never messaged them, so the age is counted from when they
   *  were captured rather than from an actual outreach. Changes the wording of
   *  the label ("captured 9d ago" vs "9d since you wrote"). */
  contacted: boolean;
}

const MS_PER_DAY = 86_400_000;

/**
 * Grade one prospect against their stage's thresholds.
 *
 * Falls back to `created_at` when you've never messaged them: a prospect
 * captured three weeks ago and never contacted is the *most* stale thing on the
 * board, and reading `null` as "fresh" would hide exactly the people who need
 * chasing. Returns `fresh` when neither timestamp parses or the stage is unknown
 * — an unreadable date must never paint a card red.
 *
 * `now` is injectable so the tests aren't clock-dependent.
 */
export function stalenessOf(
  prospect: Prospect,
  stage: Stage | undefined,
  now: number = Date.now(),
): StalenessReading {
  const contacted = prospect.last_outreach_at != null;
  const since = parseSqliteUtc(prospect.last_outreach_at ?? prospect.created_at);
  if (since == null || !stage) {
    return { level: "fresh", days: 0, contacted };
  }
  // Clamp at zero: a clock skew (or a row written a second in the future) must
  // not produce a negative age that reads as "-0d".
  const days = Math.max(0, Math.floor((now - since) / MS_PER_DAY));
  const level: Staleness =
    days >= stage.stale_days ? "stale" : days >= stage.warn_days ? "warn" : "fresh";
  return { level, days, contacted };
}

/** The card's short age label — "9d", "3w", "2mo". Compact because it sits in a
 *  row of pills on a 260px card, where a full date would crowd out the rest. */
export function formatAge(days: number): string {
  if (days < 7) return `${days}d`;
  if (days < 60) return `${Math.floor(days / 7)}w`;
  return `${Math.floor(days / 30)}mo`;
}

/** The tooltip behind the age pill — the full sentence the pill abbreviates. */
export function describeStaleness(reading: StalenessReading): string {
  const { days, contacted } = reading;
  const ago = days === 0 ? "today" : days === 1 ? "1 day ago" : `${days} days ago`;
  return contacted
    ? `You last messaged them ${ago}`
    : `Never messaged — captured ${ago}`;
}
