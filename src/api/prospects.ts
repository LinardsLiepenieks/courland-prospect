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
  /** The pitch being run on them. `null` if that pitch was later deleted. */
  pitch_id: number | null;
  /** The pipeline stage they're currently in. `null` if unassigned. */
  stage_id: number | null;
  /** Outreach counter shown in the messaging stage. Read-only: derived from
   *  messages the Chrome extension captures, not set by hand. */
  messages_sent: number;
  /** Whether this prospect has replied and we still owe them an answer — i.e.
   *  their newest captured message is incoming. Dynamic and derived from messages
   *  the Chrome extension captures: a reply at any stage sets it, and our answer
   *  clears it. Drives the "Awaiting reply" treatment on their card. */
  awaiting_reply: boolean;
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

/** Move a prospect to a different stage of its pipeline (drag or stage menu).
 *  Returns the updated prospect. */
export function setProspectStage(
  id: number,
  stageId: number,
): Promise<Prospect> {
  return invoke("set_prospect_stage", { id, stageId });
}

/** Subscribe to backend "prospects changed" pushes — fired when the Chrome
 *  extension captures a prospect or a sent message bumps a derived count — so an
 *  open view can re-fetch and update live. */
export function onProspectsChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("prospects://changed", () => cb());
}
