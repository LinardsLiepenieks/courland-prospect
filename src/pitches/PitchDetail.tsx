import type { Pitch } from "../api/pitches";
import { formatDate } from "../lib/date";
import EmptyState from "../components/EmptyState";
import styles from "./PitchDetail.module.css";

interface Props {
  pitch: Pitch | null;
  onCreateNew: () => void;
}

/** Pitch tab content: read-only detail of the active pitch, or an empty
 *  state prompting the user to create their first one. */
export default function PitchDetail({ pitch, onCreateNew }: Props) {
  if (!pitch) {
    return (
      <EmptyState
        title="No pitches yet"
        body="A pitch is a distinct thing you're selling. Create your first one to get started."
        actionLabel="Create your first pitch"
        onAction={onCreateNew}
      />
    );
  }

  return (
    <article className={styles.detail}>
      <h1 className={styles.name}>{pitch.name}</h1>
      <section className={styles.field}>
        <div className={styles.label}>Skill</div>
        {pitch.skill ? (
          <p className={styles.skill}>{pitch.skill}</p>
        ) : (
          <p className={styles.skillEmpty}>No skill described.</p>
        )}
      </section>
      <div className={styles.meta}>Added {formatDate(pitch.created_at)}</div>
    </article>
  );
}
