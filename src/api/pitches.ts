import { invoke } from "@tauri-apps/api/core";

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

export function createPitch(name: string, skill: string): Promise<Pitch> {
  return invoke("create_pitch", { name, skill });
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
