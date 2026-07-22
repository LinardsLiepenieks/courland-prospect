import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  addWatchedProfile,
  deleteWatchedProfile,
  listWatchedProfiles,
  type WatchedProfile,
} from "../api/watchlist";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import LoadError from "./LoadError";
import styles from "./WatchlistSection.module.css";

/**
 * The Watchlist editor (mounted in the Comments tab, under the run controls): a
 * hand-curated list of LinkedIn profiles a comment run checks for new posts. An add form (profile URL + an
 * optional label) sits on top; the list below shows each watched profile with
 * an "Open" affordance and a delete. The list is small and app-wide (deduped on
 * URL by the backend), so it loads once and reconciles from the DB after every
 * mutation rather than editing locally.
 */
export default function WatchlistSection() {
  const [profiles, setProfiles] = useState<WatchedProfile[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const { busy: adding, error: addError, run } = useAsyncAction();
  // Tracks the latest issued load so an older, slower response can't overwrite it.
  const fetchSeq = useRef(0);
  // Guards a resolve after unmount (skips the stale work).
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // The single fetch path. Every issue bumps `fetchSeq`; a response applies only
  // if it's still the newest issued — so a post-mutation reload and the initial
  // load can't clobber one another out of order. `loud` surfaces a failure as the
  // retryable error screen; a background refresh stays silent.
  const loadWatchlist = useCallback((loud = false) => {
    const seq = ++fetchSeq.current;
    if (loud) setLoadError(null);
    listWatchedProfiles()
      .then((p) => {
        if (mountedRef.current && seq === fetchSeq.current) setProfiles(p);
      })
      .catch((e) => {
        if (mountedRef.current && seq === fetchSeq.current && loud)
          setLoadError(errorMessage(e));
      });
  }, []);

  // Initial load (and retry via `reloadKey`).
  useEffect(() => {
    loadWatchlist(true);
  }, [loadWatchlist, reloadKey]);

  function handleAdd(e: FormEvent) {
    e.preventDefault();
    run(async () => {
      // Accept a bare "linkedin.com/in/…" paste by prepending https:// when no scheme
      // is present, so the stored URL is well-formed — the row's Open action and the
      // extension both expect a scheme. (The input is type="text", not type="url", so
      // the browser doesn't reject a scheme-less paste that the backend accepts.)
      const trimmed = url.trim();
      const normalized = trimmed && !/^https?:\/\//i.test(trimmed) ? `https://${trimmed}` : trimmed;
      // The backend validates the URL and dedups on it (re-adding updates the
      // label), so reload from the DB rather than prepending locally — that way
      // an updated existing row moves/renames correctly and can't duplicate.
      await addWatchedProfile(normalized, name.trim());
      setUrl("");
      setName("");
      loadWatchlist();
    });
  }

  // Delete owns its API call so the row can dim/revert on failure (it stays
  // mounted until the reload removes it). Reconciling from the DB rather than
  // filtering locally keeps the list authoritative.
  async function handleDelete(id: number) {
    await deleteWatchedProfile(id);
    // Remove locally on success so the row always disappears — even if the
    // reconciling reload below fails silently, which would otherwise leave the row
    // permanently dimmed and disabled. The reload then re-syncs any other drift.
    setProfiles((cur) => (cur ? cur.filter((p) => p.id !== id) : cur));
    loadWatchlist();
  }

  if (loadError) {
    return (
      <LoadError
        what="your watchlist"
        detail={loadError}
        onRetry={() => setReloadKey((k) => k + 1)}
      />
    );
  }
  if (!profiles) {
    // Local SQLite resolves near-instantly; reserve space so the layout doesn't
    // jump when it lands.
    return <div className={styles.loading} aria-busy="true" aria-hidden="true" />;
  }

  return (
    <div className={styles.watchlist}>
      <form className={styles.addForm} onSubmit={handleAdd}>
        <div className={styles.fields}>
          <input
            className={styles.urlInput}
            type="text"
            inputMode="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://www.linkedin.com/in/…"
            aria-label="LinkedIn profile URL"
            disabled={adding}
          />
          <input
            className={styles.nameInput}
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Label (optional)"
            aria-label="Label"
            disabled={adding}
          />
        </div>
        <button
          type="submit"
          className={styles.addBtn}
          disabled={adding || url.trim() === ""}
        >
          {adding ? "Adding…" : "Add"}
        </button>
      </form>

      {addError && <div className={styles.error}>{addError}</div>}

      {profiles.length === 0 ? (
        <p className={styles.empty}>
          No watched profiles yet. Add a LinkedIn profile link to check it for
          new posts.
        </p>
      ) : (
        <ul className={styles.list}>
          {profiles.map((p) => (
            <li key={p.id}>
              <WatchlistRow profile={p} onOpen={openUrl} onDelete={handleDelete} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * One watched profile. The label (or the URL, when unlabeled) is the primary
 * line; the URL sits beneath as a quiet secondary. "Open" launches the profile
 * in the user's default browser via the opener plugin (the same mechanism the
 * prospects list uses); delete fires immediately, dimming the row until the
 * parent's reload removes it — resetting state only on failure, since success
 * unmounts the row.
 */
function WatchlistRow({
  profile,
  onOpen,
  onDelete,
}: {
  profile: WatchedProfile;
  onOpen: (url: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
}) {
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Synchronous re-entry guard (state updates are async).
  const deletingRef = useRef(false);

  const trimmedName = profile.name.trim();
  const title = trimmedName || profile.linkedin_url;
  // Only show the URL as a second line when it isn't already the title.
  const showUrl = trimmedName !== "";

  async function handleDelete() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    setDeleting(true);
    setError(null);
    try {
      await onDelete(profile.id);
      // On success the row is removed and this unmounts — no state to reset.
    } catch (err) {
      setError(errorMessage(err));
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    <div className={styles.row} data-deleting={deleting}>
      <div className={styles.rowText}>
        <span className={styles.rowTitle}>{title}</span>
        {showUrl && <span className={styles.rowUrl}>{profile.linkedin_url}</span>}
        {error && <span className={styles.rowError}>{error}</span>}
      </div>

      <div className={styles.rowActions}>
        <button
          type="button"
          className={styles.openBtn}
          onClick={() => {
            // Surface an opener failure instead of swallowing it (reuses the row's
            // error line).
            onOpen(profile.linkedin_url).catch((err) => setError(errorMessage(err)));
          }}
          disabled={deleting}
          title="Open in browser"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M14 4h6v6M20 4l-8.5 8.5M18 13.5V19a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5.5"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          Open
        </button>
        <button
          type="button"
          className={styles.deleteBtn}
          onClick={() => void handleDelete()}
          disabled={deleting}
          aria-label={`Remove ${title}`}
          title="Remove"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m2 0v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V7"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}
