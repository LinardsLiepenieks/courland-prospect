import { useEffect, useRef, useState } from "react";
import { errorMessage } from "./errors";

/** How long typing must pause before an autosave fires. Shared by every
 *  autosaving surface (the Profile "about you" form, snippet cards). */
export const AUTOSAVE_DELAY_MS = 600;

type Values = Record<string, string>;

function trimAll(v: Values): Values {
  const out: Values = {};
  for (const k in v) out[k] = v[k].trim();
  return out;
}

function equal(a: Values, b: Values): boolean {
  for (const k in a) {
    if (a[k] !== b[k]) return false;
  }
  return true;
}

interface Options {
  /** The current (raw, untrimmed) field values, keyed by field. */
  values: Values;
  /** Persist the trimmed values. Resolves when the write lands. */
  persist: (trimmed: Values) => Promise<unknown>;
  /** Hold autosave off while true (e.g. a polish is in flight). */
  hold?: boolean;
  /** Evaluated at unmount: return false to skip the final flush (e.g. the row
   *  is being deleted, so there's nothing to persist). */
  canFlush?: () => boolean;
}

interface Result {
  saving: boolean;
  /** Whether the "Saved" confirmation should show (saved, clean, idle). */
  showSaved: boolean;
  dirty: boolean;
  error: string | null;
  setError: (e: string | null) => void;
  /** Flush immediately (a manual Save), skipping the debounce. */
  save: () => void;
}

/**
 * Debounced autosave for a set of text fields. Encapsulates the guards both
 * autosaving surfaces need, previously copy-pasted between them:
 *
 *  - compares *trimmed* input against the last *submitted* (JS-trimmed) values,
 *    not the backend echo — Rust's trim strips a few code points JS's leaves, so
 *    comparing the echo would keep `dirty` true forever and spin the timer;
 *  - a synchronous re-entry ref so an autosave timer and a manual click in the
 *    same frame can't both fire a write;
 *  - a failed-signature guard so a persistent write failure doesn't turn the
 *    debounce into a tight retry loop (retries wait for a fresh edit / manual Save);
 *  - an unmount flush that chains after any in-flight save (writes never
 *    reorder) so edits made in the last <AUTOSAVE_DELAY_MS aren't lost on a tab
 *    switch — a no-op when nothing changed (incl. StrictMode's extra unmount).
 *
 * The baseline is seeded from the first render's values; callers remount with
 * fresh data when the subject changes (keyed), so it needn't track external
 * updates.
 */
export function useAutosave({ values, persist, hold = false, canFlush }: Options): Result {
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Last values known to be persisted (trimmed). Seeded once from initial input.
  const savedRef = useRef(trimAll(values));
  // Synchronous re-entry guard (state updates are async).
  const savingRef = useRef(false);
  // In-flight save, so the unmount flush chains after it (never reorders writes).
  const inflightRef = useRef<Promise<unknown> | null>(null);
  // Signature of the last submission that failed to persist. While the input
  // still matches it, the debounce won't auto-retry — otherwise a persistent
  // failure (validation reject, locked DB) becomes a tight 600ms hammer loop
  // with no keystroke. A fresh edit (new signature) or a manual Save retries.
  const failedSigRef = useRef<string | null>(null);
  // Latest raw values + persist fn, mirrored so the stable callbacks below and
  // the unmount cleanup (which runs after the last render) read fresh values.
  const latestRef = useRef(values);
  latestRef.current = values;
  const persistRef = useRef(persist);
  persistRef.current = persist;
  const canFlushRef = useRef(canFlush);
  canFlushRef.current = canFlush;

  const dirty = !equal(trimAll(values), savedRef.current);
  const showSaved = saved && !dirty && !saving;
  // A change signature the effects can depend on without re-running each render.
  // JSON (not a space-join) so a space shifting across a field boundary still
  // registers as a change — `{a:"x y",b:"z"}` and `{a:"x",b:"y z"}` must differ.
  const key = JSON.stringify(values);

  // One stable save callback that reads the latest values via refs. `manual` is
  // a user-forced Save, which retries even a signature that just failed.
  const save = useRef(async (manual: boolean) => {
    if (savingRef.current) return;
    const next = trimAll(latestRef.current);
    // A debounce that fires after a manual/earlier save already flushed these
    // values has nothing to do.
    if (equal(next, savedRef.current)) return;
    const sig = JSON.stringify(next);
    // Don't auto-retry the exact input that just failed; wait for a new edit.
    if (!manual && sig === failedSigRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setError(null);
    const promise = persistRef.current(next);
    inflightRef.current = promise;
    try {
      await promise;
      savedRef.current = next;
      failedSigRef.current = null;
      setSaved(true);
    } catch (err) {
      // Keep the edits on screen and remember what failed so the debounce
      // doesn't spin; a new edit or a manual Save clears the block and retries.
      failedSigRef.current = sig;
      setError(errorMessage(err));
    } finally {
      if (inflightRef.current === promise) inflightRef.current = null;
      savingRef.current = false;
      setSaving(false);
    }
  }).current;

  // A fresh edit invalidates the "Saved" confirmation.
  useEffect(() => {
    setSaved(false);
  }, [key]);

  // Debounced autosave. Re-armed on every keystroke (cleanup clears the prior
  // timer), and re-run when a save settles so mid-save edits still flush. Held
  // off while a save is in flight or `hold` is set.
  useEffect(() => {
    if (!dirty || saving || hold) return;
    const id = window.setTimeout(() => void save(false), AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, dirty, saving, hold]);

  // Flush edits made in the last <AUTOSAVE_DELAY_MS when the component unmounts.
  useEffect(() => {
    return () => {
      if (canFlushRef.current && !canFlushRef.current()) return;
      const flush = () => {
        const next = trimAll(latestRef.current);
        if (!equal(next, savedRef.current)) return persistRef.current(next);
      };
      Promise.resolve(inflightRef.current)
        .catch(() => {})
        .then(flush)
        .catch((err) => {
          // The component is already gone, so there's no UI to surface this on
          // and no chance to retry — log it so a lost final flush is at least
          // diagnosable rather than completely silent.
          console.warn("useAutosave: final flush failed; last edits may be unsaved", err);
        });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { saving, showSaved, dirty, error, setError, save: () => void save(true) };
}
