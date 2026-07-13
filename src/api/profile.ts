import { invoke } from "@tauri-apps/api/core";

/** The user's global profile context — "skills" the AI reasons about. A single
 *  app-wide record, not tied to any pitch. */
export interface Profile {
  /** Who the user is — background, role, voice. */
  who_are_you: string;
  /** What the user is building — the product, its shape and audience. */
  what_building: string;
  updated_at: string;
}

// Typed wrappers over the Rust commands. All SQL lives in the backend; these
// are the only entry points the UI uses to touch profile data.

export function getProfile(): Promise<Profile> {
  return invoke("get_profile");
}

export function updateProfile(
  whoAreYou: string,
  whatBuilding: string,
): Promise<Profile> {
  return invoke("update_profile", {
    whoAreYou,
    whatBuilding,
  });
}

/** Polish the "who are you" context via the local Claude Code CLI. Doesn't
 *  persist — the caller drops the result into the editor. */
export function polishWho(text: string): Promise<string> {
  return invoke("polish_who", { text });
}

/** Polish the "what are you building" context. See {@link polishWho}. */
export function polishBuilding(text: string): Promise<string> {
  return invoke("polish_building", { text });
}
