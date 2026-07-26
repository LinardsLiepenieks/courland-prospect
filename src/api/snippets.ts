import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A snippet's lifecycle status.
 *  - `approved`: a normal snippet — editable, and used to compose drafts.
 *  - `proposed`: an AI-proposed snippet, extracted verbatim from a message you
 *    sent and awaiting your approve/reject. Shown in a distinct color; never used
 *    to compose a draft until approved. */
export type SnippetStatus = "approved" | "proposed";

/** A snippet: a named text fragment that composes into messages.
 *
 *  There is ONE library. Every draft sees all of it, and picking the lines that
 *  move a given customer profile toward its goal is the AI's job — which is why
 *  a snippet has no owner. `position` and `category` say *when* in a thread a
 *  line belongs, never *whom* it's for. */
export interface Snippet {
  id: number;
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
// these are the only entry points the UI uses to touch snippet data.

export function listSnippets(): Promise<Snippet[]> {
  return invoke("list_snippets");
}

/** Create a blank snippet. The card is filled in afterwards via `updateSnippet`. */
export function createSnippet(): Promise<Snippet> {
  return invoke("create_snippet");
}

/** Persist a snippet's name + content. */
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

/** Re-score and re-categorize every approved snippet through the AI — the
 *  "reorganize my whole library" action. A full reset: it overwrites hand-picked
 *  categories and hands each snippet back to auto-classification. Resolves when the
 *  whole batch finishes, with the number of snippets changed; the backend fires a
 *  single `snippets://changed` event at the end (not one per snippet), so OTHER open
 *  editors reconcile in one reshuffle — this caller reloads off its own resolution. */
export function reclassifySnippets(): Promise<number> {
  return invoke("reclassify_snippets");
}

/** Subscribe to backend "snippets changed" pushes — fired when a background pass
 *  changes the library (a new proposal lands, or a classify pass updates a
 *  snippet's position/category). No payload: there is one library, so there is
 *  nothing to scope the event to. */
export function onSnippetsChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("snippets://changed", () => cb());
}
