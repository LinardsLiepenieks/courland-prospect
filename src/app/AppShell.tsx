import Tabs, { type TabItem } from "../components/Tabs";
import type { Pitch } from "../api/pitches";
import PitchSwitcher from "../pitches/PitchSwitcher";
import PitchDetail from "../pitches/PitchDetail";
import EditPitchView from "../pitches/EditPitchView";
import ProspectsView from "../prospects/ProspectsView";
import styles from "./app.module.css";

export type TabId = "pitch" | "prospects" | "settings";

const TABS: TabItem<TabId>[] = [
  { id: "pitch", label: "Pitch" },
  { id: "prospects", label: "Prospects" },
  { id: "settings", label: "Settings" },
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
          <Tabs items={TABS} active={activeTab} onChange={onChangeTab} />
        </div>
      </header>

      <main className={styles.content}>
        {activeTab === "pitch" && (
          <PitchDetail pitch={activePitch} onCreateNew={onCreateNew} />
        )}
        {activeTab === "prospects" && <ProspectsView />}
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
