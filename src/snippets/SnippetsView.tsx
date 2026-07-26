import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  approveSnippet,
  createSnippet,
  deleteSnippet,
  listSnippets,
  onSnippetsChanged,
  reclassifySnippets,
  setSnippetCategory,
  updateSnippet,
  type Snippet,
} from "../api/snippets";
import { errorMessage } from "../lib/errors";
import { useAsyncAction } from "../lib/useAsyncAction";
import { useAutosave } from "../lib/useAutosave";
import {
  MenuDivider,
  MenuItem,
  MenuNote,
  Popover,
} from "../components/Popover";
import LoadError from "../components/LoadError";
import SavedIndicator from "../components/SavedIndicator";
import styles from "./SnippetsView.module.css";

/** One conversation-stage section: the category label and the snippets in it. */
interface StageGroup {
  category: string;
  items: Snippet[];
}

/** Group approved snippets into conversation-stage sections, ordered along the arc.
 *  Each snippet's `category` is its stage; `position` orders the sections (a group
 *  sits at its earliest snippet's position, so Opener-type stages float to the top
 *  and Close-type ones sink) and the snippets within them. Uncategorized snippets
 *  (blank category) always land in a final catch-all section. The incoming list is
 *  already position-sorted by the backend, so per-group order is preserved as-is. */
function groupByStage(approved: Snippet[]): StageGroup[] {
  const map = new Map<string, Snippet[]>();
  for (const s of approved) {
    const key = s.category.trim();
    const bucket = map.get(key);
    if (bucket) bucket.push(s);
    else map.set(key, [s]);
  }
  return [...map.entries()]
    .map(([category, items]) => ({
      category,
      items,
      minPos: Math.min(...items.map((s) => s.position)),
    }))
    .sort((a, b) => {
      // Uncategorized sinks below every named stage; otherwise order along the arc.
      if (a.category === "") return 1;
      if (b.category === "") return -1;
      return a.minPos - b.minPos;
    })
    .map(({ category, items }) => ({ category, items }));
}

/**
 * The Snippets tab — the one library every draft composes from, shared by every
 * customer profile. Proposed (AI-suggested) snippets sit on top as an amber
 * triage queue; approved snippets below are grouped into conversation-stage
 * sections you open on demand. Each row's context menu (right-click, or the ⋯
 * button) deletes it.
 *
 * Sections group by *when in a thread* a line belongs, never by whom it's for:
 * which snippets suit a given buyer is decided per draft, from that customer
 * profile's pain and goal.
 *
 * The library used to live inside a pitch's settings and again under Profile,
 * split into scopes. With a single product there's a single body of material, so
 * it gets a page of its own — and the page is this component rather than a
 * wrapper around it, since there was never a second consumer to justify a split.
 */
