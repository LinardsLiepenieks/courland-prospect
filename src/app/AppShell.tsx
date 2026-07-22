import Tabs, { type TabItem } from "../components/Tabs";
import type { Pitch } from "../api/pitches";
import PitchSwitcher from "../pitches/PitchSwitcher";
import PitchDetail from "../pitches/PitchDetail";
import EditPitchView from "../pitches/EditPitchView";
import ProspectsView from "../prospects/ProspectsView";
import ProfileView from "../profile/ProfileView";
import CommentsView from "../comments/CommentsView";
import styles from "./app.module.css";

export type TabId = "pitch" | "prospects" | "settings" | "comments" | "profile";

// Pitch-scoped views sit on the left; Comments + Profile are global (not tied to a
// pitch), so they're pushed to the far right of the nav to read as distinct.
const LEFT_TABS: TabItem<TabId>[] = [
  { id: "pitch", label: "Pitch" },
  { id: "prospects", label: "Prospects" },
  { id: "settings", label: "Settings" },
];

const RIGHT_TABS: TabItem<TabId>[] = [
  { id: "comments", label: "Comments" },
  { id: "profile", label: "Profile" },
];

interface Props {
  pitches: Pitch[];
  activePitch: Pitch | null;
  activeTab: TabId;
  onSelectPitch: (id: number) => void;
  onChangeTab: (id: TabId) => void;
  onCreateNew: () => void;
  onSavePitch: (id: number, name: string, skill: string) => Promise<void>;
  onDeletePitch: (id: number) => Promise<void>;
}

/** The main app surface: navbar (pitch switcher beside the underline tabs) and
 *  the content of the active tab, scoped to the selected pitch. */
export default function AppShell({
  pitches,
  activePitch,
  activeTab,
  onSelectPitch,
  onChangeTab,
  onCreateNew,
  onSavePitch,
  onDeletePitch,
}: Props) {
  return (
    <div className={styles.app}>
      <header className={styles.navbar}>
        <div className={styles.navInner}>
          <PitchSwitcher
            pitches={pitches}
            activeId={activePitch?.id ?? null}
            onSelect={onSelectPitch}
            onCreateNew={onCreateNew}
          />
          <span className={styles.navDivider} aria-hidden="true" />
          <Tabs items={LEFT_TABS} active={activeTab} onChange={onChangeTab} />
          <span className={styles.navSpacer} aria-hidden="true" />
          <Tabs items={RIGHT_TABS} active={activeTab} onChange={onChangeTab} />
        </div>
      </header>

      <main className={styles.content}>
        {activeTab === "pitch" && (
          <PitchDetail pitch={activePitch} onCreateNew={onCreateNew} />
        )}
        {activeTab === "prospects" && (
          <ProspectsView pitch={activePitch} onCreateNew={onCreateNew} />
        )}
        {activeTab === "comments" && <CommentsView />}
        {activeTab === "profile" && <ProfileView />}
        {activeTab === "settings" && (
          <EditPitchView
            pitch={activePitch}
            onSave={onSavePitch}
            onDelete={onDeletePitch}
            onCreateNew={onCreateNew}
          />
        )}
      </main>
    </div>
  );
}
