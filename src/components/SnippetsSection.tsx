import { useEffect, useRef, useState } from "react";
import {
  createSnippet,
  deleteSnippet,
  listSnippets,
  updateSnippet,
  type Snippet,
} from "../api/snippets";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import { useAutosave } from "../lib/useAutosave";
import LoadError from "./LoadError";
import SavedIndicator from "./SavedIndicator";
import styles from "./SnippetsSection.module.css";

interface Props {
  /** The scope: a pitch id for that pitch's snippets, or `null` for the global
   *  profile snippets. */
  pitchId: number | null;
}

/**
 * The Snippets editor, reused in both the Profile tab (`pitchId={null}`) and a
 * pitch's Settings tab (`pitchId={pitch.id}`). Lists the scope's snippets
 * newest-first, appends a blank card on "Add", and lets each card autosave its
 * name + content. Snippets are just stored for now — nothing consumes them yet.
 *
 * Keyed by `pitchId` where it's used so switching scope remounts with fresh
 * state (see the wiring in EditPitchView).
 */
export default function SnippetsSection({ pitchId }: Props) {
  const [snippets, setSnippets] = useState<Snippet[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);
  const { busy: adding, error, run } = useAsyncAction();

  useEffect(() => {
    // `active` short-circuits a resolve after unmount (incl. StrictMode's
    // double-mount) and pairs with `.catch` so a failed load can't become an
    // unhandled rejection.
    let active = true;
    setLoadError(null);
    listSnippets(pitchId)
      .then((s) => active && setSnippets(s))
      .catch((e) => active && setLoadError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, [pitchId, reloadKey]);

  function handleAdd() {
    run(async () => {
      const created = await createSnippet(pitchId);
      // Newest-first, matching the backend list order.
      setSnippets((prev) => [created, ...(prev ?? [])]);
    });
  }

  // Delete owns the API call so the card can revert/show an error on failure.
  // On success we drop the row locally — the card unmounts.
  async function handleDelete(id: number) {
    await deleteSnippet(id);
    setSnippets((prev) => prev?.filter((s) => s.id !== id) ?? null);
  }

  if (loadError) {
    return (
      <LoadError
        what="snippets"
        detail={loadError}
        onRetry={() => setReloadKey((k) => k + 1)}
      />
    );
  }
  if (!snippets) {
    // Local SQLite resolves near-instantly; reserve space so the layout doesn't
    // jump when it lands.
    return <div className={styles.loading} aria-busy="true" aria-hidden="true" />;
  }

  return (
    <div className={styles.snippets}>
      {snippets.length === 0 ? (
        <p className={styles.empty}>
          No snippets yet. Add one to start building a library.
        </p>
      ) : (
        <ul className={styles.list}>
          {snippets.map((s) => (
            <li key={s.id}>
              <SnippetCard snippet={s} onDelete={handleDelete} />
            </li>
          ))}
        </ul>
      )}

      {error && <div className={styles.error}>{error}</div>}

      <button
        type="button"
        className={styles.addBtn}
        onClick={handleAdd}
        disabled={adding}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path
            d="M12 5v14M5 12h14"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          />
        </svg>
        Add snippet
      </button>
    </div>
  );
}

/**
 * One snippet card: a name field and a content field, each autosaved after a
 * short typing pause (via `useAutosave`), plus an inline-confirm delete. The
 * unmount flush is skipped mid-delete — the row is on its way out.
 */
function SnippetCard({
  snippet,
  onDelete,
}: {
  snippet: Snippet;
  onDelete: (id: number) => Promise<void>;
}) {
  const [name, setName] = useState(snippet.name);
  const [content, setContent] = useState(snippet.content);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // Synchronous re-entry guard (state updates are async); also tells the
  // autosave's unmount flush to stand down once a delete is underway.
  const deletingRef = useRef(false);

  const { showSaved, error, setError } = useAutosave({
    values: { name, content },
    persist: (v) => updateSnippet(snippet.id, v.name, v.content),
    canFlush: () => !deletingRef.current,
  });

  async function handleDelete() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    setDeleting(true);
    setError(null);
    try {
      await onDelete(snippet.id);
      // On success the row is removed and this card unmounts — no state to reset.
    } catch (err) {
      setError(errorMessage(err));
      setConfirmingDelete(false);
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    <div className={styles.card} data-deleting={deleting}>
      <div className={styles.cardHead}>
        <input
          className={styles.nameInput}
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Snippet name"
          aria-label="Snippet name"
          disabled={deleting}
        />
        {confirmingDelete ? (
          <div className={styles.confirm}>
            <button
              type="button"
              className={styles.secondaryBtn}
              onClick={() => setConfirmingDelete(false)}
              disabled={deleting}
            >
              Cancel
            </button>
            <button
              type="button"
              className={styles.deleteBtn}
              onClick={() => void handleDelete()}
              disabled={deleting}
            >
              {deleting ? "Deleting…" : "Delete"}
            </button>
          </div>
        ) : (
          <button
            type="button"
            className={styles.removeBtn}
            onClick={() => setConfirmingDelete(true)}
            aria-label="Delete snippet"
            title="Delete snippet"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path
                d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m2 0v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V7"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        )}
      </div>

      <textarea
        className={styles.contentInput}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="What this snippet says…"
        aria-label="Snippet content"
        disabled={deleting}
      />

      {error && <div className={styles.cardError}>{error}</div>}

      <div className={styles.cardFoot}>
        <SavedIndicator visible={showSaved} />
      </div>
    </div>
  );
}
