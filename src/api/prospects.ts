import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A prospect: a person captured from LinkedIn into the pipeline. */
export interface Prospect {
  id: number;
  name: string;
  /** The LinkedIn profile URL — the natural identity/dedup key. */
  linkedin_url: string;
  /** Their headline/title as scraped, if any. */
  headline: string;
  /** The customer profile they match — what steers their drafts toward a goal.
   *  `null` when unassigned, which is a valid resting state: they're still in
   *  the pipeline, their drafts just get no goal. Deleting a customer profile
   *  leaves its prospects here, unassigned. */
  customer_id: number | null;
  /** The pipeline stage they're currently in. `null` only if their stage was
   *  deleted out from under them. */
  stage_id: number | null;
  /** Outreach counter shown in the messaging stage. Read-only: derived from
   *  messages the Chrome extension captures, not set by hand. */
  messages_sent: number;
  /** Whether this prospect has replied and we still owe them an answer — i.e.
   *  their newest captured message is incoming. Dynamic and derived from messages
   *  the Chrome extension captures: a reply at any stage sets it, and our answer
   *  clears it. Drives the "Awaiting reply" treatment on their card. */
  awaiting_reply: boolean;
  /** When you last sent this prospect a message (SQLite `YYYY-MM-DD HH:MM:SS`,
   *  UTC). Read-only: derived from captured messages alongside `messages_sent`.
   *  `null` means you've never messaged them — `staleness.ts` ages those from
   *  `created_at` instead, so a captured-but-never-contacted prospect still goes
   *  stale rather than sitting fresh forever. */
  last_outreach_at: string | null;
  /** The stage the advance analyzer thinks this prospect has outgrown into,
   *  pending your accept/dismiss. `null` when there's no open suggestion — the
   *  resting state, since suggestions are never applied on their own. */
  suggested_stage_id: number | null;
  /** The analyzer's one-line justification, shown on the card so the suggestion
   *  can be judged without reopening the thread. Empty when there's none. */
  suggested_reason: string;
  note: string;
  created_at: string;
}

// Reads for the desktop Prospects tab. Prospects are *written* by the Chrome
// extension over the loopback ingest server, not through a command — so there's
// no create/update wrapper here by design.

export function listProspects(): Promise<Prospect[]> {
  return invoke("list_prospects");
}

// Prospects are deleted from the desktop app (unlike create/update, which the
// Chrome extension owns over the ingest server). Permanently removes the row.
export function deleteProspect(id: number): Promise<void> {
  return invoke("delete_prospect", { id });
}

/** Move a prospect to a different stage of the pipeline (drag or stage menu).
 *  Returns the updated prospect. */
export function setProspectStage(
  id: number,
  stageId: number,
): Promise<Prospect> {
  return invoke("set_prospect_stage", { id, stageId });
}

/** Re-tag which customer profile a prospect matches, or clear it (`null`). Only
 *  changes how their drafts are steered — never moves them on the board — so the
 *  UI offers it inline with no confirmation. Returns the updated prospect. */
export function setProspectCustomer(
  id: number,
  customerId: number | null,
): Promise<Prospect> {
  return invoke("set_prospect_customer", { id, customerId });
}

/** Take the analyzer's pending suggestion: move the prospect to the stage it
 *  points at. Deliberately a command rather than a client-side
 *  `setProspectStage(suggested_stage_id)` — the backend re-reads the suggestion
 *  under the same lock it applies it with, so a suggestion superseded between
 *  render and click can't move someone somewhere the app no longer believes in.
 *  Returns the updated prospect (with the suggestion cleared). */
export function acceptStageSuggestion(id: number): Promise<Prospect> {
  return invoke("accept_stage_suggestion", { id });
}

/** Wave off the pending suggestion without moving anyone. Not remembered: the
 *  next message in the thread is new evidence and may raise it again. */
export function dismissStageSuggestion(id: number): Promise<Prospect> {
  return invoke("dismiss_stage_suggestion", { id });
}

/** Re-run the advance check for one prospect now. The analyzer fires on its own
 *  whenever a message lands, so this is for when nothing new will arrive to
 *  trigger it — you moved someone by hand, or you just wrote a stage's goal and
 *  want it applied to the threads already sitting there. Slow (it shells out to
 *  Claude Code); a verdict of "not yet" returns the row unchanged. */
export function recheckProspectStage(id: number): Promise<Prospect> {
  return invoke("recheck_prospect_stage", { id });
}

/** Subscribe to backend "prospects changed" pushes — fired when the Chrome
 *  extension captures a prospect or a sent message bumps a derived count — so an
 *  open view can re-fetch and update live. */
export function onProspectsChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("prospects://changed", () => cb());
}
