import { FormEvent, useRef, useState } from "react";
import type { Pitch } from "../api/pitches";
import EmptyState from "../components/EmptyState";
import { errorMessage } from "../lib/errors";
import styles from "./EditPitchView.module.css";

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
  // Synchronous re-entry guards (state updates are async — see CreatePitchView).
  const savingRef = useRef(false);
  const deletingRef = useRef(false);

  const trimmedName = name.trim();
  const dirty = trimmedName !== pitch.name || skill.trim() !== pitch.skill;
  const busy = saving || deleting;
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

        <label className={styles.fieldLabel} htmlFor="edit-skill">
          Skill
        </label>
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
          <span
            className={styles.saved}
            data-visible={showSaved}
            aria-hidden={!showSaved}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path
                d="m5 12.5 4.5 4.5L19 7"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            Saved
          </span>
          <button
            type="submit"
            className={styles.primaryBtn}
            disabled={!canSave}
          >
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </form>

      <section className={styles.danger}>
        <div className={styles.dangerText}>
          <div className={styles.dangerTitle}>Delete pitch</div>
          <div className={styles.dangerBody}>
            Remove this pitch permanently. This can't be undone.
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
