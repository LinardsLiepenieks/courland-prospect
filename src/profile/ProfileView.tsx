import { FormEvent, useState } from "react";
import { getProfile, polishWho, updateProfile, type Profile } from "../api/profile";
import LoadError from "../components/LoadError";
import PolishButton from "../components/PolishButton";
import SavedIndicator from "../components/SavedIndicator";
import { useAutosave } from "../lib/useAutosave";
import { useLoad } from "../lib/useLoad";
import ProfileDropdown from "./ProfileDropdown";
import styles from "./ProfileView.module.css";

/** Profile tab: who *you* are, and which browser Courland captures from.
 *
 *  Deliberately narrow. The product story lives on the Product tab and the
 *  snippet library on its own — what's left here is the person doing the selling,
 *  which is exactly what the commenter borrows to sound like a peer rather than a
 *  vendor. */
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
          Your background, role, and voice — the person every message is written
          as. What you sell lives under <strong>Product</strong>.
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
    </div>
  );
}

/** The "about you" context — a singleton record loaded once on mount. */
function AboutYou() {
  const { data: profile, error: loadError, retry } = useLoad(getProfile);

  if (loadError) {
    return (
      <LoadError what="your profile context" detail={loadError} onRetry={retry} />
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
  // A polish is in flight. Locks the field (and holds autosave) so the incoming
  // rewrite can't clobber text typed during the multi-second CLI call.
  const [polishing, setPolishing] = useState(false);

  const { saving, showSaved, dirty, error, setError, save } = useAutosave({
    values: { who: whoAreYou },
    persist: (v) => updateProfile(v.who),
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
