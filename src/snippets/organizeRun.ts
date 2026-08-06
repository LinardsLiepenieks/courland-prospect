/**
 * The "Organize library" run, held outside React.
 *
 * `SnippetsView` is mounted conditionally on the active tab (see `app/App.tsx`), so
 * switching tabs unmounts it. That made component state the wrong home for this
 * particular action, in two ways that both bit:
 *
 *   - **The result was thrown away.** The run takes two LLM passes, each capable of
 *     running to the CLI's 60s ceiling, so switching away mid-run is the natural thing
 *     to do. Every `setState` then landed on a dead component and the finished
 *     redundancy report vanished — and because re-running is the only way to get it
 *     back, recovering meant re-running a pass that overwrites hand-picked categories.
 *   - **The re-entry guard was lost.** `useAsyncAction`'s guard is a ref on the
 *     component instance, so a remount produced a fresh one: the button re-enabled with
 *     a run still in flight, and a second click started a second concurrent
 *     `reclassify_all` — two batches interleaving force-writes over the same rows, each
 *     accumulating its own stage-label set, and whichever redundancy pass finished last
 *     winning, possibly describing the library as it was before the other re-score.
 *
 * So the run lives here instead: module scope, one at a time, surviving unmount. The
 * view subscribes and renders whatever this says. Still deliberately not persisted to
 * disk — a redundancy report describes the library as it is right now, and a restart
 * should start clean.
 */

import {
  findRedundantSnippets,
  reclassifySnippets,
  type RedundancyGroup,
} from "../api/snippets";
import { errorMessage } from "../lib/errors";

/** Which half of the run is in flight, for the button's label. */
export type OrganizePhase = null | "rescoring" | "checking";

export interface OrganizeState {
  phase: OrganizePhase;
  /** Outcome line for a finished run, or null before the first one. */
  note: string | null;
  /** The redundancy report, or null when there's nothing to review. */
  groups: RedundancyGroup[] | null;
  /** Which snippets are ticked for deletion, per group, keyed on the group's `keep_id`
   *  (unique across groups by the backend's one-group-per-snippet rule).
   *
   *  These live here rather than in the group card for the same reason the report does.
   *  The card seeded its ticks from `keep_id` in a `useState` initializer, so a tab switch
   *  — the thing this whole module exists to survive — remounted every card and silently
   *  reset each one to the model's pre-ticked default. Anyone who had gone through the
   *  report unticking the keeper the AI got wrong then had those choices reverted with no
   *  indication, and the next "Delete" click removed the very versions they'd chosen to
   *  keep. The report was durable and the decisions taken against it were not. */
  selections: Record<number, number[]>;
  /** A failure from the re-score half (the half that can't degrade). */
  error: string | null;
  /** Bumped whenever the library may have changed, so a subscriber knows to reload —
   *  including a subscriber that mounted after the change happened. */
  libraryVersion: number;
  /** The `libraryVersion` at which a re-score last re-homed snippets across stages, or 0
   *  if none has this session.
   *
   *  The view collapses stage sections by default, so a finished re-score needs to expand
   *  them or the reorganized library reads as empty. That was a component flag, which is
   *  gone on remount — so switching away mid-run and coming back showed a wall of
   *  collapsed headers and a note claiming success, i.e. exactly the "looks like it did
   *  nothing" outcome the flag existed to prevent, reached by exactly the action this
   *  module exists to survive. As a version rather than a boolean it also can't be
   *  consumed twice or missed by a mount that arrived late. */
  restagedAt: number;
}

let state: OrganizeState = {
  phase: null,
  note: null,
  groups: null,
  selections: {},
  error: null,
  libraryVersion: 0,
  restagedAt: 0,
};

const listeners = new Set<() => void>();

/** Guards against a second concurrent run. Module-scoped, so unlike a component ref it
 *  survives the unmount that a tab switch causes. */
let inFlight = false;

