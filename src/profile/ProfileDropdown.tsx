import { useEffect, useRef, useState } from "react";
import {
  listChromeProfiles,
  openChromeProfile,
  type ChromeProfile,
} from "../api/chromeProfiles";
import { MenuItem, Popover } from "../components/Popover";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import styles from "./ProfileDropdown.module.css";

/** Where the last-selected profile is remembered (frontend-only; the backend
 *  doesn't need it — opening just takes a dir). */
const STORAGE_KEY = "cp.capture.profile";

/** Pick which Chrome profile to capture with, then open it. The dropdown only
 *  *selects* (persisting the choice); the Open button launches the selected
 *  profile at LinkedIn. Nothing opens until Open is pressed. The gate flips to
 *  ready on its own once the extension checks in. Used on the gate's
 *  "Chrome is closed" screen (primary) and the Profile tab (secondary). */
export default function ProfileDropdown({
  primary = false,
  openLabel = "Open",
}: {
  primary?: boolean;
  openLabel?: string;
}) {
  const [profiles, setProfiles] = useState<ChromeProfile[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const { busy, error, setError, run } = useAsyncAction();
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let active = true;
    listChromeProfiles()
      .then((list) => {
        if (!active) return;
        setProfiles(list);
        // Pre-select the remembered profile if it still exists, else the first.
        const stored = readStored();
        setSelected(
          list.find((p) => p.dir === stored)?.dir ?? list[0]?.dir ?? null,
        );
      })
      .catch((e) => active && setError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, []);

  function choose(dir: string) {
    setOpen(false);
    setSelected(dir);
    try {
      localStorage.setItem(STORAGE_KEY, dir);
    } catch {
      // Private-mode / storage-disabled — selection just won't persist.
    }
  }

  function launch() {
    if (!selected) return;
    run(() => openChromeProfile(selected));
  }

  const selectedProfile = profiles?.find((p) => p.dir === selected) ?? null;

  return (
    <div className={styles.wrap}>
      <div className={styles.row}>
        <button
          ref={triggerRef}
          type="button"
          className={styles.trigger}
          onClick={() => setOpen((o) => !o)}
          disabled={!profiles}
          aria-haspopup="menu"
          aria-expanded={open}
        >
          <ProfileIcon />
          <span className={styles.label}>{selectedProfile?.name ?? "Default"}</span>
          <ChevronIcon />
        </button>

        <button
          type="button"
          className={primary ? styles.openPrimary : styles.openBtn}
          onClick={launch}
          disabled={busy || !selected}
        >
          {busy ? "Opening…" : openLabel}
        </button>
      </div>

      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        {profiles?.map((p) => (
          <MenuItem
            key={p.dir}
            label={p.name}
            checked={p.dir === selected}
            onSelect={() => choose(p.dir)}
          />
        ))}
      </Popover>

      {error && <span className={styles.error}>{error}</span>}
    </div>
  );
}

/** The remembered profile dir, or null if none/unreadable. */
function readStored(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function ProfileIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8ZM5 20a7 7 0 0 1 14 0"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.chevron}>
      <path
        d="m6 9 6 6 6-6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
