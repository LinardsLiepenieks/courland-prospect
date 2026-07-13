import styles from "./LoadError.module.css";

interface Props {
  /** What failed to load, e.g. "prospects" — completes "Couldn't load …". */
  what: string;
  /** The underlying failure message (from the rejected command). */
  detail?: string | null;
  /** Re-run the load. */
  onRetry: () => void;
}

/**
 * A load failure that the user can recover from without restarting the app.
 * Replaces the dead-end error text that several views showed: a transient
 * backend hiccup would otherwise strand the surface until it remounted.
 */
export default function LoadError({ what, detail, onRetry }: Props) {
  return (
    <div className={styles.error} role="alert">
      <p className={styles.message}>Couldn't load {what}.</p>
      {detail && <p className={styles.detail}>{detail}</p>}
      <button type="button" className={styles.retry} onClick={onRetry}>
        Try again
      </button>
    </div>
  );
}
