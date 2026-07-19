import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  approveSnippet,
  copySnippet,
  createSnippet,
  deleteSnippet,
  listSnippets,
  onSnippetsChanged,
  setSnippetCategory,
  updateSnippet,
  type Snippet,
} from "../api/snippets";
import { listPitches, type Pitch } from "../api/pitches";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import { useAutosave } from "../lib/useAutosave";
import { MenuItem, Popover } from "./Popover";
import LoadError from "./LoadError";
import SavedIndicator from "./SavedIndicator";
import styles from "./SnippetsSection.module.css";

interface Props {
  /** The scope: a pitch id for that pitch's snippets, or `null` for the global
   *  profile snippets. */
  pitchId: number | null;
}

/** A destination the "Copy to…" menu can send a snippet to. `id` is a pitch id, or
 *  `null` for the global profile. */
interface CopyTarget {
  id: number | null;
  name: string;
}

/**
 * The Snippets editor, reused in both the Profile tab (`pitchId={null}`) and a
 * pitch's Settings tab (`pitchId={pitch.id}`). Proposed (AI-suggested) snippets
 * sit on top as an amber triage queue; approved snippets below are a flat list of
 * collapsibles — the name is always visible, and opening one reveals the editable
 * content. Each row's context menu (right-click, or the ⋯ button) copies the
 * snippet into another pitch (or the profile) as an independent duplicate, or
 * deletes it.
 *
 * Keyed by `pitchId` where it's used so switching scope remounts with fresh state.
 */
