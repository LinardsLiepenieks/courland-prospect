import { FormEvent, useEffect, useRef, useState } from "react";
import { polishSkill, type Pitch } from "../api/pitches";
import {
  createStage,
  deleteStage,
  listStages,
  renameStage,
  reorderStages,
  setStageColor,
  type Stage,
} from "../api/stages";
import EmptyState from "../components/EmptyState";
import LoadError from "../components/LoadError";
import SavedIndicator from "../components/SavedIndicator";
import SnippetsSection from "../components/SnippetsSection";
import { errorMessage } from "../lib/errors";
import PolishButton from "../components/PolishButton";
import StageEditor, {
  type DraftStage,
  type StageChange,
} from "./StageEditor";
import styles from "./EditPitchView.module.css";

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

interface Props {
  pitch: Pitch | null;
  onSave: (id: number, name: string, skill: string) => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onCreateNew: () => void;
}

/** Edit tab: edit the active pitch's name + skill, or delete it. Renders an
 *  empty state when no pitch is selected. */
export default function EditPitchView({
  pitch,
  onSave,
  onDelete,
  onCreateNew,
}: Props) {
  if (!pitch) {
    return (
      <EmptyState
        title="Nothing to edit"
        body="Select a pitch from the dropdown, or create one to start editing."
        actionLabel="Create a pitch"
        onAction={onCreateNew}
      />
    );
  }
  // Keyed by pitch id so switching the active pitch remounts with fresh state.
  return <EditForm key={pitch.id} pitch={pitch} onSave={onSave} onDelete={onDelete} />;
}

function EditForm({
  pitch,
  onSave,
  onDelete,
}: {
  pitch: Pitch;
  onSave: Props["onSave"];
  onDelete: Props["onDelete"];
}) {
  const [name, setName] = useState(pitch.name);
  const [skill, setSkill] = useState(pitch.skill);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [polishing, setPolishing] = useState(false);
  // Synchronous re-entry guards (state updates are async — see CreatePitchView).
  const savingRef = useRef(false);
  const deletingRef = useRef(false);

  const trimmedName = name.trim();
  const dirty = trimmedName !== pitch.name || skill.trim() !== pitch.skill;
  // Polishing joins `busy` so the skill field + Save lock while the rewrite is
  // in flight — otherwise the incoming result would clobber concurrent typing.
  const busy = saving || deleting || polishing;
  const canSave = trimmedName.length > 0 && dirty && !busy;
  const showSaved = saved && !dirty;

  // A fresh edit invalidates the "Saved" confirmation.
  function edit(setter: (v: string) => void, value: string) {
    setter(value);
    setSaved(false);
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!canSave || savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setError(null);
    try {
      await onSave(pitch.id, trimmedName, skill.trim());
      setSaved(true);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (deletingRef.current) return;
    deletingRef.current = true;
    setDeleting(true);
    setError(null);
    try {
      await onDelete(pitch.id);
      // On success the pitch is gone and the shell navigates away — no local
      // state to reset.
    } catch (err) {
      setError(errorMessage(err));
      setConfirmingDelete(false);
      deletingRef.current = false;
      setDeleting(false);
    }
  }

  return (
    <div className={styles.edit}>
      <form className={styles.form} onSubmit={handleSubmit}>
        <label className={styles.fieldLabel} htmlFor="edit-name">
          Name
        </label>
        <input
          id="edit-name"
          className={styles.input}
          value={name}
          onChange={(e) => edit(setName, e.target.value)}
          disabled={busy}
        />

        <div className={styles.skillHeader}>
          <label className={styles.fieldLabel} htmlFor="edit-skill">
            Skill
          </label>
          <PolishButton
            text={skill}
            polish={polishSkill}
            disabled={busy}
            onPolished={(t) => edit(setSkill, t)}
            onError={setError}
            onBusyChange={setPolishing}
          />
        </div>
        <textarea
          id="edit-skill"
          className={styles.textarea}
          placeholder="What is this pitch about? The angle, who it's for, why it lands."
          value={skill}
          onChange={(e) => edit(setSkill, e.target.value)}
          disabled={busy}
        />

        {error && <div className={styles.error}>{error}</div>}

        <div className={styles.actions}>
          <SavedIndicator visible={showSaved} />
          <button
            type="submit"
            className={styles.primaryBtn}
            disabled={!canSave}
          >
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </form>

      <PipelineSection pitchId={pitch.id} />

      <section className={styles.snippetsBlock}>
        <div className={styles.pipelineHead}>
          <div className={styles.sectionTitle}>Snippets</div>
          <div className={styles.sectionBody}>
            Reusable text fragments for this pitch — you'll draw on them when
            writing messages.
          </div>
        </div>
        {/* Keyed by pitch id so switching the active pitch remounts with the new
            scope's snippets rather than reusing stale state. */}
        <SnippetsSection key={pitch.id} pitchId={pitch.id} />
      </section>

      <section className={styles.danger}>
        <div className={styles.dangerText}>
          <div className={styles.dangerTitle}>Delete pitch</div>
          <div className={styles.dangerBody}>
            Remove this pitch and everything captured for it — its prospects and
            their message history. This can't be undone.
          </div>
        </div>
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
              onClick={handleDelete}
              disabled={deleting}
            >
              {deleting ? "Deleting…" : "Delete"}
            </button>
          </div>
        ) : (
          <button
            type="button"
            className={styles.deleteTrigger}
            onClick={() => setConfirmingDelete(true)}
            disabled={busy}
          >
            Delete
          </button>
        )}
      </section>
    </div>
  );
}

