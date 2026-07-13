import { invoke } from "@tauri-apps/api/core";

/** A snippet: a named text fragment that will later compose into messages. Owned
 *  by exactly one place — a pitch (`pitch_id` set) or the global profile
 *  (`pitch_id` null). The origin (pitch vs profile) is always known from
 *  `pitch_id`. */
export interface Snippet {
  id: number;
  /** Owning pitch, or `null` when the snippet belongs to the global profile. */
  pitch_id: number | null;
  name: string;
  content: string;
  created_at: string;
}

// Typed wrappers over the Rust snippet commands. All SQL lives in the backend;
// these are the only entry points the UI uses to touch snippet data. `pitchId`
// is `null` for the global/profile scope and a pitch id for a pitch's snippets.

export function listSnippets(pitchId: number | null): Promise<Snippet[]> {
  return invoke("list_snippets", { pitchId });
}

/** Create a blank snippet in the given scope. The card is filled in afterwards
 *  via `updateSnippet`. */
export function createSnippet(pitchId: number | null): Promise<Snippet> {
  return invoke("create_snippet", { pitchId });
}

/** Persist a snippet's name + content. Ownership is fixed at creation. */
export function updateSnippet(
  id: number,
  name: string,
  content: string,
): Promise<Snippet> {
  return invoke("update_snippet", { id, name, content });
}

export function deleteSnippet(id: number): Promise<void> {
  return invoke("delete_snippet", { id });
}
