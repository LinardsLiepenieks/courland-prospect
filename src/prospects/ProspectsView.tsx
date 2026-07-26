import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listCustomers, type Customer } from "../api/customers";
import {
  deleteProspect,
  listProspects,
  onProspectsChanged,
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
  type Stage,
} from "../api/stages";
import LoadError from "../components/LoadError";
import { errorMessage } from "../lib/errors";
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
  onOpen: (url: string) => void;
  onMove: (id: number, stageId: number) => void;
  onSetCustomer: (id: number, customerId: number | null) => void;
  onDelete: (id: number) => Promise<void>;
}

const VIEW_KEY = "cp.prospects.view";

function readStoredView(): ViewMode {
  return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "pipeline";
}

/** Map persisted stages to editor drafts (stable key derived from the id). */
function toDrafts(stages: Stage[]): DraftStage[] {
  return stages.map((s) => ({
    key: `s${s.id}`,
    id: s.id,
    name: s.name,
    kind: s.kind,
    color: s.color,
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
  const [view, setView] = useState<ViewMode>(readStoredView);
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
  const viewProps: ProspectViewProps = {
    prospects,
    stages,
    customers,
    messagingStageId,
    busyIds,
    onOpen: (url) => void openUrl(url),
    onMove: (id, stageId) => void writeProspect(id, () => setProspectStage(id, stageId)),
    onSetCustomer: (id, customerId) =>
      void writeProspect(id, () => setProspectCustomer(id, customerId)),
    onDelete: handleDelete,
  };

  return (
    <section className={view === "pipeline" ? styles.wrapWide : styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Prospects</h1>
        <span className={styles.count}>{prospects.length}</span>
        <span className={styles.headSpacer} aria-hidden="true" />
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
      ) : view === "list" ? (
        <ProspectsList {...viewProps} />
      ) : (
        <PipelineBoard {...viewProps} />
      )}

      <PipelineSection stages={stages} />
    </section>
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
        <div className={styles.sectionTitle}>Pipeline stages</div>
        <div className={styles.sectionBody}>
          The funnel everyone moves through, whichever customer profile they
          match. The first stage is the messaging stage.
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
