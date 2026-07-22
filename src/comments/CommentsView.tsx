import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  deleteCommentDraft,
  getCommentRun,
  listCommentDrafts,
  onCommentsChanged,
  queueCommentDrafts,
  requestCommentScrape,
  updateCommentDraft,
  type CommentDraft,
  type CommentRun,
} from "../api/comments";
import WatchlistSection from "../components/WatchlistSection";
import LoadError from "../components/LoadError";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import styles from "./CommentsView.module.css";

/** How many posts a scrape drafts comments for. Matches the backend clamp. */
const COUNTS = [5, 10, 20, 50];
const DEFAULT_COUNT = 20;

const STATUS_LABEL: Record<CommentDraft["status"], string> = {
  draft: "Draft",
  queued: "Queued",
  posting: "Posting…",
  posted: "Posted",
  failed: "Failed",
};

/** A draft is editable (and counts toward "Post all") until it's posting/posted. */
function isEditable(status: CommentDraft["status"]): boolean {
  return status === "draft" || status === "queued" || status === "failed";
}

/**
 * The Comments tab: the cockpit for the LinkedIn commenter. Scrape the feed +
 * watchlist for new posts (the extension does the browser work), review and edit
 * the AI-drafted comments here, then "Post all" to have the extension auto-submit
 * them, paced. Everything reconciles live off the `comments://changed` event, so a
 * scrape's drafts appear and a post's status updates without a manual refresh.
 */
