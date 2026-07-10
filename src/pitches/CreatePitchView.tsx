import { FormEvent, useEffect, useRef, useState } from "react";
import { errorMessage } from "../lib/errors";
import styles from "./CreatePitchView.module.css";

interface Props {
  onCreate: (name: string, skill: string) => Promise<void>;
  onCancel: () => void;
}

/** Full-page pitch setup — the whole app surface switches to this screen
 *  (the shell's navbar/tabs are unmounted while it's shown). */
export default function CreatePitchView({ onCreate, onCancel }: Props) {
  const [name, setName] = useState("");
  const [skill, setSkill] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  // Synchronous re-entry guard: state updates are async, so two submit events
  // in one frame (Enter-repeat, Enter+click) could both pass a state-only check.
  const submittingRef = useRef(false);

  const canSubmit = name.trim().length > 0 && !submitting;

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  // Escape cancels, matching the dropdown/menu convention.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && !submitting) onCancel();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel, submitting]);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    // Guard order matters: bail on invalid/empty BEFORE claiming the ref, or a
    // rejected submit would leave the ref stuck true and dead-lock the form.
    if (!canSubmit || submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate(name.trim(), skill.trim());
    } catch (err) {
      // Keep the form open with its values so the user can retry.
      setError(errorMessage(err));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <main className={styles.screen}>
      <header className={styles.topbar}>
        <div className={styles.topbarInner}>
          <button
            type="button"
            className={styles.back}
            onClick={onCancel}
            disabled={submitting}
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              aria-hidden="true"
            >
              <path
                d="m15 6-6 6 6 6"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            Back
          </button>
        </div>
      </header>

      <div className={styles.body}>
        <div className={styles.intro}>
          <h1 className={styles.title}>New pitch</h1>
          <p className={styles.subtitle}>
            Set up a distinct thing you're selling.
          </p>
        </div>

        <form className={styles.form} onSubmit={handleSubmit}>
          <label className={styles.fieldLabel} htmlFor="pitch-name">
            Name
          </label>
          <input
            id="pitch-name"
            ref={nameRef}
            className={styles.input}
            placeholder="e.g. Design-in-code"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />

          <label className={styles.fieldLabel} htmlFor="pitch-skill">
            Skill
          </label>
          <textarea
            id="pitch-skill"
            className={styles.textarea}
            placeholder="What is this pitch about? The angle, who it's for, why it lands."
            value={skill}
            onChange={(e) => setSkill(e.target.value)}
          />

          {error && <div className={styles.error}>{error}</div>}

          <div className={styles.actions}>
            <button
              type="button"
              className={styles.secondaryBtn}
              onClick={onCancel}
              disabled={submitting}
            >
              Cancel
            </button>
            <button
              type="submit"
              className={styles.primaryBtn}
              disabled={!canSubmit}
            >
              {submitting ? "Creating…" : "Create pitch"}
            </button>
          </div>
        </form>
      </div>
    </main>
  );
}
