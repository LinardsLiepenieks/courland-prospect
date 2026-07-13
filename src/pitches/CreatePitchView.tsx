import { FormEvent, useEffect, useRef, useState } from "react";
import { polishSkill } from "../api/pitches";
import type { StageInput } from "../api/stages";
import { useAsyncAction } from "../lib/useAsyncAction";
import PolishButton from "../components/PolishButton";
import StageEditor, { type DraftStage, fullCycleDraft } from "./StageEditor";
import styles from "./CreatePitchView.module.css";

interface Props {
  onCreate: (name: string, skill: string, stages: StageInput[]) => Promise<void>;
  onCancel: () => void;
}

/** Full-page pitch setup — the whole app surface switches to this screen
 *  (the shell's navbar/tabs are unmounted while it's shown). */
export default function CreatePitchView({ onCreate, onCancel }: Props) {
  const [name, setName] = useState("");
  const [skill, setSkill] = useState("");
  // Seeded with the Full-cycle template; the user tweaks it before creating.
  const [stages, setStages] = useState<DraftStage[]>(fullCycleDraft);
  const [polishing, setPolishing] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const { busy: submitting, error, setError, run } = useAsyncAction();

  // Block submit while polishing so the incoming rewrite can't land after the
  // pitch is already created (and so the skill field stays locked).
  const canSubmit = name.trim().length > 0 && !submitting && !polishing;

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

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    // On failure `run` keeps the form open with its values (in `error`) to retry.
    run(() =>
      onCreate(
        name.trim(),
        skill.trim(),
        stages.map((s) => ({ name: s.name, kind: s.kind, color: s.color })),
      ),
    );
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

          <div className={styles.skillHeader}>
            <label className={styles.fieldLabel} htmlFor="pitch-skill">
              Skill
            </label>
            <PolishButton
              text={skill}
              polish={polishSkill}
              disabled={submitting}
              onPolished={setSkill}
              onError={setError}
              onBusyChange={setPolishing}
            />
          </div>
          <textarea
            id="pitch-skill"
            className={styles.textarea}
            placeholder="What is this pitch about? The angle, who it's for, why it lands."
            value={skill}
            onChange={(e) => setSkill(e.target.value)}
            disabled={submitting || polishing}
          />

          <span className={`${styles.fieldLabel} ${styles.stagesLabel}`}>
            Pipeline stages
          </span>
          <p className={styles.hint}>
            The funnel prospects move through. The first is the messaging stage,
            where you track outreach. Tweak now or later in Settings.
          </p>
          <StageEditor
            stages={stages}
            onChange={(next) => setStages(next)}
            disabled={submitting}
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
