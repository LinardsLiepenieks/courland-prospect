import { useEffect, useState } from "react";
import { aiAvailable } from "../api/ai";

// The CLI's presence doesn't change while the app runs, so probe once per
// session and share the cached promise — every Polish button reads the same
// result instead of each spawning its own `claude --version`.
let cached: Promise<boolean> | null = null;

function probe(): Promise<boolean> {
  if (!cached) cached = aiAvailable().catch(() => false);
  return cached;
}

/**
 * Whether the local Claude Code CLI is reachable. `null` while the probe is in
 * flight (treat as "assume available" so the UI doesn't flicker disabled on the
 * fast path); `true`/`false` once known.
 */
export function useAiAvailable(): boolean | null {
  const [available, setAvailable] = useState<boolean | null>(null);
  useEffect(() => {
    let active = true;
    probe().then((v) => active && setAvailable(v));
    return () => {
      active = false;
    };
  }, []);
  return available;
}
