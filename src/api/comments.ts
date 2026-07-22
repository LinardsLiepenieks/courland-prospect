import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A drafted comment's lifecycle (mirrors the Rust `comment_drafts.status`):
 *  - `draft`:   generated, editable, not yet approved for posting.
 *  - `queued`:  approved via "Post all"; waiting for the extension to post it.
 *  - `posting`: the extension has claimed it and is submitting it now.
 *  - `posted`:  live on LinkedIn.
 *  - `failed`:  a post attempt failed (see `error`); the next "Post all" retries it. */
export type CommentStatus = "draft" | "queued" | "posting" | "posted" | "failed";

/** One drafted comment in the review inbox: a scraped post plus the comment the AI
 *  composed for it. Mirrors the Rust `CommentDraft`. */
export interface CommentDraft {
  id: number;
  /** The post's canonical permalink — its identity, and where the comment posts. */
  permalink: string;
  author_name: string;
  post_text: string;
  comment: string;
  status: CommentStatus;
  /** Last failure reason (empty unless `status` is `failed`). */
  error: string;
  created_at: string;
  posted_at: string | null;
}

/** The commenter control record (mirrors the Rust `CommentRun`). `status`:
 *  `idle` (nothing running), `requested` (asked the extension to scrape), or
 *  `scraping` (the extension is scraping now). */
export interface CommentRun {
  status: "idle" | "requested" | "scraping";
  count: number;
  include_watchlist: boolean;
  updated_at: string;
}

// Typed wrappers over the Rust comment commands. All SQL lives in the backend;
// these are the only entry points the Comments tab uses. The extension drives the
// other half of the flow over the loopback ingest server.

export function listCommentDrafts(): Promise<CommentDraft[]> {
  return invoke("list_comment_drafts");
}

export function getCommentRun(): Promise<CommentRun> {
  return invoke("get_comment_run");
}

/** Ask the extension to run a scrape: records the budget + watchlist choice and
 *  flips the run to `requested`. Returns the updated run. */
export function requestCommentScrape(
  count: number,
  includeWatchlist: boolean,
): Promise<CommentRun> {
  return invoke("request_comment_scrape", { count, includeWatchlist });
}

/** Save an edited comment. Rejected (throws) once the draft is posting/posted. */
export function updateCommentDraft(id: number, comment: string): Promise<CommentDraft> {
  return invoke("update_comment_draft", { id, comment });
}

export function deleteCommentDraft(id: number): Promise<void> {
  return invoke("delete_comment_draft", { id });
}

/** The "Post all" action: queue every editable, non-empty draft for the extension
 *  to post. Resolves with how many were queued. */
export function queueCommentDrafts(): Promise<number> {
  return invoke("queue_comment_drafts");
}

/** Subscribe to backend "comments changed" pushes — fired whenever the inbox or the
 *  run state changes, from a command (the app) or from the extension over the
 *  ingest server. A listener re-fetches so both halves of the flow stay reflected
 *  live without polling. */
export function onCommentsChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("comments://changed", () => cb());
}
