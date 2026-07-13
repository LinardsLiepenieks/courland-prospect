import { FormEvent, useEffect, useState } from "react";
import {
  getProfile,
  polishBuilding,
  polishWho,
  updateProfile,
  type Profile,
} from "../api/profile";
import LoadError from "../components/LoadError";
import PolishButton from "../components/PolishButton";
import SavedIndicator from "../components/SavedIndicator";
import SnippetsSection from "../components/SnippetsSection";
import { errorMessage } from "../lib/errors";
import { useAutosave } from "../lib/useAutosave";
import ProfileDropdown from "./ProfileDropdown";
import styles from "./ProfileView.module.css";

/** Profile tab: two sections — the user's global "about you" context (who they
 *  are / what they're building, app-wide reference the AI reasons about) and the
 *  capture browser profiles the dedicated Chrome launches into. Each section
 *  loads its own data, so one can't block the other. */
export default function ProfileView() {
  return (
    <div className={styles.profile}>
      <header className={styles.intro}>
        <h1 className={styles.title}>Profile</h1>
        <p className={styles.subtitle}>
          How the AI sees you, and which browser Courland captures from.
        </p>
      </header>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>About you</h2>
        <p className={styles.sectionSub}>
          Reference notes the AI uses to think about you — global, not tied to
          any pitch.
        </p>
        <AboutYou />
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Capture browser</h2>
        <p className={styles.sectionSub}>
          Open your Chrome to capture from LinkedIn — pick a profile to open its
          window. The Courland extension must be enabled in whichever profile you
          use.
        </p>
        <ProfileDropdown />
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Snippets</h2>
        <p className={styles.sectionSub}>
          Reusable text fragments you'll draw on when writing messages — global,
          not tied to any pitch.
        </p>
        <SnippetsSection pitchId={null} />
      </section>
    </div>
  );
}

/** The "about you" context — a singleton record loaded once on mount. */
function AboutYou() {
  const [profile, setProfile] = useState<Profile | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load after a failure.
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    // `active` short-circuits a resolve after unmount (incl. StrictMode's
    // double-mount) and pairs with `.catch` so a failed load can't become an
    // unhandled rejection.
    let active = true;
    setLoadError(null);
    getProfile()
      .then((p) => active && setProfile(p))
      .catch((e) => active && setLoadError(errorMessage(e)));
    return () => {
      active = false;
    };
  }, [reloadKey]);

  if (loadError) {
    return (
      <LoadError
        what="your profile context"
        detail={loadError}
        onRetry={() => setReloadKey((k) => k + 1)}
      />
    );
  }
  if (!profile) {
    // Local SQLite resolves near-instantly; this reserves layout without a
    // flash of spinner for the common fast path.
    return <div className={styles.loading} aria-busy="true" aria-hidden="true" />;
  }
  return <AboutYouForm initial={profile} />;
}

function AboutYouForm({ initial }: { initial: Profile }) {
  const [whoAreYou, setWhoAreYou] = useState(initial.who_are_you);
  const [whatBuilding, setWhatBuilding] = useState(initial.what_building);
  // A polish is in flight. Locks the fields (and holds autosave) so the incoming
  // rewrite can't clobber text typed during the multi-second CLI call.
  const [polishing, setPolishing] = useState(false);

  const { saving, showSaved, dirty, error, setError, save } = useAutosave({
    values: { who: whoAreYou, what: whatBuilding },
    persist: (v) => updateProfile(v.who, v.what),
    hold: polishing,
  });

  // Manual save flushes immediately, skipping the debounce.
  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    save();
  }

  return (
    <form className={styles.form} onSubmit={handleSubmit}>
      <div className={styles.fieldHeader}>
        <label className={styles.fieldLabel} htmlFor="profile-who">
          Who are you?
        </label>
        <PolishButton
          text={whoAreYou}
          polish={polishWho}
          disabled={saving || polishing}
          onPolished={setWhoAreYou}
          onError={setError}
          onBusyChange={setPolishing}
        />
      </div>
      <textarea
        id="profile-who"
        className={styles.textarea}
        placeholder="Your background, role, and voice — how you'd describe yourself."
        value={whoAreYou}
        onChange={(e) => setWhoAreYou(e.target.value)}
        disabled={polishing}
      />

      <div className={styles.fieldHeader}>
        <label className={styles.fieldLabel} htmlFor="profile-building">
          What are you building?
        </label>
        <PolishButton
          text={whatBuilding}
          polish={polishBuilding}
          disabled={saving || polishing}
          onPolished={setWhatBuilding}
          onError={setError}
          onBusyChange={setPolishing}
        />
      </div>
      <textarea
        id="profile-building"
        className={styles.textarea}
        placeholder="The product you're building — what it is, who it's for, why it matters."
        value={whatBuilding}
        onChange={(e) => setWhatBuilding(e.target.value)}
        disabled={polishing}
      />

      {error && <div className={styles.error}>{error}</div>}

      <div className={styles.actions}>
        <SavedIndicator visible={showSaved} />
        <button
          type="submit"
          className={styles.primaryBtn}
          disabled={!dirty || saving || polishing}
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>
    </form>
  );
}