function emit(next: Partial<OrganizeState>) {
  state = { ...state, ...next };
  for (const listener of listeners) listener();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** `useSyncExternalStore` requires a stable snapshot — returning the same object until
 *  something actually changes is what stops it looping. */
export function getSnapshot(): OrganizeState {
  return state;
}

/**
 * Re-score the library, then search it for redundancy. Resolves when both are done; a
 * second call while one is running is dropped rather than queued.
 *
 * Re-score first, deliberately: it re-homes snippets across stages, so the redundancy
 * pass reads the library in its final shape. `libraryVersion` is bumped between the two
 * so the board shows the re-staged library while the slower second pass runs.
 *
 * **The two halves fail independently.** Each gets its own try/catch, and the redundancy
 * pass runs whatever the re-score did. It used to sit inside the re-score's `try`, so a
 * failed re-score skipped it entirely — which is backwards twice over: the redundancy pass
 * is read-only, writes nothing, and would have worked, and the whole reason these are two
 * commands sequenced here rather than one is so that either can fail alone. A re-score can
 * fail for reasons that say nothing about the library's redundancy, so losing both is a
 * strictly worse answer than losing one.
 */
export async function start(): Promise<void> {
  if (inFlight) return;
  inFlight = true;
  emit({ phase: "rescoring", note: null, groups: null, selections: {}, error: null });

  let note: string | null = null;
  try {
    const changed = await reclassifySnippets();
    note =
      changed > 0
        ? `Re-scored ${changed} snippet${changed === 1 ? "" : "s"}.`
        : "Everything was already up to date.";
    // Stages moved, so tell a subscriber (including one that mounts later) to expand them.
    const libraryVersion = state.libraryVersion + 1;
    emit({ phase: "checking", libraryVersion, restagedAt: libraryVersion });
  } catch (err) {
    emit({ phase: "checking", error: errorMessage(err) });
  }

  try {
    const found = await findRedundantSnippets();
    emit({ groups: found.length > 0 ? found : null });
    if (found.length === 0 && note) note += " Nothing looks redundant.";
  } catch {
    note = note
      ? `${note} Couldn't check for redundancy.`
      : // The re-score already failed, so say both plainly rather than stacking two
        // sentences that each blame something different.
        null;
  }

  if (note) emit({ note });
  inFlight = false;
  emit({ phase: null, libraryVersion: state.libraryVersion + 1 });
}

/** The ticks a group starts with: the model's suggestion, i.e. everything except the
 *  keeper. Used when `selections` has no entry for the group yet, so an untouched group
 *  needs no state at all and only a real choice is recorded. */
export function defaultTicks(group: RedundancyGroup): number[] {
  return group.members.filter((m) => m.id !== group.keep_id).map((m) => m.id);
}

/** Record the ticks for one group, so they survive the unmount a tab switch causes. */
export function setSelection(keepId: number, ids: number[]) {
  emit({ selections: { ...state.selections, [keepId]: ids } });
}

/** Retire one group from the report — its picks were deleted, or the user waved it away.
 *  Keyed on `keep_id`, unique across groups by the backend's one-group-per-snippet rule.
 *  Emptying the report clears it, so no bare heading is left behind. */
export function resolveGroup(keepId: number) {
  const next = (state.groups ?? []).filter((g) => g.keep_id !== keepId);
  const { [keepId]: _gone, ...selections } = state.selections;
  emit({ groups: next.length > 0 ? next : null, selections });
}

export function dismissGroups() {
  emit({ groups: null, selections: {} });
}

/** Dismiss the outcome line. Module state outlives the view, so without this a note or
 *  error sat at the top of the tab for the rest of the session, greeting every visit and
 *  attached to nothing the user had just done — and unlike the pre-store version, leaving
 *  the tab no longer cleared it. */
export function dismissNote() {
  emit({ note: null, error: null });
}

/** Announce that the library changed outside a run (a delete from a card, an add), so
 *  subscribers reload — and drop an outcome line that no longer describes the library. */
export function noteLibraryChanged() {
  emit({ libraryVersion: state.libraryVersion + 1, note: null, error: null });
}
