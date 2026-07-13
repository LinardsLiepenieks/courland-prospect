import { invoke } from "@tauri-apps/api/core";
import type { StageInput } from "./stages";

/** A pitch: a distinct thing you're selling, that prospects attach to. */
export interface Pitch {
  id: number;
  name: string;
  /** What the pitch is about — the skill/angle you're selling. */
  skill: string;
  created_at: string;
}

// Typed wrappers over the Rust commands. All SQL lives in the backend; these
// are the only entry points the UI uses to touch pitch data.

export function listPitches(): Promise<Pitch[]> {
  return invoke("list_pitches");
}

/** Create a pitch and seed its pipeline. `stages` is the funnel set up in the
 *  create flow (empty falls back to the backend's Full-cycle template). */
export function createPitch(
  name: string,
  skill: string,
  stages: StageInput[],
): Promise<Pitch> {
  return invoke("create_pitch", { name, skill, stages });
}

export function updatePitch(
  id: number,
  name: string,
  skill: string,
): Promise<Pitch> {
  return invoke("update_pitch", { id, name, skill });
}

export function deletePitch(id: number): Promise<void> {
  return invoke("delete_pitch", { id });
}

/** Polish the skill text through the local Claude Code CLI, returning the
 *  rewritten version. Doesn't persist — the caller drops the result into the
 *  editor for the user to review and save. */
export function polishSkill(text: string): Promise<string> {
  return invoke("polish_skill", { text });
}