export default function SnippetsSection({ pitchId }: Props) {
  const [snippets, setSnippets] = useState<Snippet[] | null>(null);
  const [pitches, setPitches] = useState<Pitch[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);
  const { busy: adding, error, run } = useAsyncAction();
  // Tracks the latest issued load so an older, slower response can't overwrite it.
  const fetchSeq = useRef(0);
  // Guards a resolve after unmount (React no-ops the setState, but this skips the
  // stale work and mirrors the previous `active` flag).
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // The single fetch path. Every issue bumps `fetchSeq`; a response applies only if
  // it's still the newest issued — so a background `snippets://changed` refresh, a
  // post-mutation reload, and the initial load can't clobber one another out of
  // order. `loud` surfaces a failure as the retryable error screen; a background
  // refresh stays silent (whatever's on screen is still valid).
  const loadSnippets = useCallback(
    (loud = false) => {
      const seq = ++fetchSeq.current;
      if (loud) setLoadError(null);
      listSnippets(pitchId)
        .then((s) => {
          if (mountedRef.current && seq === fetchSeq.current) setSnippets(s);
        })
        .catch((e) => {
          if (mountedRef.current && seq === fetchSeq.current && loud)
            setLoadError(errorMessage(e));
        });
    },
    [pitchId],
  );

  // Initial load (and retry via `reloadKey`).
  useEffect(() => {
    loadSnippets(true);
  }, [loadSnippets, reloadKey]);

  // The pitch list backs the "Copy to…" menu's targets. A failure here isn't fatal —
  // the menu just offers fewer destinations — so it's fetched separately and stays
  // silent on error (never blocks the snippet editor).
  useEffect(() => {
    let active = true;
    listPitches()
      .then((p) => {
        if (active) setPitches(p);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  // Live-refresh when a background pass changes THIS scope's snippets — a new
  // proposal, a classify pass updating a snippet's position/category, or a copy
  // landing in this scope. The event payload is the affected scope (a pitch id, or
  // `null` for the profile), so both scopes subscribe and match on equality. The
  // reload goes through `loadSnippets`, so it can't clobber (or be clobbered by) a
  // concurrent user action.
  useEffect(() => {
    let active = true;
    const unlisten = onSnippetsChanged((changedScope) => {
      if (active && changedScope === pitchId) loadSnippets();
    });
    return () => {
      active = false;
      // Tauri's `listen` resolves once registration completes (it effectively
      // never rejects), but catch anyway so teardown can't leak an unhandled
      // rejection.
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [pitchId, loadSnippets]);

  function handleAdd() {
    run(async () => {
      const created = await createSnippet(pitchId);
      // Guard against a live-refresh (`snippets://changed`) having already folded
      // this row in — the created snippet is committed before this resolves, so a
      // concurrent refetch can beat us here; dropping the duplicate avoids a double
      // card / duplicate React key. A blank card sorts to the top of the approved
      // list (mid-arc, newest) and opens expanded so you type straight into it.
      setSnippets((prev) =>
        prev?.some((s) => s.id === created.id)
          ? prev
          : [created, ...(prev ?? [])],
      );
    });
  }

  // Delete owns the API call so the card can revert/show an error on failure (it
  // stays mounted, dimmed, until the reload removes it). Reconciling from the DB
  // rather than filtering locally means a concurrent background refresh can't
  // resurrect the just-deleted row. Rejecting a proposal reuses this exact path.
  async function handleDelete(id: number) {
    await deleteSnippet(id);
    loadSnippets();
  }

  // Approve/set-category own their API call, then reload from the DB so the row
  // re-renders in its correct sorted position (no in-place edit that would later
  // jump when the classify pass lands).
  async function handleApprove(id: number) {
    await approveSnippet(id);
    loadSnippets();
  }
  async function handleSetCategory(id: number, category: string) {
    await setSnippetCategory(id, category);
    loadSnippets();
  }

  // Copy owns its API call and returns nothing to the list: the source is untouched
  // (an independent duplicate is created in the target scope), so this scope's view
  // doesn't change. The target scope's open editor, if any, refreshes off the
  // backend's `snippets://changed` event.
  async function handleCopy(id: number, targetId: number | null) {
    await copySnippet(id, targetId);
  }

  // The scope's distinct categories (for the chip typeahead), derived from approved
  // snippets. Must run before the early returns.
  const categories = useMemo(() => {
    const set = new Set<string>();
    for (const s of snippets ?? []) {
      if (s.status === "approved" && s.category.trim()) set.add(s.category);
    }
    return [...set].sort((a, b) => a.localeCompare(b));
  }, [snippets]);

  // Where a snippet can be copied to: every scope except its own. From a pitch you
  // can copy to the profile or any other pitch; from the profile, to any pitch.
  const copyTargets = useMemo<CopyTarget[]>(() => {
    const candidates = pitches.filter((p) => p.id !== pitchId);
    // Pitch names aren't unique, so count them: a name shared by two pitches gets a
    // disambiguator (its skill, or `#id` as a last resort) so the destination is
    // never ambiguous.
    const nameCounts = new Map<string, number>();
    for (const p of candidates)
      nameCounts.set(p.name, (nameCounts.get(p.name) ?? 0) + 1);

    const targets: CopyTarget[] = [];
    if (pitchId !== null) targets.push({ id: null, name: "Profile" });
    for (const p of candidates) {
      const ambiguous = (nameCounts.get(p.name) ?? 0) > 1;
      const suffix = ambiguous
        ? p.skill.trim()
          ? ` · ${p.skill.trim()}`
          : ` · #${p.id}`
        : "";
      targets.push({ id: p.id, name: `${p.name}${suffix}` });
    }
    return targets;
  }, [pitches, pitchId]);

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

  // Proposed snippets are a triage queue — always on top. Approved snippets are the
  // flat, position-sorted library below (the backend does the ordering).
  const proposed = snippets.filter((s) => s.status === "proposed");
  const approved = snippets.filter((s) => s.status === "approved");

  return (
    <div className={styles.snippets}>
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

      {error && <div className={styles.error}>{error}</div>}

      {snippets.length === 0 ? (
        <p className={styles.empty}>
          No snippets yet. Add one to start building a library.
        </p>
      ) : (
        <ul className={styles.list}>
          {proposed.map((s) => (
            <li key={s.id}>
              <ProposedCard
                snippet={s}
                onApprove={handleApprove}
                onReject={handleDelete}
              />
            </li>
          ))}

          {approved.map((s) => (
            <li key={s.id}>
              <SnippetCard
                snippet={s}
                categories={categories}
                copyTargets={copyTargets}
                onDelete={handleDelete}
                onSetCategory={handleSetCategory}
                onCopy={handleCopy}
              />
            </li>
          ))}
        </ul>
      )}

      <p className={styles.hint}>
        Wrap a blank in [brackets] — like <span className={styles.hintTag}>[first name]</span> or{" "}
        <span className={styles.hintTag}>[what they mentioned]</span> — and the AI fills it from the
        prospect and the conversation when drafting.
      </p>
    </div>
  );
}

/** A short-lived status shown in a card header after a menu action. */
type Flash = { kind: "ok" | "err"; text: string };

/**
 * One approved snippet, as a collapsible. Collapsed, only the name shows (with its
 * category, if any); opening it reveals the editable name + content, each autosaved
 * after a short typing pause (via `useAutosave`). A blank (just-added) snippet opens
 * expanded so you type straight in. The header's context menu — right-click, or the
 * ⋯ button — copies the snippet elsewhere or deletes it. The unmount flush is
 * skipped mid-delete — the row is on its way out.
 */
function SnippetCard({
  snippet,
  categories,
  copyTargets,
  onDelete,
  onSetCategory,
  onCopy,
}: {
  snippet: Snippet;
  categories: string[];
  copyTargets: CopyTarget[];
  onDelete: (id: number) => Promise<void>;
  onSetCategory: (id: number, category: string) => Promise<void>;
  onCopy: (id: number, targetId: number | null) => Promise<void>;
}) {
  const [name, setName] = useState(snippet.name);
  const [content, setContent] = useState(snippet.content);
  // A blank, freshly-added card opens expanded and focuses its name field so you
  // type straight in; snippets that already have content start collapsed.
  const startExpanded = useRef(name.trim() === "" && content.trim() === "");
  const [expanded, setExpanded] = useState(startExpanded.current);
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [flash, setFlash] = useState<Flash | null>(null);
  const menuBtnRef = useRef<HTMLButtonElement>(null);
  const flashTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Synchronous re-entry guard (state updates are async); also tells the
  // autosave's unmount flush to stand down once a delete is underway.
  const deletingRef = useRef(false);

  const { showSaved, error, setError } = useAutosave({
    values: { name, content },
    persist: (v) => updateSnippet(snippet.id, v.name, v.content),
    canFlush: () => !deletingRef.current,
  });

  useEffect(
    () => () => {
      if (flashTimer.current) clearTimeout(flashTimer.current);
    },
    [],
  );

  function showFlash(next: Flash) {
    if (flashTimer.current) clearTimeout(flashTimer.current);
    setFlash(next);
    flashTimer.current = setTimeout(() => setFlash(null), 2400);
  }

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

  async function handleCopy(target: CopyTarget) {
    setMenuOpen(false);
    try {
      await onCopy(snippet.id, target.id);
      showFlash({ kind: "ok", text: `Copied to ${target.name}` });
    } catch (err) {
      showFlash({ kind: "err", text: errorMessage(err) });
    }
  }

  const bodyId = `snippet-body-${snippet.id}`;
  const title = name.trim() || "Untitled snippet";

  return (
    <div className={styles.card} data-deleting={deleting}>
      <div
        className={styles.cardHead}
        onContextMenu={(e) => {
          // Leave the name field its own native edit menu (paste, spellcheck).
          if ((e.target as HTMLElement).closest("input, textarea")) return;
          e.preventDefault();
          // Not while deleting, nor mid delete-confirm — the ⋯ button (the menu's
          // anchor) is swapped out for the Cancel/Delete buttons then, so the menu
          // would open stranded with no anchor to position against.
          if (!deleting && !confirmingDelete) setMenuOpen(true);
        }}
      >
        {/* The chevron is the disclosure control, rendered first in BOTH states so
            React updates it in place (rather than remounting) — keyboard focus on
            it survives an expand/collapse. */}
        <button
          type="button"
          className={styles.chevronBtn}
          onClick={() => setExpanded((v) => !v)}
          aria-expanded={expanded}
          aria-controls={bodyId}
          aria-label={`${expanded ? "Collapse" : "Expand"} ${title}`}
          data-expanded={expanded}
          disabled={deleting}
        >
          <Chevron />
        </button>
        {expanded ? (
          <input
            className={styles.nameInput}
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Snippet name"
            aria-label="Snippet name"
            disabled={deleting}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus={startExpanded.current}
          />
        ) : (
          // A mouse affordance for expanding by clicking the name; kept out of the
          // tab order (the chevron above is the keyboard control) but still read by
          // screen readers, so the name isn't hidden.
          <button
            type="button"
            className={styles.headToggle}
            onClick={() => setExpanded(true)}
            tabIndex={-1}
            disabled={deleting}
          >
            <span
              className={styles.titleText}
              data-untitled={name.trim() === "" || undefined}
            >
              {title}
            </span>
            {snippet.category.trim() && (
              <span
                className={styles.miniCat}
                data-manual={snippet.manual}
                title={`Category: ${snippet.category}`}
              >
                <span className={styles.categoryDot} aria-hidden="true" />
                {snippet.category}
              </span>
            )}
          </button>
        )}

        {flash && (
          <span className={styles.flash} data-kind={flash.kind} role="status">
            {flash.text}
          </span>
        )}

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
            ref={menuBtnRef}
            type="button"
            className={styles.menuBtn}
            onClick={() => setMenuOpen((o) => !o)}
            aria-label="Snippet actions"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            title="Actions"
            disabled={deleting}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <circle cx="5" cy="12" r="1.6" fill="currentColor" />
              <circle cx="12" cy="12" r="1.6" fill="currentColor" />
              <circle cx="19" cy="12" r="1.6" fill="currentColor" />
            </svg>
          </button>
        )}

        <Popover
          open={menuOpen}
          onClose={() => setMenuOpen(false)}
          anchorRef={menuBtnRef}
        >
          <div className={styles.menuLabel}>Copy to</div>
          {copyTargets.length === 0 ? (
            <div className={styles.menuEmpty}>No other pitches yet</div>
          ) : (
            copyTargets.map((t) => (
              <MenuItem
                key={t.id === null ? "profile" : t.id}
                label={t.name}
                leading={t.id === null ? <ProfileIcon /> : <PitchIcon />}
                onSelect={() => void handleCopy(t)}
              />
            ))
          )}
          <div className={styles.menuDivider} role="separator" />
          <MenuItem
            label="Delete"
            danger
            onSelect={() => {
              setMenuOpen(false);
              setConfirmingDelete(true);
            }}
          />
        </Popover>
      </div>

      {/* `inert` while collapsed removes the whole body — textarea, category chip,
          error — from the tab order and the a11y tree (the grid trick only clips
          paint, not focusability). */}
      <div className={styles.bodyWrap} data-expanded={expanded}>
        <div className={styles.bodyInner} inert={!expanded}>
          <textarea
            id={bodyId}
            className={styles.contentInput}
            value={content}
            onChange={(e) => setContent(e.target.value)}
            placeholder="What this snippet says… use [brackets] for blanks the AI fills in, like [first name]"
            aria-label="Snippet content"
            disabled={deleting}
          />

          {error && <div className={styles.cardError}>{error}</div>}

          <div className={styles.cardFoot}>
            <CategoryChip
              snippet={snippet}
              categories={categories}
              onSet={onSetCategory}
              disabled={deleting}
            />
            <SavedIndicator visible={showSaved} />
          </div>
        </div>
      </div>
    </div>
  );
}

function Chevron() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M9 6l6 6-6 6"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ProfileIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="8" r="3.4" stroke="currentColor" strokeWidth="1.7" />
      <path
        d="M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </svg>
  );
}

function PitchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M4 13V6a2 2 0 0 1 2-2h5l9 9-7 7-9-9z"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinejoin="round"
      />
      <circle cx="8.5" cy="8.5" r="1.3" fill="currentColor" />
    </svg>
  );
}

