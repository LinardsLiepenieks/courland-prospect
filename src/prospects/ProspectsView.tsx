import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listCustomers, type Customer } from "../api/customers";
import {
  acceptStageSuggestion,
  deleteProspect,
  dismissStageSuggestion,
  listProspects,
  onProspectsChanged,
  recheckProspectStage,
  setProspectCustomer,
  setProspectStage,
  type Prospect,
} from "../api/prospects";
import {
  createStage,
  deleteStage,
  listStages,
  onStagesChanged,
  renameStage,
  reorderStages,
  setStageColor,
  setStageGoal,
  setStageThresholds,
  type Stage,
} from "../api/stages";
import LoadError from "../components/LoadError";
import { errorMessage } from "../lib/errors";
import { useNow } from "../lib/useNow";
import ProspectsList from "./ProspectsList";
import PipelineBoard from "./PipelineBoard";
import StageEditor, { type DraftStage, type StageChange } from "./StageEditor";
import styles from "./ProspectsView.module.css";

type ViewMode = "pipeline" | "list";

/** Props shared by the two prospect views (list + pipeline). */
export interface ProspectViewProps {
  prospects: Prospect[];
  stages: Stage[];
  customers: Customer[];
  messagingStageId: number | null;
  /** Prospect ids with a write in flight — controls disable. */
  busyIds: Set<number>;
  /** Prospect ids with a manual advance re-check running. A subset of `busyIds`
   *  kept separate because this one is slow (it shells out to Claude Code) and
   *  earns its own "Checking…" treatment rather than silent disabling. */
  checkingIds: Set<number>;
  /** The instant staleness is graded against. Owned here and passed down so both
   *  views share ONE clock — two cards a millisecond apart can't land on opposite
   *  sides of a day boundary — and so it advances on its own (see `useNow`)
   *  rather than freezing at whenever the view mounted. */
  now: number;
  onOpen: (url: string) => void;
  onMove: (id: number, stageId: number) => void;
  onSetCustomer: (id: number, customerId: number | null) => void;
  /** Take the analyzer's pending advance suggestion (moves them). */
  onAcceptSuggestion: (id: number) => void;
  /** Wave it off without moving them. */
  onDismissSuggestion: (id: number) => void;
  /** Re-run the advance check for this prospect now. */
  onRecheck: (id: number) => void;
  onDelete: (id: number) => Promise<void>;
}

const VIEW_KEY = "cp.prospects.view";

function readStoredView(): ViewMode {
  return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "pipeline";
}

/** The customer-profile filter's value. A number is a profile id; `"all"` shows
 *  everyone and `"none"` shows only unassigned prospects — which is its own
 *  useful view (people whose drafts have no goal to aim at yet). */
type CustomerFilter = number | "all" | "none";

/** Apply the filter. Kept out of the component so the rule — and the fact that
 *  `"none"` means `customer_id == null`, not "no filter" — is stated once. */
function filterByCustomer(prospects: Prospect[], filter: CustomerFilter): Prospect[] {
  if (filter === "all") return prospects;
  if (filter === "none") return prospects.filter((p) => p.customer_id == null);
  return prospects.filter((p) => p.customer_id === filter);
}

/** Map persisted stages to editor drafts (stable key derived from the id). */
function toDrafts(stages: Stage[]): DraftStage[] {
  return stages.map((s) => ({
    key: `s${s.id}`,
    id: s.id,
    name: s.name,
    kind: s.kind,
    color: s.color,
    goal: s.goal,
    warnDays: s.warn_days,
    staleDays: s.stale_days,
  }));
}

/** Prospects tab: everyone captured from LinkedIn, in the one pipeline, shown as
 *  a board or a flat list. Capture happens in LinkedIn; here you move people
 *  through stages, re-tag which customer profile they match, and delete.
 *
 *  The pipeline editor lives at the bottom of this page rather than in a separate
 *  settings tab — there's one pipeline now, and editing it right under the board
 *  it reorganizes is the whole of what "settings" used to be. */
