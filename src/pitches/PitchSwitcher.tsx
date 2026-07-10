import { useEffect, useRef, useState } from "react";
import type { Pitch } from "../api/pitches";
import styles from "./PitchSwitcher.module.css";

interface Props {
  pitches: Pitch[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onCreateNew: () => void;
}

/**
 * The pitch context switcher. Lists pitches plus a distinct "create new"
 * action. Custom (not a native <select>) so the create action, active check
 * and open/close motion can be styled and keyboard-driven.
 */
export default function PitchSwitcher({
  pitches,
  activeId,
  onSelect,
  onCreateNew,
}: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const active = pitches.find((p) => p.id === activeId) ?? null;

  // Close on outside click or Escape.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: PointerEvent) {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function handleSelect(id: number) {
    onSelect(id);
    setOpen(false);
  }

  function handleCreate() {
    setOpen(false);
    onCreateNew();
  }

  return (
    <div className={styles.root} ref={rootRef}>
      <button
        type="button"
        className={styles.trigger}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className={styles.triggerLabel} data-placeholder={!active}>
          {active ? active.name : "Select a pitch"}
        </span>
        <svg
          className={styles.chevron}
          data-open={open}
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          aria-hidden="true"
        >
          <path
            d="m6 9 6 6 6-6"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>

      {open && (
        <div className={styles.menu} role="menu">
          {pitches.length > 0 && (
            <ul className={styles.list}>
              {pitches.map((pitch) => (
                <li key={pitch.id}>
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={pitch.id === activeId}
                    className={styles.item}
                    data-active={pitch.id === activeId}
                    onClick={() => handleSelect(pitch.id)}
                  >
                    <span className={styles.itemName}>{pitch.name}</span>
                    {pitch.id === activeId && (
                      <svg
                        width="15"
                        height="15"
                        viewBox="0 0 24 24"
                        fill="none"
                        aria-hidden="true"
                      >
                        <path
                          d="m5 12.5 4.5 4.5L19 7"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        />
                      </svg>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {pitches.length > 0 && <div className={styles.divider} />}
          <button
            type="button"
            role="menuitem"
            className={styles.createItem}
            onClick={handleCreate}
          >
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              aria-hidden="true"
            >
              <path
                d="M12 5v14M5 12h14"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
              />
            </svg>
            Create a new pitch
          </button>
        </div>
      )}
    </div>
  );
}
