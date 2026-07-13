import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Pitch } from "../api/pitches";
import {
  deleteProspect,
  listProspects,
  onProspectsChanged,
  setProspectStage,
  type Prospect,
} from "../api/prospects";
import { listStages, onStagesChanged, type Stage } from "../api/stages";
import EmptyState from "../components/EmptyState";
import LoadError from "../components/LoadError";
import { errorMessage } from "../lib/errors";
import ProspectsList from "./ProspectsList";
import PipelineBoard from "./PipelineBoard";
import styles from "./ProspectsView.module.css";

interface Props {
  /** The active pitch this view is scoped to; null when none exist yet. */
  pitch: Pitch | null;
  onCreateNew: () => void;
}

type ViewMode = "pipeline" | "list";

/** Props shared by the two prospect views (list + pipeline). */
export interface ProspectViewProps {
  prospects: Prospect[];
  stages: Stage[];
  messagingStageId: number | null;
  /** Prospect ids with a stage write in flight — controls disable. */
  busyIds: Set<number>;
  onOpen: (url: string) => void;
  onMove: (id: number, stageId: number) => void;
  onDelete: (id: number) => Promise<void>;
}

const VIEW_KEY = "cp.prospects.view";

function readStoredView(): ViewMode {
  return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "pipeline";
}

/** Prospects tab: the people captured (via the Chrome extension) for the active
 *  pitch, shown as a flat list or a pipeline board. Capture happens in LinkedIn;
 *  here you move prospects through stages, track outreach, and delete. */
export default function ProspectsView({ pitch, onCreateNew }: Props) {
  const [prospects, setProspects] = useState<Prospect[] | null>(null);
  const [stages, setStages] = useState<Stage[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyIds, setBusyIds] = useState<Set<number>>(new Set());
  const [view, setView] = useState<ViewMode>(readStoredView);
  // Bumped by the retry button to re-run both loads after a failure.
  const [reloadKey, setReloadKey] = useState(0);

  // Prospects are the full set (filtered per pitch below); loaded once.
  useEffect(() => {
    let active = true;
    setLoadError(null);
    listProspects()
      .then((p) => active && setProspects(p))
      .catch((e) => active && setLoadError(errorMessage(e)));
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

  // Stages are per pitch — reload when the active pitch changes.
  useEffect(() => {
    if (!pitch) {
      setStages(null);
      return;
    }
    let active = true;
    listStages(pitch.id)
      .then((s) => active && setStages(s))
      .catch((e) => active && setLoadError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, [pitch?.id, reloadKey]);

  // Live refresh on pipeline edits made elsewhere (the pitch's stage editor in
  // Settings). Re-fetch stages *and* prospects together: a stage delete
  // reassigns prospects, so applying the two fetches atomically (one batched
  // update, not two independent promises) avoids a frame where the new stages
  // and the old prospects disagree — which would flash cards out of the board.
  // Silent background update.
  useEffect(() => {
    if (!pitch) return;
    const pitchId = pitch.id;
    let active = true;
    const unlisten = onStagesChanged(() => {
      Promise.all([listStages(pitchId), listProspects()])
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
  }, [pitch?.id]);

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

  async function handleMove(id: number, stageId: number) {
    if (busyIds.has(id)) return;
    setBusy(id, true);
    setActionError(null);
    try {
      const updated = await setProspectStage(id, stageId);
      setProspects((prev) => prev && prev.map((p) => (p.id === id ? updated : p)));
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusy(id, false);
    }
  }

  // Throws on failure so DeleteControl can surface the error and keep the row.
  async function handleDelete(id: number) {
    await deleteProspect(id);
    setProspects((prev) => prev && prev.filter((p) => p.id !== id));
  }

  // No pitches exist at all — prospects attach to a pitch, so there's nothing
  // to scope to yet. Nudge toward creating the first pitch.
  if (!pitch) {
    return (
      <EmptyState
        title="No pitches yet"
        body="Prospects attach to a pitch. Create your first pitch, then capture people from LinkedIn."
        actionLabel="Create your first pitch"
        onAction={onCreateNew}
      />
    );
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

  const scoped = prospects.filter((p) => p.pitch_id === pitch.id);

  if (scoped.length === 0) {
    return (
      <div className={styles.blank}>
        <h2 className={styles.blankTitle}>No prospects yet</h2>
        <p className={styles.blankBody}>
          Open a LinkedIn chat and hit <strong>Add to Prospects</strong> with{" "}
          <strong>{pitch.name}</strong> selected — they'll show up here.
        </p>
      </div>
    );
  }

  const messagingStageId = stages.find((s) => s.kind === "messaging")?.id ?? null;
  const viewProps: ProspectViewProps = {
    prospects: scoped,
    stages,
    messagingStageId,
    busyIds,
    onOpen: (url) => void openUrl(url),
    onMove: handleMove,
    onDelete: handleDelete,
  };

  return (
    <section className={view === "pipeline" ? styles.wrapWide : styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Prospects</h1>
        <span className={styles.count}>{scoped.length}</span>
        <span className={styles.headSpacer} aria-hidden="true" />
        <ViewToggle view={view} onChange={changeView} />
      </header>

      {actionError && (
        <div className={styles.actionError} role="alert">
          {actionError}
        </div>
      )}

      {view === "list" ? (
        <ProspectsList {...viewProps} />
      ) : (
        <PipelineBoard {...viewProps} />
      )}
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
