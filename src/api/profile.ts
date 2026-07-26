import { invoke } from "@tauri-apps/api/core";

/** Who the *user* is — background, role, voice. A single app-wide record.
 *
 *  Deliberately narrow: the product story lives in {@link ./product}. The person
 *  writing and the thing being sold are different jobs, and keeping them apart is
 *  what lets a public comment borrow the founder's persona without reading as
 *  pitch copy. */
export interface Profile {
  who_are_you: string;
  updated_at: string;
}

// Typed wrappers over the Rust commands. All SQL lives in the backend; these
// are the only entry points the UI uses to touch profile data.

export function getProfile(): Promise<Profile> {
  return invoke("get_profile");
}

export function updateProfile(whoAreYou: string): Promise<Profile> {
  return invoke("update_profile", { whoAreYou });
}

/** Polish the "who are you" context via the local Claude Code CLI. Doesn't
 *  persist — the caller drops the result into the editor. */
export function polishWho(text: string): Promise<string> {
  return invoke("polish_who", { text });
}
