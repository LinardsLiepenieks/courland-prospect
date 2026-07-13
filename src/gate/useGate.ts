import { useEffect, useState } from "react";
import { gateStatus, onGateStatus, type GateStatus } from "../api/gate";

/** How often to poll as the reliable floor beneath the push events. */
const POLL_MS = 3000;

/** Same gate reading — used to skip a redundant re-render (the poll produces a
 *  fresh object every tick even when nothing changed). */
function sameStatus(a: GateStatus, b: GateStatus): boolean {
  if (a.state !== b.state) return false;
  return (a.state === "error" ? a.detail : null) === (b.state === "error" ? b.detail : null);
}

/**
 * Track the backend gate status. Events are the fast path; a poll is the
 * reliable floor (so a missed event can never leave the UI stuck). Starts
 * optimistically in `initializing`.
 *
 * Readings are ordered by a monotonic sequence stamped at dispatch: a slow
 * in-flight poll can otherwise resolve *after* a fresher event (or a later
 * poll) and clobber it back to a stale state — which, since the gate overlays
 * the app, would flash the gate screen over a working session. Only a reading
 * at least as new as the last applied one wins.
 */
export function useGate(): GateStatus {
  const [status, setStatus] = useState<GateStatus>({ state: "initializing" });

  useEffect(() => {
    let active = true;
    let latest = 0; // last sequence handed out
    let applied = 0; // sequence of the reading currently reflected

    const apply = (seq: number, s: GateStatus) => {
      if (!active || seq < applied) return;
      applied = seq;
      setStatus((prev) => (sameStatus(prev, s) ? prev : s));
    };

    const poll = () => {
      const seq = ++latest;
      gateStatus().then((s) => apply(seq, s)).catch(() => {});
    };

    // A push event reflects the backend at emit time — stamp it newest.
    const unlisten = onGateStatus((s) => apply(++latest, s));

    poll();
    const timer = window.setInterval(poll, POLL_MS);

    return () => {
      active = false;
      window.clearInterval(timer);
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, []);

  return status;
}