export default function SnippetsView() {
  const [snippets, setSnippets] = useState<Snippet[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);
  // The "re-score & re-categorize everything" action: a confirm gate plus its own
  // async-action (busy flag + caught error + re-entry guard); the component stays
  // mounted across the batch, so `useAsyncAction`'s finally-reset contract fits.
  const [confirmingReclassify, setConfirmingReclassify] = useState(false);
  // Which stage sections are OPEN (by category label). Sections are closed by
  // default (a stage absent from the set is collapsed), so the library reads as a
  // tidy list of stage headers you open on demand. Held here, not per section, so a
  // card stays a sibling in one flat list keyed by id — a background re-stage moves
  // it in place rather than remounting it (which would drop focus and in-flight
  // edits).
  const [openStages, setOpenStages] = useState<ReadonlySet<string>>(new Set());
  // The card the user is actively working in (just added, expanded, or editing). Its
  // stage section is force-shown even when collapsed, so a background re-stage that
  // moves the card into a closed section can't hide it (and blur it) mid-edit. Cleared
  // when the user manually toggles a section (they've taken control of what's open).
  const [activeSnippetId, setActiveSnippetId] = useState<number | null>(null);
  // Outcome line for a finished re-score ("Re-scored N snippets") — the batch returns a
  // count that would otherwise be discarded. Cleared when a new re-score starts.
  const [reclassifyNote, setReclassifyNote] = useState<string | null>(null);
  // Set true when a re-score finishes so the next loaded list opens every stage section.
  // Re-scoring re-homes cards across sections, which are collapsed by default, so without
  // this the freshly organized library would look empty.
  const [expandAllOnNextLoad, setExpandAllOnNextLoad] = useState(false);
  // Stable base for the per-section header ids that link each card to its stage (a11y).
  const sectionIdBase = useId();
  const {
    busy: reclassifying,
    error: reclassifyError,
    run: runReclassify,
  } = useAsyncAction();
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
  const loadSnippets = useCallback((loud = false) => {
    const seq = ++fetchSeq.current;
    if (loud) setLoadError(null);
    listSnippets()
      .then((s) => {
        if (mountedRef.current && seq === fetchSeq.current) setSnippets(s);
      })
      .catch((e) => {
        if (mountedRef.current && seq === fetchSeq.current && loud)
          setLoadError(errorMessage(e));
      });
  }, []);

  // Initial load (and retry via `reloadKey`).
  useEffect(() => {
    loadSnippets(true);
  }, [loadSnippets, reloadKey]);

  // Live-refresh when a background pass changes the library — a new proposal, or a
  // classify pass updating a snippet's position/category. The reload goes through
  // `loadSnippets`, so it can't clobber (or be clobbered by) a concurrent user
  // action.
  useEffect(() => {
    let active = true;
    const unlisten = onSnippetsChanged(() => {
      if (active) loadSnippets();
    });
    return () => {
      active = false;
      // Tauri's `listen` resolves once registration completes (it effectively
      // never rejects), but catch anyway so teardown can't leak an unhandled
      // rejection.
      void unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [loadSnippets]);

  // After a re-score lands its reorganized list, open every stage section so nothing
  // hides. Cards re-home across sections during a re-score and sections are collapsed by
  // default, so otherwise the reorganized library would read as empty. One-shot, cleared
  // once applied so ordinary background refreshes don't force sections open.
  useEffect(() => {
    if (!expandAllOnNextLoad || !snippets) return;
    const cats = new Set<string>();
    for (const s of snippets) {
      if (s.status === "approved") cats.add(s.category.trim());
    }
    setOpenStages(cats);
    setExpandAllOnNextLoad(false);
  }, [expandAllOnNextLoad, snippets]);

  function handleAdd() {
    run(async () => {
      const created = await createSnippet();
      // Mark it active so its section stays shown even after the classify pass
      // re-homes it out of Uncategorized — otherwise, with sections closed by
      // default, the card you're meant to type into would vanish mid-edit.
      setActiveSnippetId(created.id);
      // Guard against a live-refresh (`snippets://changed`) having already folded
      // this row in — the created snippet is committed before this resolves, so a
      // concurrent refetch can beat us here; dropping the duplicate avoids a double
      // card / duplicate React key. A blank card (empty category) lands in the
      // Uncategorized section and opens expanded + autofocused, so `autoFocus`
      // scrolls it into view to type straight into.
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

  // Re-score + re-categorize the whole library. Resolves once the batch finishes (the
  // backend emits `snippets://changed` once at the end for any OTHER open editor; this
  // window reloads itself here). The reload runs in a `finally` so a batch that applied
  // some rows and then errored still shows the DB's real state, not a stale list — and
  // sets `expandAllOnNextLoad` so the reorganized, re-homed cards don't hide in the
  // collapsed sections they moved into.
  function handleReclassify() {
    setConfirmingReclassify(false);
    setReclassifyNote(null);
    runReclassify(async () => {
      try {
        const changed = await reclassifySnippets();
        setReclassifyNote(
          changed > 0
            ? `Re-scored ${changed} snippet${changed === 1 ? "" : "s"}.`
            : "Everything was already up to date.",
        );
      } finally {
        setExpandAllOnNextLoad(true);
        loadSnippets();
      }
    });
  }

  // Set a stage section's open state explicitly (the caller passes the desired
  // state from what's currently on screen). Clearing the active-card override first
  // means one click always matches the visible state — even for a section that was
  // open only because it held the active card.
  function setStageOpen(category: string, open: boolean) {
    setActiveSnippetId(null);
    setOpenStages((prev) => {
      const next = new Set(prev);
      if (open) next.add(category);
      else next.delete(category);
      return next;
    });
  }

  // The library's distinct categories (for the chip typeahead), derived from
  // approved snippets. Must run before the early returns.
  const categories = useMemo(() => {
    const set = new Set<string>();
    for (const s of snippets ?? []) {
      if (s.status === "approved" && s.category.trim()) set.add(s.category);
    }
    return [...set].sort((a, b) => a.localeCompare(b));
  }, [snippets]);

  if (loadError) {
    return (
      <Page>
        <LoadError
          what="snippets"
          detail={loadError}
          onRetry={() => setReloadKey((k) => k + 1)}
        />
      </Page>
    );
  }
  if (!snippets) {
    // Local SQLite resolves near-instantly; reserve space so the layout doesn't
    // jump when it lands.
    return (
      <Page>
        <div className={styles.loading} aria-busy="true" aria-hidden="true" />
      </Page>
    );
  }

  // Proposed snippets are a triage queue — always on top. Approved snippets are the
  // flat, position-sorted library below (the backend does the ordering).
  const proposed = snippets.filter((s) => s.status === "proposed");
  const approved = snippets.filter((s) => s.status === "approved");

  return (
    <Page>
      <div className={styles.snippets}>
        <button
          type="button"
          className={styles.addBtn}
          onClick={handleAdd}
          disabled={adding}
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
              strokeWidth="2"
              strokeLinecap="round"
            />
          </svg>
          Add snippet
        </button>

        {approved.length >= 2 && (
          <div className={styles.toolbar}>
            {confirmingReclassify ? (
              <div className={styles.rescoreConfirm}>
                <span className={styles.rescoreWarn}>
                  Re-score all {approved.length}? This overwrites categories you
                  set by hand.
                </span>
                <button
                  type="button"
                  className={styles.secondaryBtn}
                  onClick={() => setConfirmingReclassify(false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className={styles.rescoreGoBtn}
                  onClick={handleReclassify}
                >
                  Re-score
                </button>
              </div>
            ) : (
              <button
                type="button"
                className={styles.rescoreBtn}
                onClick={() => setConfirmingReclassify(true)}
                disabled={reclassifying || adding}
              >
                {reclassifying ? (
                  <>
                    <span className={styles.spinner} aria-hidden="true" />
                    Re-scoring…
                  </>
                ) : (
                  <>
                    <SparkIcon />
                    Re-score &amp; re-categorize
                  </>
                )}
              </button>
            )}
          </div>
        )}

        {error && <div className={styles.error}>{error}</div>}
        {reclassifyError && (
          <div className={styles.error}>{reclassifyError}</div>
        )}
        {reclassifyNote && !reclassifying && (
          <div className={styles.rescoreNote} role="status">
            {reclassifyNote}
          </div>
        )}

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

            {groupByStage(approved).flatMap((group, i) => {
              // Open if the user opened it, or if it holds the card being worked in —
              // the latter keeps an active card visible through a background re-stage,
              // in the same render, so it never flashes hidden or loses focus.
              const collapsed =
                !openStages.has(group.category) &&
                !group.items.some((s) => s.id === activeSnippetId);
              const uncategorized = group.category === "";
              // Ties each card back to its stage header for screen readers: the header
              // and cards are flat siblings (so a re-stage moves a card without a remount),
              // so the grouping is only visual unless the cards point at the header.
              const headerId = `${sectionIdBase}-sec-${i}`;
              return [
                <li
                  // `cat:` namespaces real stages so the empty-category sentinel can't
                  // collide with a stage a user literally named "uncategorized".
                  key={
                    uncategorized
                      ? "stage-uncategorized"
                      : `cat:${group.category}`
                  }
                  className={styles.sectionRow}
                >
                  <button
                    type="button"
                    id={headerId}
                    className={styles.sectionToggle}
                    onClick={() => setStageOpen(group.category, collapsed)}
                    aria-expanded={!collapsed}
                  >
                    <span
                      className={styles.sectionChevron}
                      data-expanded={!collapsed}
                    >
                      <Chevron />
                    </span>
                    <span
                      className={styles.sectionName}
                      data-uncat={uncategorized || undefined}
                    >
                      {uncategorized ? "Uncategorized" : group.category}
                    </span>
                    <span className={styles.sectionCount}>
                      {group.items.length}
                    </span>
                  </button>
                </li>,
                // Cards stay siblings in this one <ul>, keyed by id — so a background
                // re-stage moves a card between sections in place instead of remounting
                // it. Collapsing hides the run via `hidden` (no unmount, no lost edits).
                // `role=group` + `aria-labelledby` restore the stage association a screen
                // reader would otherwise lose (the collapsed card carries no stage text).
                ...group.items.map((s) => (
                  <li
                    key={s.id}
                    hidden={collapsed}
                    role="group"
                    aria-labelledby={headerId}
                  >
                    <SnippetCard
                      snippet={s}
                      categories={categories}
                      onDelete={handleDelete}
                      onSetCategory={handleSetCategory}
                      onActivity={setActiveSnippetId}
                    />
                  </li>
                )),
              ];
            })}
          </ul>
        )}

        <p className={styles.hint}>
          Wrap a blank in [brackets] — like{" "}
          <span className={styles.hintTag}>[first name]</span> or{" "}
          <span className={styles.hintTag}>[what they mentioned]</span> — and
          the AI fills it from the prospect and the conversation when drafting.
        </p>
      </div>
    </Page>
  );
}

/** The tab's page shell: title, standfirst, and whatever state the library is in
 *  below. Wraps the loading and error states too, so the heading doesn't pop in
 *  after the list resolves. */
function Page({ children }: { children: React.ReactNode }) {
  return (
    <div className={styles.page}>
      <header className={styles.intro}>
        <h1 className={styles.title}>Snippets</h1>
        <p className={styles.subtitle}>
          Everything you're willing to say, in your own words. A draft is
          stitched from these and nothing else — the AI picks the ones that fit
          whoever you're writing to and where the thread has got to.
        </p>
      </header>
      {children}
    </div>
  );
}

/**
 * One approved snippet, as a collapsible. Collapsed, only the name shows (with its
 * category, if any); opening it reveals the editable name + content, each autosaved
 * after a short typing pause (via `useAutosave`). A blank (just-added) snippet opens
 * expanded so you type straight in. The header's context menu — right-click, or the
 * ⋯ button — deletes it. The unmount flush is skipped mid-delete — the row is on its
 * way out.
 */
function SnippetCard({
  snippet,
  categories,
  onDelete,
  onSetCategory,
  onActivity,
}: {
  snippet: Snippet;
  categories: string[];
  onDelete: (id: number) => Promise<void>;
  onSetCategory: (id: number, category: string) => Promise<void>;
  /** Mark this card as the one being worked in (expanded / edited), so its stage
   *  section stays open through a background re-stage. */
  onActivity: (id: number) => void;
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
  const menuBtnRef = useRef<HTMLButtonElement>(null);
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
          onClick={() => {
            setExpanded((v) => !v);
            onActivity(snippet.id);
          }}
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
            onChange={(e) => {
              setName(e.target.value);
              onActivity(snippet.id);
            }}
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
            onClick={() => {
              setExpanded(true);
              onActivity(snippet.id);
            }}
            tabIndex={-1}
            disabled={deleting}
          >
            <span
              className={styles.titleText}
              data-untitled={name.trim() === "" || undefined}
            >
              {title}
            </span>
          </button>
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
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              aria-hidden="true"
            >
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
            onChange={(e) => {
              setContent(e.target.value);
              onActivity(snippet.id);
            }}
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
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
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

function SparkIcon() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6L12 3z"
        fill="currentColor"
      />
      <path
        d="M18.5 14.5l.7 1.9 1.9.7-1.9.7-.7 1.9-.7-1.9-1.9-.7 1.9-.7.7-1.9z"
        fill="currentColor"
      />
    </svg>
  );
}

/**
 * The category (conversation-stage) editor on a snippet card. Shows the current stage
 * (AI-derived or hand-picked); clicking opens a combobox: type a NEW stage, or pick
 * an EXISTING one to move the snippet to another section. Setting a stage pins the
 * snippet (the per-edit AI pass won't re-categorize it); "Clear" re-enables auto. A
 * subtle dot marks a manual (hand-picked) stage vs. an AI-suggested one.
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
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);

  async function apply(next: string) {
    const v = next.trim();
    setOpen(false);
    if (v === snippet.category) return; // unchanged — no write
    setBusy(true);
    setError(null);
    try {
      await onSet(snippet.id, v);
      // Parent reloads; the snippet re-renders (and re-homes to its new section).
    } catch (err) {
      // Surface the failure (matching the delete/approve cards) instead of a silent
      // revert, so a rejected write isn't mistaken for a save.
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  // Existing stages you can move to — the current one is excluded (it's a no-op).
  const others = categories.filter((c) => c !== snippet.category);

  return (
    <>
      <button
        ref={chipRef}
        type="button"
        className={styles.categoryChip}
        data-empty={snippet.category.trim() === ""}
        data-manual={snippet.manual}
        onClick={() => {
          setError(null);
          setValue("");
          setOpen((o) => !o);
        }}
        disabled={disabled || busy}
        aria-haspopup="menu"
        aria-expanded={open}
        title={
          snippet.manual
            ? "Stage set by you — click to change"
            : "AI-suggested stage — click to change"
        }
      >
        {snippet.category.trim() ? (
          <>
            <span className={styles.categoryDot} aria-hidden="true" />
            {snippet.category}
          </>
        ) : (
          "＋ Stage"
        )}
      </button>

      <Popover open={open} onClose={() => setOpen(false)} anchorRef={chipRef}>
        <div className={styles.comboField}>
          <input
            className={styles.comboInput}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (value.trim()) void apply(value);
              } else if (e.key === "Escape") {
                setOpen(false);
              }
            }}
            placeholder="New stage…"
            aria-label="New stage"
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          />
        </div>
        {others.length > 0 && <MenuNote heading>Move to</MenuNote>}
        {others.map((c) => (
          <MenuItem key={c} label={c} onSelect={() => void apply(c)} />
        ))}
        {snippet.category.trim() && (
          <>
            <MenuDivider />
            <MenuItem
              label="Clear stage"
              danger
              onSelect={() => void apply("")}
            />
          </>
        )}
      </Popover>

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
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            aria-hidden="true"
          >
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
        <span className={styles.proposedHint}>
          Spotted in a message you sent
        </span>
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
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  aria-hidden="true"
                >
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
