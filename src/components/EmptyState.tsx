import styles from "./EmptyState.module.css";

interface Props {
  title: string;
  body: string;
  actionLabel: string;
  onAction: () => void;
}

/** Centered empty state with a title, supporting copy, and a single action. */
export default function EmptyState({ title, body, actionLabel, onAction }: Props) {
  return (
    <div className={styles.empty}>
      <h2 className={styles.title}>{title}</h2>
      <p className={styles.body}>{body}</p>
      <button type="button" className={styles.btn} onClick={onAction}>
        {actionLabel}
      </button>
    </div>
  );
}
