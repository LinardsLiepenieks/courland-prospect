import { useEffect, useState } from "react";

/** One hour, the interval the board re-grades staleness on. */
const HOUR_MS = 3_600_000;

/**
 * A `Date.now()` that advances on its own, so time-derived UI doesn't freeze at
 * whenever the component happened to mount.
 *
 * The Prospects board grades every card against its stage's day thresholds. Read
 * once at render, a board left open overnight keeps yesterday's verdict — a
 * prospect crossing `warn_days` at 03:00 stays untinted until some unrelated
 * state change forces a re-render. For a feature whose whole job is surfacing
 * people going cold on a board you leave open, that's the wrong failure.
 *
 * Hourly rather than by the minute: the thresholds are measured in days, so an
 * hour is already an order of magnitude finer than the smallest distinction the
 * UI can draw, and it costs one re-render an hour instead of sixty.
 */
export function useNow(intervalMs: number = HOUR_MS): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    // A laptop that slept through the boundary fires no interval, so also
    // re-read whenever the window comes back to the foreground — that's the
    // moment someone is actually looking at the board again.
    const refresh = () => setNow(Date.now());
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      clearInterval(id);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [intervalMs]);

  return now;
}
