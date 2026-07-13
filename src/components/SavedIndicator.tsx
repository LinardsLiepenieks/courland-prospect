import styles from "../styles/form.module.css";

interface Props {
  visible: boolean;
}

/** Quiet "Saved ✓" confirmation shared by editing surfaces. Stays mounted and
 *  fades via `data-visible` so it can animate out rather than pop away. */
export default function SavedIndicator({ visible }: Props) {
  return (
    <span className={styles.saved} data-visible={visible} aria-hidden={!visible}>
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
  );
}
