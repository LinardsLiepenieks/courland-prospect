import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  approveSnippet,
  createSnippet,
  deleteSnippet,
  listSnippets,
  onSnippetsChanged,
  setSnippetCategory,
  updateSnippet,
  type RedundancyGroup,
  type Snippet,
} from "../api/snippets";
import * as organize from "./organizeRun";
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
import { hasBlank, splitOnBlanks } from "./blanks";
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
  // The confirm gate for "organize library". Only the gate is component state — the run
  // itself, its outcome and its report all live in `organizeRun`, because this view is
  // unmounted on a tab switch and the run outlives that. See below.
  const [confirmingOrganize, setConfirmingOrganize] = useState(false);
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
  // The organize run — phase, outcome note, redundancy report and error — lives in a
  // module-level store, NOT in this component. A tab switch unmounts this view (App
  // renders it conditionally), and holding a 2×60s run's state here meant the finished
  // report was discarded and the re-entry guard reset. See `organizeRun`.
  const {
    phase,
    note: organizeNote,
    groups,
    selections,
    error: organizeError,
    libraryVersion,
    restagedAt,
  } = useSyncExternalStore(organize.subscribe, organize.getSnapshot);
  const organizing = phase !== null;
  // The last re-stage this mount has expanded sections for. A version rather than a
  // one-shot boolean, because the boolean lived here and a tab switch mid-run threw it
  // away: the run finished, the reorganized library loaded into a view whose sections were
  // all collapsed, and the result read as "it did nothing".
  const expandedForRef = useRef(0);
  // The Organize trigger and its confirm, so focus can follow the swap between them in
  // both directions. Opening the gate replaces the <button> with a <div>, which unmounts
  // the focused node — a two-step gate that can't be completed from the keyboard isn't a
  // gate, it's a dead end.
  const organizeBtnRef = useRef<HTMLButtonElement>(null);
  const organizeGoRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirmingOrganize) organizeGoRef.current?.focus();
  }, [confirmingOrganize]);
  // Stable base for the per-section header ids that link each card to its stage (a11y).
  const sectionIdBase = useId();
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

  // Reload whenever the organize run says the library moved. This also covers a run that
  // progressed while this view was unmounted (a tab switch): the version it left behind
  // differs from the one this mount last saw, so remounting reloads once rather than
  // showing the pre-run list. The initial load above already covers version 0.
  useEffect(() => {
    if (libraryVersion > 0) loadSnippets();
  }, [libraryVersion, loadSnippets]);

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
  // default, so otherwise the reorganized library would read as empty.
  //
  // Keyed on the store's `restagedAt` rather than a local one-shot flag, so this works on a
  // mount that arrived AFTER the re-score finished — the tab-switch case, where the flag
  // version silently did nothing. Applied at most once per re-stage, so ordinary background
  // refreshes don't keep forcing sections open.
  useEffect(() => {
    if (restagedAt === 0 || restagedAt === expandedForRef.current || !snippets) return;
    expandedForRef.current = restagedAt;
    const cats = new Set<string>();
    for (const s of snippets) {
      if (s.status === "approved") cats.add(s.category.trim());
    }
    setOpenStages(cats);
  }, [restagedAt, snippets]);

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
      // The list is already updated in place above; this only retires an outcome line
      // that no longer describes the library.
      organize.noteLibraryChanged();
    });
  }

  // Delete owns the API call so the card can revert/show an error on failure (it
  // stays mounted, dimmed, until the reload removes it). Reconciling from the DB
  // rather than filtering locally means a concurrent background refresh can't
  // resurrect the just-deleted row. Rejecting a proposal reuses this exact path.
  // `noteLibraryChanged` rather than a bare `loadSnippets()`: it bumps the store's
  // `libraryVersion` (which this view reloads on) AND clears a stale outcome line, since a
  // note describing a library that has since changed is no longer true. That was what the
  // store's `libraryVersion` doc always claimed the mechanism was for; the out-of-run half
  // had just never been wired up, so these called `loadSnippets` directly.
  async function handleDelete(id: number) {
    await deleteSnippet(id);
    organize.noteLibraryChanged();
  }

  // Approve/set-category own their API call, then reload from the DB so the row
  // re-renders in its correct sorted position (no in-place edit that would later
  // jump when the classify pass lands).
  async function handleApprove(id: number) {
    await approveSnippet(id);
    organize.noteLibraryChanged();
  }
  async function handleSetCategory(id: number, category: string) {
    await setSnippetCategory(id, category);
    organize.noteLibraryChanged();
  }

  // Organize the library: re-score + re-categorize every snippet, then search the result
  // for redundancy. Everything about the run — sequencing, the outcome line, the report,
  // the re-entry guard, and the flag that expands the re-homed sections — lives in
  // `organizeRun`, because this view is unmounted on a tab switch and the run takes two
  // passes of up to 60s each. All this does is close the gate and start it.
  function handleOrganize() {
    setConfirmingOrganize(false);
    void organize.start();
  }

  // Closing the gate puts focus back where it came from. Without this, dismissing a
  // confirmation drops focus to <body> exactly as opening it did — so a keyboard user is
  // penalised for changing their mind.
  function cancelOrganize() {
    setConfirmingOrganize(false);
    organizeBtnRef.current?.focus();
  }

  // Delete the snippets picked out of one group, then retire the group.
  //
  // Sequential rather than `Promise.all`: every delete takes the same single SQLite
  // connection lock, so racing them only adds contention. But each one is caught
  // individually — aborting the loop on the first rejection used to leave the REST of
  // the user's picks alive, which is the opposite of what they asked for. The likeliest
  // rejection is "Snippet not found" over a row something else already deleted, i.e. a
  // row that's in the desired state anyway.
  async function handleGroupDelete(keepId: number, ids: number[]) {
    const failures: string[] = [];
    for (const id of ids) {
      try {
        await deleteSnippet(id);
      } catch (err) {
        failures.push(errorMessage(err));
      }
    }
    loadSnippets();
    if (failures.length > 0) throw new Error(failures[0]);
    organize.resolveGroup(keepId);
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

  // Snippets by id, so the redundancy panel can render rows from the ids its groups
  // carry — and notice the ones that have since disappeared. Must run before the early
  // returns.
  const byId = useMemo(
    () => new Map((snippets ?? []).map((s) => [s.id, s])),
    [snippets],
  );

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
  // What the confirm gate should actually count. Both backend passes skip blank-content
  // rows, so counting every approved snippet made the gate promise a number the outcome
  // note then contradicted whenever an unfinished blank card was open.
  const organizable = approved.filter((s) => s.content.trim() !== "");

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
            {confirmingOrganize ? (
              <div className={styles.organizeConfirm}>
                {/* `role="alert"` because opening this gate replaces the button that had
                    focus, so without it a screen-reader user gets no indication that a
                    question appeared at all — only that their button vanished. It states
                    the count and the overwrite, which are the two things the gate exists
                    to say. */}
                <span className={styles.organizeWarn} role="alert">
                  Organize all {organizable.length}? Re-scores every snippet,
                  re-groups them by stage, re-tags what each is about, and flags
                  redundant ones. This overwrites stages you set by hand.
                </span>
                <button
                  type="button"
                  className={styles.secondaryBtn}
                  onClick={cancelOrganize}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className={styles.organizeGoBtn}
                  onClick={handleOrganize}
                  // Focus moves here when the gate opens. The <button> that was focused is
                  // replaced by this <div>, so focus would otherwise fall to <body> and a
                  // keyboard user would have to Tab from the top of the document — through
                  // the tab bar — to reach a confirm they just asked for.
                  ref={organizeGoRef}
                >
                  Organize
                </button>
              </div>
            ) : (
              <button
                type="button"
                className={styles.organizeBtn}
                onClick={() => setConfirmingOrganize(true)}
                disabled={organizing || adding}
                ref={organizeBtnRef}
              >
                {organizing ? (
                  <>
                    <span className={styles.spinner} aria-hidden="true" />
                    {/* Two passes over the whole library can take a while; naming
                        the current one is the difference between a wait that's
                        explained and one that reads as a hang. */}
                    {phase === "checking"
                      ? "Finding duplicates…"
                      : "Re-scoring…"}
                  </>
                ) : (
                  <>
                    <SparkIcon />
                    Organize library
                  </>
                )}
              </button>
            )}
          </div>
        )}

        {error && <div className={styles.error}>{error}</div>}
        {/* Both of these are module state, so they outlive this view — without a way to
            dismiss them, a failure at 10am greeted every later visit to the tab for the
            rest of the session, attached to nothing the user had just done. */}
        {organizeError && (
          <div className={styles.error}>
            {organizeError}
            <button
              type="button"
              className={styles.noteDismissBtn}
              onClick={organize.dismissNote}
              aria-label="Dismiss this message"
            >
              ✕
            </button>
          </div>
        )}
        {organizeNote && !organizing && (
          <div className={styles.organizeNote} role="status">
            {organizeNote}
            <button
              type="button"
              className={styles.noteDismissBtn}
              onClick={organize.dismissNote}
              aria-label="Dismiss this message"
            >
              ✕
            </button>
          </div>
        )}

        {groups && !organizing && (
          <RedundancyPanel
            groups={groups}
            byId={byId}
            selections={selections}
            onToggle={organize.setSelection}
            onDelete={handleGroupDelete}
            onDismissGroup={organize.resolveGroup}
            onDismissAll={organize.dismissGroups}
          />
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
          Each snippet gets a <strong>stage</strong> (when in a thread it fits —
          you can change this) and a <strong>topic</strong> (what it's about —
          the AI's read). A draft prefers lines on the topic the conversation is
          already on, and changes subject only when the thread gives it a reason.
        </p>

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
 * The redundancy report from the last organize: groups of snippets the AI judged to say
 * the same thing, each collapsible down to the version worth keeping.
 *
 * Ephemeral and advisory. Nothing here is marked in the database and nothing was deleted
 * to build it — the backend only reports groups, and every deletion is the user ticking
 * rows and the ordinary `delete_snippet` running. That asymmetry with the re-score half
 * of the same click is deliberate: a wrong stage label is a shrug, a wrong deletion is
 * lost writing.
 *
 * The report is a snapshot, and the library keeps moving under it — a background
 * `snippets://changed`, a delete from a card below, another window. So groups are
 * rendered from whatever ids they still resolve to, and one left pointing at fewer than
 * two live snippets isn't a duplicate pair any more and drops out silently.
 */
function RedundancyPanel({
  groups,
  byId,
  selections,
  onToggle,
  onDelete,
  onDismissGroup,
  onDismissAll,
}: {
  groups: RedundancyGroup[];
  byId: Map<number, Snippet>;
  /** Ticks per group, keyed on `keep_id`. Absent = the group hasn't been touched yet, so
   *  it shows the model's suggestion (see `defaultTicks`). */
  selections: Record<number, number[]>;
  onToggle: (keepId: number, ids: number[]) => void;
  onDelete: (keepId: number, ids: number[]) => Promise<void>;
  onDismissGroup: (keepId: number) => void;
  onDismissAll: () => void;
}) {
  // Resolve each member to a live row, and keep it ONLY if that row still holds the text
  // the model judged. An existence check is not enough: `snippets.id` is a plain SQLite
  // rowid alias, so deleting the highest-id snippet hands its id to the next insert —
  // which can be a background propose pass, with no action from the user at all. Without
  // the content compare, a group could come back to life pointing at an unrelated new
  // snippet and pre-tick a real one for deletion on a false premise. The same compare
  // also drops a member that was simply edited since the report was made.
  const live = groups
    .map((group) => ({
      group,
      items: group.members
        .map((m) => {
          const snippet = byId.get(m.id);
          return snippet && snippet.content.trim() === m.analyzed.trim()
            ? snippet
            : undefined;
        })
        .filter((s): s is Snippet => s !== undefined),
    }))
    .filter(({ items }) => items.length >= 2);

  // Resolve each group's ticks once per store change, so the array identity handed to a card
  // is stable. `defaultTicks` builds a fresh array, and passing that straight down would
  // change identity on every render — churning the card's derived Set and re-running its
  // reprieve effect each time.
  const ticks = useMemo(() => {
    const byGroup = new Map<number, readonly number[]>();
    for (const g of groups) {
      byGroup.set(g.keep_id, selections[g.keep_id] ?? organize.defaultTicks(g));
    }
    return byGroup;
  }, [groups, selections]);

  if (live.length === 0) return null;

  return (
    <section className={styles.dupePanel} aria-label="Possible redundancy">
      <div className={styles.dupePanelHead}>
        <h2 className={styles.dupePanelTitle}>
          {live.length === 1
            ? "1 group says the same thing twice"
            : `${live.length} groups say the same thing twice`}
        </h2>
        <button
          type="button"
          className={styles.dupeDismissBtn}
          onClick={onDismissAll}
        >
          Dismiss
        </button>
      </div>

      <ul className={styles.dupeGroups}>
        {live.map(({ group, items }, i) => (
          <li key={group.keep_id}>
            <RedundancyGroupCard
              group={group}
              items={items}
              index={i}
              selected={ticks.get(group.keep_id) ?? []}
              onToggle={onToggle}
              onDelete={onDelete}
              onDismiss={() => onDismissGroup(group.keep_id)}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * One redundant group: the shared point it makes, and a tickable row per snippet in it.
 *
 * A tick means "delete this one", and everything except the AI's suggested keeper starts
 * ticked — so the default reading of a group is already the action you want ("collapse
 * this to its best line") and the common case is a single click. The suggestion is only
 * a starting point: retick freely.
 *
 * The one thing the UI won't let you do is empty a group. Deleting every version of a
 * point isn't collapsing redundancy, it's losing the point — so whenever a single
 * snippet is left unticked, its checkbox locks rather than warning about it afterwards.
 */
function RedundancyGroupCard({
  group,
  items,
  index,
  selected: selectedIds,
  onToggle,
  onDelete,
  onDismiss,
}: {
  group: RedundancyGroup;
  items: Snippet[];
  /** Position in the panel, for the entrance stagger. */
  index: number;
  /** Ticked-for-deletion ids, owned by `organizeRun` rather than by this card — a tab
   *  switch remounts it, and holding the user's choices here silently reverted them to the
   *  pre-ticked default. */
  selected: readonly number[];
  onToggle: (keepId: number, ids: number[]) => void;
  onDelete: (keepId: number, ids: number[]) => Promise<void>;
  onDismiss: () => void;
}) {
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Two-step gate on the delete, matching every other destructive control here.
  const [confirming, setConfirming] = useState(false);
  // Ties the group's reason to each checkbox for screen readers — the rows are the
  // group's only visible structure, so without this the shared point isn't announced.
  const reasonId = useId();
  // Synchronous re-entry guard (state updates are async).
  const busyRef = useRef(false);
  // Focus follows the confirm swap in both directions — see the `key`s on the footer.
  const deleteTriggerRef = useRef<HTMLButtonElement>(null);
  const confirmDeleteRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirming) confirmDeleteRef.current?.focus();
  }, [confirming]);

  function cancelConfirm() {
    setConfirming(false);
    deleteTriggerRef.current?.focus();
  }

  // `selected` is the user's raw ticks. `doomed` is what will actually be deleted, and
  // it's what the whole card renders from — so a row shown ticked is always exactly a
  // row the Delete button removes.
  //
  // Both derive from `items` rather than from `selected` alone, because a snippet
  // deleted elsewhere leaves its id behind in `selected`: sending that on would fail
  // with "Snippet not found" over a row that's already gone.
  const ticked = items.filter((s) => selected.has(s.id));

  // A group must always keep one row, and the checkbox lock alone can't guarantee that.
  // The lock only ever guarded the "one row left unticked" case, but `items` shrinks
  // when a snippet is deleted from its own card below (or in another window) while this
  // panel is open — so if the row that was holding the group open is the one that
  // vanished, the count goes straight from one unticked to zero, never passing through
  // the case the lock watches, and every remaining row is ticked.
  //
  // So the survivor is guaranteed here instead: with nothing left unticked, the last row
  // standing is reprieved — it un-ticks itself and locks. `doomed.length < items.length`
  // then holds unconditionally, which is the actual invariant, rather than something the
  // UI merely tries not to violate.
  const reprieved =
    items.length > 0 && ticked.length === items.length
      ? (items.find((s) => s.id === group.keep_id) ?? items[0])
      : null;
  const doomed = ticked.filter((s) => s.id !== reprieved?.id);
  const kept = items.filter((s) => !doomed.some((d) => d.id === s.id));
  const lockedId = kept.length === 1 ? kept[0].id : null;

  // Commit the reprieve to `selected`, rather than leaving it a mask over a tick that's
  // still set underneath. Both halves have a job: deriving it above is what makes the
  // "one row always survives" invariant hold on the very first render, and clearing the
  // tick here is what stops the reprieve from being revocable — otherwise the protection
  // lasts only while every row is ticked, and unticking some *other* row would drop that
  // condition and silently re-arm the row the UI had just marked "Kept".
  //
  // Keyed on the id (a primitive), not the object, so this can't re-fire on identity
  // churn: removing the tick makes `reprieved` null on the next render, and the guard
  // returns `prev` unchanged once there's nothing left to clear.
  const reprievedId = reprieved?.id;
  useEffect(() => {
    if (reprievedId === undefined || !selected.has(reprievedId)) return;
    onToggle(
      group.keep_id,
      [...selected].filter((id) => id !== reprievedId),
    );
  }, [reprievedId, selected, onToggle, group.keep_id]);

  function toggle(id: number) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onToggle(group.keep_id, [...next]);
  }

  async function handleDelete() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await onDelete(
        group.keep_id,
        doomed.map((s) => s.id),
      );
      // Success retires the group, unmounting this card — no state to reset.
    } catch (err) {
      setError(errorMessage(err));
      busyRef.current = false;
      setBusy(false);
    }
  }

  return (
    <div
      className={styles.dupeGroup}
      data-busy={busy}
      // Stagger the entrance, capped so a long report doesn't crawl in.
      style={{ animationDelay: `${Math.min(index * 40, 200)}ms` }}
    >
      {/* A group with no reason gets a line anyway. The model is allowed to return an empty
          one, and omitting the element gave the group that couldn't be explained LESS
          framing than an explained one — bare pre-ticked rows and a Delete button, with no
          description tied to the checkboxes at all. The friction should not be inversely
          proportional to the justification. */}
      <p
        className={group.reason ? styles.dupeReason : styles.dupeReasonMissing}
        id={reasonId}
      >
        {group.reason || "No reason given — check these carefully."}
      </p>

      <ul className={styles.dupeRows}>
        {items.map((s) => {
          // From `doomed`, never `selected` — so a reprieved row visibly un-ticks
          // itself rather than sitting ticked while surviving the delete.
          const ticked = doomed.some((d) => d.id === s.id);
          const locked = s.id === lockedId;
          return (
            <li key={s.id}>
              <label
                className={styles.dupeRow}
                data-ticked={ticked || undefined}
                data-locked={locked || undefined}
              >
                <input
                  type="checkbox"
                  className={styles.dupeCheck}
                  checked={ticked}
                  disabled={busy || locked}
                  onChange={() => toggle(s.id)}
                  // NO `aria-label` here, deliberately. An explicit label on the input
                  // wins over the wrapping <label>'s text, which would reduce the whole
                  // row to "Delete Untitled snippet" — and since most snippets are
                  // untitled, every row in the group would announce identically, hiding
                  // the content that is the only way to judge which version to keep.
                  // The <label> supplies the name; the reason describes the group.
                  // Unconditional: the reason line always renders now, and a reasonless
                  // group is exactly the one whose checkboxes most need a description.
                  aria-describedby={reasonId}
                />
                <span className={styles.dupeBody}>
                  <span className={styles.dupeName}>
                    {/* The name gets its own span so the struck-through treatment
                        can be scoped to it: `text-decoration` propagates to
                        descendants and a child can't opt out, so striking the
                        whole row would strike the pills beside it too. */}
                    <span className={styles.dupeNameText}>
                      {s.name.trim() || "Untitled snippet"}
                    </span>
                    {s.id === group.keep_id && (
                      <span className={styles.dupeSuggest}>
                        AI suggests keeping
                      </span>
                    )}
                    {locked && (
                      <span className={styles.dupeLocked}>
                        Kept — a group can't be emptied
                      </span>
                    )}
                  </span>
                  <span className={styles.dupeContent}>
                    <BlankedContent content={s.content} />
                  </span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>

      {error && <div className={styles.cardError}>{error}</div>}

      {/* Deleting here needs the same two-step gate as deleting a single snippet from
          its own card (⋯ → Delete → confirm) and as Organize itself. It is the most
          destructive control in the view — the rows arrive pre-ticked from a model's
          judgement, so an unconfirmed click would act on a default the user never
          chose, and there is no undo anywhere in the app. */}
      {/* Distinct `key`s per branch on purpose. Without them React reconciles these
          children by index, and index 1 is "Delete N" before the swap and "Cancel" after —
          the same DOM node, so the control under the user's focus silently changed identity
          from destructive to safe. A second Enter then cancelled, making the delete appear
          to do nothing, twice, with no announcement either time. */}
      <div className={styles.dupeFoot}>
        {confirming ? (
          <>
            {/* Announced, because the swap above replaces the focused control: otherwise
                the only signal that a confirmation appeared is visual. */}
            <span key="warn" className={styles.dupeWarn} role="alert">
              Delete {doomed.length} of these {items.length}? This can't be undone.
            </span>
            <button
              key="cancel"
              type="button"
              className={styles.secondaryBtn}
              onClick={cancelConfirm}
              disabled={busy}
            >
              Cancel
            </button>
            <button
              key="confirm-delete"
              type="button"
              className={styles.deleteBtn}
              onClick={() => void handleDelete()}
              disabled={busy}
              ref={confirmDeleteRef}
            >
              {busy ? "Deleting…" : "Delete"}
            </button>
          </>
        ) : (
          <>
            <button
              key="keep-all"
              type="button"
              className={styles.secondaryBtn}
              onClick={onDismiss}
              disabled={busy}
            >
              Keep all
            </button>
            {/* The card's own delete button, reused as-is — same affordance, and it
                already carries the danger treatment, disabled state, press feedback
                and reduced-motion handling. */}
            <button
              key="delete-trigger"
              type="button"
              className={styles.deleteBtn}
              onClick={() => setConfirming(true)}
              disabled={busy || doomed.length === 0}
              ref={deleteTriggerRef}
            >
              {`Delete ${doomed.length}`}
            </button>
          </>
        )}
      </div>
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
            <div className={styles.chips}>
              <CategoryChip
                snippet={snippet}
                categories={categories}
                onSet={onSetCategory}
                disabled={deleting}
              />
              <TopicChip topic={snippet.topic} />
            </div>
            <SavedIndicator visible={showSaved} />
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * Snippet content with its blanks rendered as slots rather than literal brackets.
 *
 * A fragment, not a wrapper: the two callers frame it differently (a proposal's body is
 * a <p>, a redundancy row's is a <span>), so each keeps its own element and only the
 * part-mapping is shared. Both places show unapproved or about-to-be-deleted text, and
 * both judgements — is this line reusable, is this the version to keep — depend on
 * seeing which words are the founder's own and which get filled in later.
 */
function BlankedContent({ content }: { content: string }) {
  return (
    <>
      {splitOnBlanks(content).map((part, i) =>
        part.kind === "blank" ? (
          <span key={i} className={styles.blank}>
            {/* The dashed pill says "this gets filled in" visually and nowhere else — a
                border isn't in the accessibility tree, so stripping the literal brackets
                left "Hi first name, we're SOC2 certified" announcing identically to a
                snippet that says exactly that. Both judgements this component supports
                turn on that distinction, and two group members differing only in where
                the blank sits used to read the same. */}
            <span className={styles.srOnly}>blank: </span>
            {part.label}
          </span>
        ) : (
          part.value
        ),
      )}
    </>
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
 * What a snippet is about, beside the stage it belongs to.
 *
 * Read-only on purpose, and styled a step quieter than the stage chip: the stage is the
 * axis you organize by and set by hand, the topic is the AI's read of the subject. Making
 * both look equally editable would misrepresent which one you control.
 *
 * Renders nothing when there's no topic. A blank topic is a normal, expected answer — a
 * line like "worth a quick call?" has no subject — so an empty slot is the right treatment
 * rather than a "＋ Topic" affordance that implies something is missing.
 */
function TopicChip({ topic }: { topic: string }) {
  const label = topic.trim();
  if (!label) return null;
  return (
    <span
      className={styles.topicChip}
      title="What this snippet is about — the AI picks this, and a draft prefers staying on the thread's current topic"
    >
      {label}
    </span>
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
 *
 * A proposal may carry blanks (`[first name]`) where a detail was tied to one
 * person. Those render as slots rather than as literal brackets: approving is a
 * judgement about whether the line is reusable, and that judgement depends on seeing
 * which parts are the founder's own words and which get filled in later.
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
  const blanked = hasBlank(snippet.content);

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

      <p className={styles.proposedContent}>
        <BlankedContent content={snippet.content} />
      </p>

      {error && <div className={styles.cardError}>{error}</div>}

      <div className={styles.proposedFoot}>
        <span className={styles.proposedHint}>
          {blanked
            ? "Blanks fill in per prospect"
            : "Spotted in a message you sent"}
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