export default function CommentsView() {
  const [drafts, setDrafts] = useState<CommentDraft[] | null>(null);
  const [run, setRun] = useState<CommentRun | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  // Seed the scrape controls from the persisted run once, then let the user own
  // them — later run updates (status changes) must not reset their selection.
  const [count, setCount] = useState(DEFAULT_COUNT);
  const [includeWatchlist, setIncludeWatchlist] = useState(true);
  const seededRef = useRef(false);

  const fetchSeq = useRef(0);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const reload = useCallback((loud = false) => {
    const seq = ++fetchSeq.current;
    if (loud) setLoadError(null);
    Promise.all([listCommentDrafts(), getCommentRun()])
      .then(([d, r]) => {
        if (!mountedRef.current || seq !== fetchSeq.current) return;
        setDrafts(d);
        setRun(r);
        if (!seededRef.current) {
          seededRef.current = true;
          setCount(COUNTS.includes(r.count) ? r.count : DEFAULT_COUNT);
          setIncludeWatchlist(r.include_watchlist);
        }
      })
      .catch((e) => {
        if (mountedRef.current && seq === fetchSeq.current && loud) setLoadError(errorMessage(e));
      });
  }, []);

  // Initial load (+ retry via reloadKey).
  useEffect(() => {
    reload(true);
  }, [reload, reloadKey]);

  // Reconcile live on backend pushes (a scrape's drafts landing, a post's status
  // changing) — from this app's own commands and from the extension.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    onCommentsChanged(() => reload())
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [reload]);

  const scrape = useAsyncAction();
  const post = useAsyncAction();

  const scraping = run?.status === "requested" || run?.status === "scraping";
  const editableCount =
    drafts?.filter((d) => isEditable(d.status) && d.comment.trim() !== "").length ?? 0;

  function handleScrape() {
    scrape.run(async () => {
      const updated = await requestCommentScrape(count, includeWatchlist);
      setRun(updated);
    });
  }

  function handlePostAll() {
    post.run(async () => {
      await queueCommentDrafts();
      reload();
    });
  }

  async function handleDelete(id: number) {
    await deleteCommentDraft(id);
    // Remove locally on success so the card always disappears — even if the
    // reconciling reload below fails silently, which would otherwise leave the card
    // permanently dimmed and disabled. The reload then re-syncs any other drift.
    setDrafts((cur) => (cur ? cur.filter((d) => d.id !== id) : cur));
    reload();
  }

  if (loadError) {
    return (
      <div className={styles.comments}>
        <LoadError
          what="your comment drafts"
          detail={loadError}
          onRetry={() => setReloadKey((k) => k + 1)}
        />
      </div>
    );
  }

  return (
    <div className={styles.comments}>
      <header className={styles.intro}>
        <h1 className={styles.title}>Comments</h1>
        <p className={styles.subtitle}>
          Scrape LinkedIn for new posts worth engaging, review the drafted
          comments, then post them — the drafts speak in your voice, never a pitch.
        </p>
      </header>

      {/* ── Run control ── */}
      <section className={styles.section}>
        <div className={styles.controlBar}>
          <label className={styles.countField}>
            <span className={styles.countLabel}>Posts to draft</span>
            <select
              className={styles.countSelect}
              value={count}
              onChange={(e) => setCount(Number(e.target.value))}
              disabled={scrape.busy}
            >
              {COUNTS.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>

          <label className={styles.checkField}>
            <input
              type="checkbox"
              checked={includeWatchlist}
              onChange={(e) => setIncludeWatchlist(e.target.checked)}
              disabled={scrape.busy}
            />
            <span>Include watchlist</span>
          </label>

          <button
            type="button"
            className={styles.scrapeBtn}
            onClick={handleScrape}
            disabled={scrape.busy}
            title={
              scraping
                ? "A scrape is running — click to request another (also unsticks a stalled run)"
                : undefined
            }
          >
            {scraping ? "Scrape again" : "Scrape for posts"}
          </button>
        </div>

        {scraping && (
          <p className={styles.status} role="status">
            {run?.status === "requested"
              ? "Waiting for your browser — keep a LinkedIn tab open and signed in."
              : "Scraping the feed and your watchlist… drafts will appear below as they’re ready."}
          </p>
        )}
        {scrape.error && <div className={styles.error}>{scrape.error}</div>}
      </section>

      {/* ── Draft inbox ── */}
      <section className={styles.section}>
        <div className={styles.draftsHeader}>
          <h2 className={styles.sectionTitle}>Drafts</h2>
          <button
            type="button"
            className={styles.postBtn}
            onClick={handlePostAll}
            disabled={post.busy || scraping || editableCount === 0}
            title={
              editableCount === 0
                ? "No drafts ready to post"
                : `Post ${editableCount} comment${editableCount === 1 ? "" : "s"}`
            }
          >
            {post.busy
              ? "Queuing…"
              : editableCount > 0
                ? `Post all (${editableCount})`
                : "Post all"}
          </button>
        </div>
        {post.error && <div className={styles.error}>{post.error}</div>}

        {drafts == null ? (
          <div className={styles.loading} aria-busy="true" aria-hidden="true" />
        ) : drafts.length === 0 ? (
          <p className={styles.empty}>
            No drafts yet. Hit “Scrape for posts” to gather new posts and draft a
            comment for each.
          </p>
        ) : (
          <ul className={styles.list}>
            {drafts.map((d) => (
              <li key={d.id}>
                <DraftCard draft={d} onOpen={openUrl} onDelete={handleDelete} />
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* ── Watchlist ── */}
      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Watchlist</h2>
        <p className={styles.sectionSub}>
          LinkedIn profiles a scrape checks for new posts before the feed — a
          hand-picked list, global, not tied to any pitch.
        </p>
        <WatchlistSection />
      </section>
    </div>
  );
}

/**
 * One draft: the post it's for (author + text + a link out), an editable comment,
 * and its status. The comment saves on blur (never mid-keystroke, so an incoming
 * live reload can't clobber a word being typed): local text is authoritative while
 * the field is dirty, and re-syncs from the backend only when it isn't.
 */
function DraftCard({
  draft,
  onOpen,
  onDelete,
}: {
  draft: CommentDraft;
  onOpen: (url: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
}) {
  const editable = isEditable(draft.status);
  const [text, setText] = useState(draft.comment);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  // True while the field holds unsaved edits — freezes external re-sync so a live
  // reload doesn't overwrite what's being typed.
  const dirtyRef = useRef(false);
  const deletingRef = useRef(false);

  useEffect(() => {
    if (!dirtyRef.current) setText(draft.comment);
  }, [draft.comment]);

  async function saveIfDirty() {
    if (!dirtyRef.current) return;
    dirtyRef.current = false;
    if (text === draft.comment) return;
    try {
      await updateCommentDraft(draft.id, text);
      setSaveError(null);
    } catch (e) {
      // Rejected (e.g. it started posting) — reset to the server's text on the
      // next reload and tell the user their edit didn't stick.
      setSaveError(errorMessage(e));
    }
  }

  async function handleDelete() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    setDeleting(true);
    try {
      await onDelete(draft.id);
    } catch (e) {
      setSaveError(errorMessage(e));
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  const author = draft.author_name.trim() || "LinkedIn member";

  return (
    <div className={styles.card} data-deleting={deleting}>
      <div className={styles.cardHead}>
        <span className={styles.author}>{author}</span>
        <span className={styles.badge} data-status={draft.status}>
          {STATUS_LABEL[draft.status]}
        </span>
      </div>

      {draft.post_text && <p className={styles.postText}>{draft.post_text}</p>}

      <textarea
        className={styles.comment}
        value={text}
        onChange={(e) => {
          dirtyRef.current = true;
          setText(e.target.value);
        }}
        onBlur={() => void saveIfDirty()}
        disabled={!editable}
        placeholder="Your comment…"
        rows={3}
      />

      {draft.status === "failed" && draft.error && (
        <p className={styles.failure}>Couldn’t post: {draft.error}</p>
      )}
      {saveError && <p className={styles.failure}>{saveError}</p>}

      <div className={styles.cardActions}>
        <button
          type="button"
          className={styles.openBtn}
          onClick={() => {
            // Surface an opener failure (a malformed scraped permalink) instead of
            // swallowing it — scraped URLs are lower-trust than hand-entered ones.
            onOpen(draft.permalink).catch((e) => setSaveError(errorMessage(e)));
          }}
          title="Open the post in your browser"
        >
          Open post
        </button>
        <button
          type="button"
          className={styles.deleteBtn}
          onClick={() => void handleDelete()}
          disabled={deleting}
          title="Dismiss this draft"
        >
          Dismiss
        </button>
      </div>
    </div>
  );
}
