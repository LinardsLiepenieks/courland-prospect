import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "./errors";

interface Result<T> {
  /** The loaded value, or `null` while the first load is in flight. */
  data: T | null;
  error: string | null;
  /** Replace the value locally after a write that already persisted, so the view
   *  doesn't have to re-fetch to reflect its own edit. */
  setData: React.Dispatch<React.SetStateAction<T | null>>;
  /** Re-run the load — wired to the retry button on the error screen. */
  retry: () => void;
}

/**
 * Load a record once on mount, with a retryable error.
 *
 * The three guards this bakes in were previously hand-written at every call
 * site: an `active` flag so a resolve after unmount (including StrictMode's
 * double-mount) is dropped, a paired `.catch` so a failed load can't surface as
 * an unhandled rejection, and a reload counter behind `retry`.
 *
 * `load` is read through a ref, so an inline arrow doesn't re-trigger the fetch
 * on every render — same approach as `useAutosave`'s `persist`.
 *
 * For anything more than load-once-then-mutate-locally — a view coordinating two
 * fetches, or one that reloads in the background and must not let a slow response
 * clobber a newer one — reach for a bespoke effect instead; this deliberately
 * doesn't grow those cases.
 */
export function useLoad<T>(load: () => Promise<T>): Result<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    let active = true;
    setError(null);
    loadRef
      .current()
      .then((v) => {
        if (active) setData(v);
      })
      .catch((e) => {
        if (active) setError(errorMessage(e));
      });
    return () => {
      active = false;
    };
  }, [reloadKey]);

  const retry = useCallback(() => setReloadKey((k) => k + 1), []);

  return { data, error, setData, retry };
}
