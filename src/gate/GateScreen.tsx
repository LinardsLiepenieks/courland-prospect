import { useEffect, useState } from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { extensionDir, type GateStatus } from "../api/gate";
import ProfileDropdown from "../profile/ProfileDropdown";
import styles from "./gate.module.css";

/**
 * The hard gate. Shown whenever the app isn't `ready`: waiting on the extension
 * to check in, Chrome closed, the extension not loaded, or an error. Readiness
 * is driven by the extension's heartbeat, so these screens recover on their own
 * once it starts checking in — no manual "re-check" needed.
 */
export default function GateScreen({ status }: { status: GateStatus }) {
  if (status.state === "initializing") {
    return (
      <Shell>
        <span className={styles.spinner} aria-hidden="true" />
        <h1 className={styles.title}>Connecting…</h1>
        <p className={styles.body}>Looking for your Chrome and the capture extension.</p>
      </Shell>
    );
  }

  if (status.state === "extensionMissing") {
    return <ExtensionMissing />;
  }

  if (status.state === "error") {
    return (
      <Shell>
        <h1 className={styles.title}>Something went wrong</h1>
        <p className={styles.body}>{status.detail}</p>
      </Shell>
    );
  }

  // chromeClosed
  return (
    <Shell>
      <h1 className={styles.title}>Chrome is closed</h1>
      <p className={styles.body}>
        Courland captures prospects from LinkedIn in your Chrome. Pick a profile
        and open it — with the Courland extension enabled — to continue.
      </p>
      <div className={styles.picker}>
        <span className={styles.pickerLabel}>Profile</span>
        <ProfileDropdown primary openLabel="Open Chrome" />
      </div>
    </Shell>
  );
}

/** Chrome is running but the extension isn't checking in — walk through loading
 *  it. Recovers automatically: once the extension is enabled its heartbeat
 *  arrives and the gate advances to ready, so there's no button to press. */
function ExtensionMissing() {
  const [dir, setDir] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    extensionDir()
      .then((d) => active && setDir(d))
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  return (
    <Shell wide>
      <h1 className={styles.title}>Load the capture extension</h1>
      <p className={styles.body}>
        Chrome is open but the Courland extension isn't checking in. Make sure
        it's enabled in the Chrome profile you opened. First time? Load it — in
        that Chrome window:
      </p>
      <ol className={styles.steps}>
        <li>
          Open <code className={styles.code}>chrome://extensions</code>
        </li>
        <li>
          Turn on <strong>Developer mode</strong> (top-right)
        </li>
        <li>
          Click <strong>Load unpacked</strong> and choose this folder:
          <div className={styles.pathRow}>
            <code className={styles.path}>{dir ?? "…"}</code>
            <button
              className={styles.ghost}
              onClick={() => dir && void openPath(dir)}
              disabled={!dir}
            >
              Reveal
            </button>
          </div>
        </li>
      </ol>
      <p className={styles.waiting}>
        <span className={styles.spinnerSmall} aria-hidden="true" />
        Waiting for the extension — this continues on its own once it's enabled.
      </p>
    </Shell>
  );
}

function Shell({ children, wide }: { children: React.ReactNode; wide?: boolean }) {
  return (
    <div className={styles.screen}>
      <div className={`${styles.card} ${wide ? styles.cardWide : ""}`}>{children}</div>
    </div>
  );
}
