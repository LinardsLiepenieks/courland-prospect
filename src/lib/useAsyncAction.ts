import { useRef, useState } from "react";
import { errorMessage } from "./errors";

interface Result {
  /** True while the action is running — drive button disabled/label off this. */
  busy: boolean;
  /** The last failure message, or null. Cleared when a new run starts. */
  error: string | null;
  setError: (e: string | null) => void;
  /** Run an async action once at a time: a synchronous re-entry guard drops a
   *  second call fired in the same frame (Enter-repeat, Enter+click), `busy`
   *  toggles around it, and a rejection is caught into `error`. */
  run: (fn: () => Promise<unknown>) => void;
}

/**
 * The shared "one async action, with a busy flag and a caught error" pattern —
 * previously copy-pasted as a `xxxRef` + `busy` state + try/catch/finally in
 * every form (create pitch, add snippet, open Chrome profile).
 *
 * Scoped to actions where the component **stays mounted** across the action
 * (the reset runs in `finally`). Delete-then-unmount flows deliberately keep
 * their own handler — they reset only on failure because success unmounts the
 * component, so a `finally` reset would setState on a gone component.
 */
export function useAsyncAction(): Result {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Synchronous re-entry guard (state updates are async).
  const busyRef = useRef(false);

  const run = useRef((fn: () => Promise<unknown>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    void (async () => {
      try {
        await fn();
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    })();
  }).current;

  return { busy, error, setError, run };
}
