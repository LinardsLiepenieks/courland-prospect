import styles from "./ProspectsView.module.css";

/** Placeholder for the prospects feature, scoped to the active pitch. */
export default function ProspectsView() {
  return (
    <div className={styles.wrap}>
      <div className={styles.badge}>Coming soon</div>
      <h2 className={styles.title}>Prospects</h2>
      <p className={styles.body}>
        Track and qualify the people you're pitching to. This is next.
      </p>
    </div>
  );
}