/** The pitch's pipeline, edited in place. Each edit persists immediately via the
 *  stage commands; the editor is locked (`busy`) during a write so operations
 *  serialize and the just-written row always has its server id before the next
 *  action. After any successful write we reload the authoritative list. */
function PipelineSection({ pitchId }: { pitchId: number }) {
  const [stages, setStages] = useState<DraftStage[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);
  const [busy, setBusy] = useState(false);
  const [opError, setOpError] = useState<string | null>(null);
  // Serialize writes: state updates are async, so a synchronous guard stops two
  // clicks in one frame from both passing the `busy` check.
  const busyRef = useRef(false);

  useEffect(() => {
    let active = true;
    setLoadError(null);
    listStages(pitchId)
      .then((s) => active && setStages(toDrafts(s)))
      .catch((e) => active && setLoadError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, [pitchId, reloadKey]);

  async function persist(
    prev: DraftStage[],
    next: DraftStage[],
    change: StageChange,
  ) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setOpError(null);
    // Optimistic: show the edit immediately, reconcile with the server after.
    setStages(next);
    try {
      switch (change.type) {
        case "add": {
          const added = next.find((s) => s.id === undefined);
          if (added) await createStage(pitchId, added.name);
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
          const ids = next
            .map((s) => s.id)
            .filter((id): id is number => id != null);
          await reorderStages(pitchId, ids);
          break;
        }
      }
      // Reload so ids/positions are authoritative (esp. after an add).
      setStages(toDrafts(await listStages(pitchId)));
    } catch (err) {
      setStages(prev); // revert the optimistic edit
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
          The funnel prospects move through. The first is the messaging stage.
        </div>
      </div>
      {loadError ? (
        <LoadError
          what="stages"
          detail={loadError}
          onRetry={() => setReloadKey((k) => k + 1)}
        />
      ) : stages ? (
        <>
          <StageEditor
            stages={stages}
            onChange={(next, change) => void persist(stages, next, change)}
            disabled={busy}
          />
          {opError && <div className={styles.error}>{opError}</div>}
        </>
      ) : (
        <div className={styles.pipelineLoading} aria-hidden="true" />
      )}
    </section>
  );
}
