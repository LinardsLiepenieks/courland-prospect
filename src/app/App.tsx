import { useEffect, useState } from "react";
import {
  createPitch,
  deletePitch,
  listPitches,
  updatePitch,
  type Pitch,
} from "../api/pitches";
import type { StageInput } from "../api/stages";
import LoadError from "../components/LoadError";
import { errorMessage } from "../lib/errors";
import CreatePitchView from "../pitches/CreatePitchView";
import AppShell, { type TabId } from "./AppShell";

type Mode = "shell" | "create";

/**
 * Top-level state: which pitch is the active context, which tab is showing,
 * and whether we're in the full-screen create flow. The dropdown selects the
 * active pitch; both tabs are views scoped to it.
 */
export default function App() {
  const [pitches, setPitches] = useState<Pitch[]>([]);
  const [activePitchId, setActivePitchId] = useState<number | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("pitch");
  const [mode, setMode] = useState<Mode>("shell");
  const [loadError, setLoadError] = useState<string | null>(null);
  // Bumped by the retry button to re-run the load effect after a failure.
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    // `active` short-circuits a resolve after unmount (incl. StrictMode's
    // double-mount) and pairs with the `.catch` so a failed load can't become
    // an unhandled rejection.
    let active = true;
    setLoadError(null);
    listPitches()
      .then((loaded) => {
        if (!active) return;
        // Only seed when state is still pristine, so a slow load can never
        // clobber a pitch the user created while it was in flight. In practice
        // the local-SQLite list resolves at startup before any create, so this
        // is defense-in-depth rather than a reachable race.
        setPitches((prev) => (prev.length > 0 ? prev : loaded));
        setActivePitchId((prev) => prev ?? loaded[0]?.id ?? null);
      })
      // A failed load surfaces a recoverable error screen (with retry) instead
      // of silently leaving an empty shell that looks like a fresh, blank DB.
      .catch((e) => {
        if (active) setLoadError(errorMessage(e));
      });
    return () => {
      active = false;
    };
  }, [reloadKey]);

  const activePitch = pitches.find((p) => p.id === activePitchId) ?? null;

  async function handleCreate(
    name: string,
    skill: string,
    stages: StageInput[],
  ) {
    const created = await createPitch(name, skill, stages);
    setPitches((prev) => [created, ...prev]);
    setActivePitchId(created.id);
    setActiveTab("pitch");
    setMode("shell");
  }

  async function handleUpdate(id: number, name: string, skill: string) {
    const updated = await updatePitch(id, name, skill);
    setPitches((prev) => prev.map((p) => (p.id === id ? updated : p)));
  }

  async function handleDelete(id: number) {
    await deletePitch(id);
    // Functional update so a pitch created during the await (via the switcher's
    // create flow) isn't clobbered. `remaining` from the render closure is only
    // read to reselect when the deleted pitch was still active — a state any
    // concurrent create would have moved us off of — so it's accurate there.
    const remaining = pitches.filter((p) => p.id !== id);
    setPitches((prev) => prev.filter((p) => p.id !== id));
    setActivePitchId((cur) => (cur === id ? (remaining[0]?.id ?? null) : cur));
    setActiveTab("pitch");
  }

  if (loadError) {
    return (
      <LoadError
        what="your pitches"
        detail={loadError}
        onRetry={() => setReloadKey((k) => k + 1)}
      />
    );
  }

  if (mode === "create") {
    return (
      <CreatePitchView
        onCreate={handleCreate}
        onCancel={() => setMode("shell")}
      />
    );
  }

  return (
    <AppShell
      pitches={pitches}
      activePitch={activePitch}
      activeTab={activeTab}
      onSelectPitch={setActivePitchId}
      onChangeTab={setActiveTab}
      onCreateNew={() => setMode("create")}
      onSavePitch={handleUpdate}
      onDeletePitch={handleDelete}
    />
  );
}
