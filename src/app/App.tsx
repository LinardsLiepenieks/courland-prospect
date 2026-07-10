import { useEffect, useState } from "react";
import {
  createPitch,
  deletePitch,
  listPitches,
  updatePitch,
  type Pitch,
} from "../api/pitches";
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

  useEffect(() => {
    // `active` short-circuits a resolve after unmount (incl. StrictMode's
    // double-mount) and pairs with the `.catch` so a failed load can't become
    // an unhandled rejection.
    let active = true;
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
      // TODO: once the shell has an error surface, distinguish a load failure
      // from a genuinely empty DB here rather than only logging.
      .catch((e) => {
        if (active) console.error("Failed to load pitches", e);
      });
    return () => {
      active = false;
    };
  }, []);

  const activePitch = pitches.find((p) => p.id === activePitchId) ?? null;

  async function handleCreate(name: string, skill: string) {
    const created = await createPitch(name, skill);
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
