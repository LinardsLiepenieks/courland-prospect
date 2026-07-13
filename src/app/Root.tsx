import App from "./App";
import GateScreen from "../gate/GateScreen";
import { useGate } from "../gate/useGate";

/**
 * Top-level gate. The app is hard-gated on a healthy Chrome + a loaded capture
 * extension: until the gate reports `ready`, the gate screen covers everything.
 *
 * `App` stays mounted underneath the gate the whole time — the gate is a fixed
 * opaque overlay, not a swap. A transient not-ready blip (a missed heartbeat, a
 * stale poll) therefore never tears down the working session: the active pitch,
 * tab, and any in-flight edits survive, and autosave hooks don't fire spurious
 * unmount flushes. When readiness recovers the overlay simply lifts.
 */
export default function Root() {
  const status = useGate();

  return (
    <>
      <App />
      {status.state !== "ready" && <GateScreen status={status} />}
    </>
  );
}