/**
 * The category chip on a snippet card — the manual override. Shows the current
 * category (AI-derived or hand-picked); clicking opens an inline field with a
 * typeahead over the scope's existing categories. Committing a value pins the
 * snippet (the AI won't re-categorize it); clearing it re-enables auto. A subtle
 * dot marks a manual (hand-picked) category vs. an AI-suggested one.
 */
function CategoryChip({
  snippet,
  categories,
  onSet,
  disabled,
}: {
  snippet: Snippet;
  categories: string[];
  onSet: (id: number, category: string) => Promise<void>;
  disabled: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(snippet.category);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const listId = `cats-${snippet.id}`;

  async function commit() {
    const next = value.trim();
    setEditing(false);
    if (next === snippet.category) return; // unchanged — no write
    setBusy(true);
    setError(null);
    try {
      await onSet(snippet.id, next);
      // Parent reloads; this card re-renders with the new value.
    } catch (err) {
      // Surface the failure (matching the delete/approve cards) instead of a
      // silent revert, so a rejected write isn't mistaken for a save.
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  if (editing) {
    return (
      <>
        <input
          className={styles.categoryInput}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onBlur={() => void commit()}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void commit();
            } else if (e.key === "Escape") {
              setValue(snippet.category);
              setEditing(false);
            }
          }}
          list={listId}
          placeholder="Category"
          aria-label="Snippet category"
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
        />
        <datalist id={listId}>
          {categories.map((c) => (
            <option key={c} value={c} />
          ))}
        </datalist>
      </>
    );
  }

  return (
    <>
      <button
        type="button"
        className={styles.categoryChip}
        data-empty={snippet.category.trim() === ""}
        data-manual={snippet.manual}
        onClick={() => {
          setError(null);
          setValue(snippet.category);
          setEditing(true);
        }}
        disabled={disabled || busy}
        title={
          snippet.manual
            ? "Category set by you — click to change"
            : "AI-suggested category — click to change"
        }
      >
        {snippet.category.trim() ? (
          <>
            <span className={styles.categoryDot} aria-hidden="true" />
            {snippet.category}
          </>
        ) : (
          "＋ Category"
        )}
      </button>
      {error && <span className={styles.chipError}>{error}</span>}
    </>
  );
}