export default function ProspectsView() {
  const [prospects, setProspects] = useState<Prospect[] | null>(null);
  const [stages, setStages] = useState<Stage[] | null>(null);
  const [customers, setCustomers] = useState<Customer[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyIds, setBusyIds] = useState<Set<number>>(new Set());
  // Synchronous mirror of `busyIds`. State updates are async, so two writes for
  // the SAME prospect issued in one frame would both pass a state-based check —
  // and since each command returns the whole row, the later response would
  // overwrite the other's column with a pre-write value. Reachable now that a row
  // has two write paths (stage move and customer re-tag) sitting side by side.
  const busyRef = useRef<Set<number>>(new Set());
  const [checkingIds, setCheckingIds] = useState<Set<number>>(new Set());
  // Ticks hourly (and on refocus), so a board left open re-grades instead of
  // holding the verdict it rendered with.
  const now = useNow();
  const [view, setView] = useState<ViewMode>(readStoredView);
  // Which kind of buyer the board is narrowed to. Deliberately NOT persisted:
  // a filter you forgot you set is a board that silently lies about how much
  // work you have, and this one resets every time you open the tab.
  const [customerFilter, setCustomerFilter] = useState<CustomerFilter>("all");
  // Bumped by the retry button to re-run the loads after a failure.
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let active = true;
    setLoadError(null);
    Promise.all([listProspects(), listStages()])
      .then(([p, s]) => {
        if (!active) return;
        setProspects(p);
        setStages(s);
      })
      .catch((e) => active && setLoadError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, [reloadKey]);

  // The customer list only backs the per-row re-tag menu. A failure here isn't
  // fatal — the menu just offers nothing — so it loads separately and stays
  // silent on error rather than taking over the board.
  useEffect(() => {
    let active = true;
    listCustomers()
      .then((c) => active && setCustomers(c))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [reloadKey]);

  // Live refresh: the Chrome extension captures prospects and sent-message counts
  // out of band, so re-fetch when the backend says something changed. Silent
  // background update — a transient failure keeps the current list rather than
  // taking over the view with an error.
  useEffect(() => {
    let active = true;
    const unlisten = onProspectsChanged(() => {
      listProspects()
        .then((p) => active && setProspects(p))
        .catch(() => {});
    });
    return () => {
      active = false;
      void unlisten.then((off) => off());
    };
  }, []);

  // Live refresh on pipeline edits — including the editor below, which is why the
  // board never goes stale while you reorganize it. Re-fetch stages *and*
  // prospects together: a stage delete reassigns prospects, so applying the two
  // in one batched update avoids a frame where the new stages and the old
  // prospects disagree, which would flash cards out of the board.
  useEffect(() => {
    let active = true;
    const unlisten = onStagesChanged(() => {
      Promise.all([listStages(), listProspects()])
        .then(([s, p]) => {
          if (!active) return;
          setStages(s);
          setProspects(p);
        })
        .catch(() => {});
    });
    return () => {
      active = false;
      void unlisten.then((off) => off());
    };
  }, []);

  function changeView(next: ViewMode) {
    setView(next);
    localStorage.setItem(VIEW_KEY, next);
  }

  function setBusy(id: number, on: boolean) {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  /** Run a per-prospect write behind the shared busy/error handling, applying the
   *  returned row in place. Stage moves and customer re-tags are the same shape. */
  async function writeProspect(id: number, write: () => Promise<Prospect>) {
    if (busyRef.current.has(id)) return;
    busyRef.current.add(id);
    setBusy(id, true);
    setActionError(null);
    try {
      const updated = await write();
      setProspects((prev) => prev && prev.map((p) => (p.id === id ? updated : p)));
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      busyRef.current.delete(id);
      setBusy(id, false);
    }
  }

  /** A manual advance re-check. Same write plumbing as everything else, wrapped
   *  in its own flag so the card can say "Checking…" for the seconds the CLI
   *  takes. A verdict of "not yet" comes back as the unchanged row, which is a
   *  perfectly good answer and deliberately not surfaced as an error. */
  function handleRecheck(id: number) {
    if (busyRef.current.has(id)) return;
    setCheckingIds((prev) => new Set(prev).add(id));
    void writeProspect(id, () => recheckProspectStage(id)).finally(() =>
      setCheckingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      }),
    );
  }

  // Throws on failure so DeleteControl can surface the error and keep the row.
  async function handleDelete(id: number) {
    await deleteProspect(id);
    setProspects((prev) => prev && prev.filter((p) => p.id !== id));
  }

  if (loadError) {
    return (
      <LoadError
        what="prospects"
        detail={loadError}
        onRetry={() => setReloadKey((k) => k + 1)}
      />
    );
  }

  if (!prospects || !stages) {
    // Local SQLite resolves near-instantly; reserve layout without a spinner
    // flash on the common fast path.
    return <div className={styles.loading} aria-busy="true" aria-hidden="true" />;
  }

  const messagingStageId = stages.find((s) => s.kind === "messaging")?.id ?? null;
  const visible = filterByCustomer(prospects, customerFilter);
  const viewProps: ProspectViewProps = {
    prospects: visible,
    stages,
    customers,
    messagingStageId,
    busyIds,
    checkingIds,
    now,
    onOpen: (url) => void openUrl(url),
    onMove: (id, stageId) => void writeProspect(id, () => setProspectStage(id, stageId)),
    onSetCustomer: (id, customerId) =>
      void writeProspect(id, () => setProspectCustomer(id, customerId)),
    onAcceptSuggestion: (id) => void writeProspect(id, () => acceptStageSuggestion(id)),
    onDismissSuggestion: (id) => void writeProspect(id, () => dismissStageSuggestion(id)),
    onRecheck: handleRecheck,
    onDelete: handleDelete,
  };

  return (
    <section className={view === "pipeline" ? styles.wrapWide : styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Prospects</h1>
        {/* The count follows the filter, so it always answers "how many am I
            looking at" — with the total alongside when the two differ, so a
            narrowed board can never be mistaken for an empty pipeline. */}
        <span className={styles.count}>
          {visible.length}
          {visible.length !== prospects.length && (
            <span className={styles.countTotal}> / {prospects.length}</span>
          )}
        </span>
        <span className={styles.headSpacer} aria-hidden="true" />
        {customers.length > 0 && (
          <CustomerFilterSelect
            customers={customers}
            value={customerFilter}
            onChange={setCustomerFilter}
          />
        )}
        <ViewToggle view={view} onChange={changeView} />
      </header>

      {actionError && (
        <div className={styles.actionError} role="alert">
          {actionError}
        </div>
      )}

      {prospects.length === 0 ? (
        <div className={styles.blank}>
          <h2 className={styles.blankTitle}>No prospects yet</h2>
          <p className={styles.blankBody}>
            Open a LinkedIn chat and hit <strong>Add to Prospects</strong> — pick
            the customer profile they match and they'll show up here.
          </p>
        </div>
      ) : visible.length === 0 ? (
        // Filtered to nothing. Distinct from the empty pipeline above, and it
        // offers the way out rather than leaving you staring at a blank board.
        <div className={styles.blank}>
          <h2 className={styles.blankTitle}>Nobody in this filter</h2>
          <p className={styles.blankBody}>
            None of your {prospects.length} prospects match it.{" "}
            <button
              type="button"
              className={styles.blankAction}
              onClick={() => setCustomerFilter("all")}
            >
              Show all
            </button>
          </p>
        </div>
      ) : view === "list" ? (
        <ProspectsList {...viewProps} />
      ) : (
        <PipelineBoard {...viewProps} />
      )}

      <PipelineSection stages={stages} />
    </section>
  );
}

/**
 * Narrow the board to one kind of buyer.
 *
 * A native `<select>` rather than the app's `Popover` menu: this is a plain
 * one-of-N choice with no per-item affordances (unlike the card pills, which
 * carry color dots and a clear action), and the native control brings keyboard
 * behavior, type-ahead, and platform styling for free. Hidden entirely when
 * there are no customer profiles — a filter with one option is furniture.
 */
function CustomerFilterSelect({
  customers,
  value,
  onChange,
}: {
  customers: Customer[];
  value: CustomerFilter;
  onChange: (next: CustomerFilter) => void;
}) {
  return (
    <label className={styles.filter} data-active={value !== "all" || undefined}>
      <span className={styles.filterLabel}>Customer</span>
      <select
        className={styles.filterSelect}
        value={String(value)}
        onChange={(e) => {
          const raw = e.target.value;
          onChange(raw === "all" || raw === "none" ? raw : Number(raw));
        }}
        aria-label="Filter by customer profile"
      >
        <option value="all">All</option>
        {customers.map((c) => (
          <option key={c.id} value={String(c.id)}>
            {c.name}
          </option>
        ))}
        <option value="none">No profile</option>
      </select>
    </label>
  );
}

/** Segmented Pipeline / List switch. */
function ViewToggle({
  view,
  onChange,
}: {
  view: ViewMode;
  onChange: (v: ViewMode) => void;
}) {
  return (
    <div className={styles.toggle} role="group" aria-label="View mode">
      <button
        type="button"
        className={styles.toggleBtn}
        data-active={view === "pipeline" || undefined}
        onClick={() => onChange("pipeline")}
        aria-pressed={view === "pipeline"}
      >
        Pipeline
      </button>
      <button
        type="button"
        className={styles.toggleBtn}
        data-active={view === "list" || undefined}
        onClick={() => onChange("list")}
        aria-pressed={view === "list"}
      >
        List
      </button>
    </div>
  );
}

/**
 * The pipeline, edited in place beneath the board it reorganizes. Each edit
 * persists immediately via the stage commands; the editor locks (`busy`) during a
 * write so operations serialize and the just-written row always has its server id
 * before the next action. The board above reconciles off the backend's
 * `stages://changed` event, so the two never disagree.
 *
 * `stages` is passed down (rather than fetched again) so the editor and the board
 * always render the same list — this section is a sibling view of the same data,
 * not an independent one.
 */
function PipelineSection({ stages }: { stages: Stage[] }) {
  const [drafts, setDrafts] = useState<DraftStage[]>(() => toDrafts(stages));
  const [busy, setBusy] = useState(false);
  const [opError, setOpError] = useState<string | null>(null);
  // Serialize writes: state updates are async, so a synchronous guard stops two
  // clicks in one frame from both passing the `busy` check.
  const busyRef = useRef(false);

  // Follow the authoritative list, except while a write is in flight — then the
  // optimistic drafts are newer than anything the parent could hand us.
  useEffect(() => {
    if (!busyRef.current) setDrafts(toDrafts(stages));
  }, [stages]);

  async function persist(prev: DraftStage[], next: DraftStage[], change: StageChange) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setOpError(null);
    // Optimistic: show the edit immediately, reconcile with the server after.
    setDrafts(next);
    try {
      switch (change.type) {
        case "add": {
          const added = next.find((s) => s.id === undefined);
          if (added) await createStage(added.name);
          break;
        }
        case "remove": {
          const row = prev.find((s) => s.key === change.key);
          if (row?.id != null) await deleteStage(row.id);
          break;
        }
        case "rename": {
          const row = prev.find((s) => s.key === change.key);
          if (row?.id != null) await renameStage(row.id, change.name);
          break;
        }
        case "color": {
          const row = prev.find((s) => s.key === change.key);
          if (row?.id != null) await setStageColor(row.id, change.color);
          break;
        }
        case "goal": {
          const row = prev.find((s) => s.key === change.key);
          if (row?.id != null) await setStageGoal(row.id, change.goal);
          break;
        }
        case "thresholds": {
          const row = prev.find((s) => s.key === change.key);
          if (row?.id != null) {
            await setStageThresholds(row.id, change.warnDays, change.staleDays);
          }
          break;
        }
        case "reorder": {
          const ids = next.map((s) => s.id).filter((id): id is number => id != null);
          await reorderStages(ids);
          break;
        }
      }
      // Reload so ids/positions are authoritative (esp. after an add). The
      // command also emits `stages://changed`, which refreshes the board.
      setDrafts(toDrafts(await listStages()));
    } catch (err) {
      setDrafts(prev); // revert the optimistic edit
      setOpError(errorMessage(err));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  return (
    <section className={styles.pipeline}>
      <div className={styles.pipelineHead}>
        <div className={styles.sectionTitle}>Cycle stages</div>
        <div className={styles.sectionBody}>
          The funnel everyone moves through, whichever customer profile they
          match. The first stage is the messaging stage. Give a stage a goal and
          drafts for the people in it aim at that goal — and after every new
          message, Claude checks whether they've met it and offers to move them
          on.
        </div>
      </div>
      <StageEditor
        stages={drafts}
        onChange={(next, change) => void persist(drafts, next, change)}
        disabled={busy}
      />
      {opError && <div className={styles.opError}>{opError}</div>}
    </section>
  );
}
