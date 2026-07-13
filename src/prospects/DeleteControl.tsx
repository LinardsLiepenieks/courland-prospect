import { useRef, useState } from "react";
import { MenuItem, Popover } from "../components/Popover";
import { errorMessage } from "../lib/errors";
import styles from "./DeleteControl.module.css";

/** A trash button that opens a Delete / Cancel menu. Shared by the prospect list
 *  rows and the pipeline cards so the delete flow (confirm menu, re-entry guard,
 *  inline error) lives in one place. On success the parent drops the prospect and
 *  this unmounts — no local reset needed. */
export default function DeleteControl({
  name,
  onDelete,
}: {
  name: string;
  onDelete: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Synchronous re-entry guard — state updates are async.
  const deletingRef = useRef(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  async function run() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    setDeleting(true);
    setError(null);
    try {
      await onDelete();
      // Success: the parent unmounts this; leave the menu as-is.
    } catch (err) {
      setError(errorMessage(err));
      setOpen(false);
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    // Stop clicks bubbling to a card/row that would open LinkedIn or start a drag.
    <div className={styles.wrap} onClick={(e) => e.stopPropagation()}>
      <button
        ref={triggerRef}
        type="button"
        className={styles.trash}
        onClick={() => {
          setError(null);
          setOpen((o) => !o);
        }}
        aria-label={`Delete ${name}`}
        aria-haspopup="menu"
        aria-expanded={open}
        title={`Delete ${name}`}
      >
        <TrashIcon />
      </button>
      {error && <span className={styles.error}>{error}</span>}
      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        <MenuItem
          label={deleting ? "Deleting…" : "Delete"}
          danger
          disabled={deleting}
          leading={<TrashIcon />}
          onSelect={run}
        />
        <MenuItem label="Cancel" disabled={deleting} onSelect={() => setOpen(false)} />
      </Popover>
    </div>
  );
}

/** Trash outline, 15×15. `currentColor` so hover can tint it. */
function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M4 7h16M10 11v6M14 11v6M5 7l1 13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1l1-13M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