/**
 * One AI-proposed snippet: read-only (the verbatim guarantee is the whole point),
 * shown in a distinct amber treatment with Approve / Reject. Approving flips it to
 * a normal snippet — the parent swaps in a `SnippetCard` at the same id, so the
 * user can then edit it freely. Both actions unmount this card on success, so like
 * the delete flow we reset state only on failure (a `finally` reset would fire on
 * an unmounted component).
 */
function ProposedCard({
  snippet,
  onApprove,
  onReject,
}: {
  snippet: Snippet;
  onApprove: (id: number) => Promise<void>;
  onReject: (id: number) => Promise<void>;
}) {
  const [busy, setBusy] = useState<null | "approve" | "reject">(null);
  const [error, setError] = useState<string | null>(null);
  // Synchronous re-entry guard (state updates are async).
  const busyRef = useRef(false);

  async function act(
    kind: "approve" | "reject",
    fn: (id: number) => Promise<void>,
  ) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(kind);
    setError(null);
    try {
      await fn(snippet.id);
      // Success unmounts this card — nothing to reset.
    } catch (err) {
      setError(errorMessage(err));
      busyRef.current = false;
      setBusy(null);
    }
  }

  const name = snippet.name.trim();

  return (
    <div className={styles.proposed} data-busy={busy !== null}>
      <div className={styles.proposedHead}>
        <span className={styles.proposedBadge}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3z"
              fill="currentColor"
            />
          </svg>
          Proposed
        </span>
        {name && <span className={styles.proposedName}>{name}</span>}
      </div>

      <p className={styles.proposedContent}>{snippet.content}</p>

      {error && <div className={styles.cardError}>{error}</div>}

      <div className={styles.proposedFoot}>
        <span className={styles.proposedHint}>Spotted in a message you sent</span>
        <div className={styles.proposedActions}>
          <button
            type="button"
            className={styles.rejectBtn}
            onClick={() => void act("reject", onReject)}
            disabled={busy !== null}
          >
            {busy === "reject" ? "Rejecting…" : "Reject"}
          </button>
          <button
            type="button"
            className={styles.approveBtn}
            onClick={() => void act("approve", onApprove)}
            disabled={busy !== null}
          >
            {busy === "approve" ? (
              "Approving…"
            ) : (
              <>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                  <path
                    d="M5 12.5l4.5 4.5L19 7"
                    stroke="currentColor"
                    strokeWidth="2.2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                Approve
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
