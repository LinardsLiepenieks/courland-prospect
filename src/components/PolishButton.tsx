import { useRef, useState } from "react";
import { errorMessage } from "../lib/errors";
import { useAiAvailable } from "../lib/useAiAvailable";
import styles from "./PolishButton.module.css";

interface Props {
  /** The current text to polish. */
  text: string;
  /** The command that rewrites `text` (e.g. `polishSkill`, `polishWho`). Kept as
   *  a prop so the button is decoupled from any one field/command. */
  polish: (text: string) => Promise<string>;
  /** Disable externally (e.g. while the form is saving/submitting). */
  disabled?: boolean;
  /** Called with the rewritten text on success. */
  onPolished: (text: string) => void;
  /** Report a failure message, or `null` when a fresh attempt clears prior errors. */
  onError: (message: string | null) => void;
  /** Notify the parent when a polish starts/ends, so it can lock the skill field
   *  (and Save) while the rewrite is in flight — otherwise text typed during the
   *  call would be clobbered by the incoming result. */
  onBusyChange?: (busy: boolean) => void;
}

/** A quiet action that rewrites the skill through the local Claude Code CLI.
 *  Non-destructive: it hands the result to `onPolished` for the caller to drop
 *  into the editor — the user still saves manually. */
export default function PolishButton({
  text,
  polish,
  disabled,
  onPolished,
  onError,
  onBusyChange,
}: Props) {
  const [polishing, setPolishing] = useState(false);
  // Synchronous re-entry guard — state updates are async, matching the form pattern.
  const polishingRef = useRef(false);
  const aiReady = useAiAvailable();
  // Only treat as unavailable once the probe has resolved false — while it's
  // still `null`, stay optimistic so the button doesn't flicker disabled.
  const unavailable = aiReady === false;

  const canPolish =
    text.trim().length > 0 && !disabled && !polishing && !unavailable;

  async function handlePolish() {
    if (!canPolish || polishingRef.current) return;
    polishingRef.current = true;
    setPolishing(true);
    onBusyChange?.(true);
    onError(null);
    try {
      onPolished(await polish(text));
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      polishingRef.current = false;
      setPolishing(false);
      onBusyChange?.(false);
    }
  }

  return (
    <button
      type="button"
      className={styles.polish}
      onClick={handlePolish}
      disabled={!canPolish}
      aria-busy={polishing}
      title={
        unavailable
          ? "Polish needs Claude Code — install it and reopen the app."
          : "Rewrite this with Claude Code"
      }
    >
      <span className={styles.icon} aria-hidden="true">
        {polishing ? (
          <svg className={styles.spinner} width="14" height="14" viewBox="0 0 24 24" fill="none">
            <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
            <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
          </svg>
        ) : (
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <path
              d="M12 3l1.9 4.6L18.5 9.5 13.9 11.4 12 16 10.1 11.4 5.5 9.5 10.1 7.6 12 3Z"
              fill="currentColor"
            />
            <path d="M18 14l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7L18 14Z" fill="currentColor" />
          </svg>
        )}
      </span>
      {polishing ? "Polishing…" : "Polish"}
    </button>
  );
}
