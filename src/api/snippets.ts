import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A snippet's lifecycle status.
 *  - `approved`: a normal snippet — editable, and used to compose drafts.
 *  - `proposed`: an AI-proposed snippet, extracted verbatim from a message you
 *    sent and awaiting your approve/reject. Shown in a distinct color; never used
 *    to compose a draft until approved. */
export type SnippetStatus = "approved" | "proposed";

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
  status: SnippetStatus;
  /** Where on the conversation arc this snippet sits: 0 (opener) → 1 (closing ask).
   *  AI-derived; the editor's primary sort. 0.5 until classified. */
  position: number;
  /** A reusable group label many snippets share (empty = uncategorized). */
  category: string;
  /** True when the user hand-picked the category; the AI won't re-classify it. */
  manual: boolean;
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

/** Approve an AI-proposed snippet — it becomes a normal, editable snippet that
 *  composes drafts. Rejecting a proposal is just `deleteSnippet` (a rejected
 *  proposal has no value to keep). Returns the snippet in its approved state. */
export function approveSnippet(id: number): Promise<Snippet> {
  return invoke("approve_snippet", { id });
}

/** Set a snippet's category by hand. A non-empty category pins the snippet so the
 *  AI won't re-categorize it; passing an empty string clears the category and
 *  re-enables auto-classification. Returns the updated snippet. */
export function setSnippetCategory(
  id: number,
  category: string,
): Promise<Snippet> {
  return invoke("set_snippet_category", { id, category });
}

/** Copy a snippet into another scope as an independent duplicate. `targetPitchId`
 *  is a pitch id, or `null` for the global profile. The new snippet carries only the
 *  source's name + content (it re-classifies in its new scope); the source is left
 *  untouched. Returns the newly created snippet. */
export function copySnippet(
  id: number,
  targetPitchId: number | null,
): Promise<Snippet> {
  return invoke("copy_snippet", { id, targetPitchId });
}

/** Subscribe to backend "snippets changed" pushes — fired when a background pass
 *  changes a scope's snippets (a new proposal lands, or a classify pass updates a
 *  snippet's position/category). The payload is the affected scope: a pitch id, or
 *  `null` for the global profile — so a listener reloads only when its own scope
 *  changed. */
export function onSnippetsChanged(
  cb: (scope: number | null) => void,
): Promise<UnlistenFn> {
  return listen<number | null>("snippets://changed", (e) => cb(e.payload));
}
