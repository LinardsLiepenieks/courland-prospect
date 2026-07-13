import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** A stage's kind. Every pipeline has exactly one `"messaging"` stage, always
 *  first; the rest are `"standard"`. */
export type StageKind = "messaging" | "standard";

/** A stage color — a palette token, not a hex, so it re-themes in dark mode.
 *  Mirrors the `--stage-<token>` variables in global.css. */
export type StageColor =
  | "blue"
  | "amber"
  | "green"
  | "purple"
  | "teal"
  | "pink"
  | "red"
  | "gray";

/** The pickable palette, in display/rotation order. Hand-kept mirror of the
 *  Rust source of truth (`STAGE_COLORS` in `features/stages/model.rs`) — Rust
 *  and TS can't share the list, but the Rust test `ts_stage_colors_mirror_the_
 *  rust_palette` parses this array and fails if the two drift. */
export const STAGE_COLORS: StageColor[] = [
  "blue",
  "amber",
  "green",
  "purple",
  "teal",
  "pink",
  "red",
  "gray",
];

/** A pipeline stage belonging to a pitch — one step of its funnel. */
export interface Stage {
  id: number;
  pitch_id: number;
  name: string;
  kind: StageKind;
  /** 0-based order within the pitch's pipeline. */
  position: number;
  color: StageColor;
  created_at: string;
}

/** A stage as sent to the backend when seeding a pipeline at pitch creation.
 *  Order in the array is the stage order; the backend validates that exactly
 *  one messaging stage exists and sits first, and that the color is known. */
export interface StageInput {
  name: string;
  kind: StageKind;
  color: StageColor;
}

// Typed wrappers over the Rust stage commands. All SQL lives in the backend.

export function listStages(pitchId: number): Promise<Stage[]> {
  return invoke("list_stages", { pitchId });
}

/** Append a new standard stage to the end of a pitch's pipeline. */
export function createStage(pitchId: number, name: string): Promise<Stage> {
  return invoke("create_stage", { pitchId, name });
}

export function renameStage(id: number, name: string): Promise<Stage> {
  return invoke("rename_stage", { id, name });
}

/** Set a stage's color to a palette token. Returns the updated stage. */
export function setStageColor(id: number, color: StageColor): Promise<Stage> {
  return invoke("set_stage_color", { id, color });
}

/** Delete a stage; its prospects fall back to the previous stage. The messaging
 *  stage and the last remaining stage can't be deleted (backend rejects). */
export function deleteStage(id: number): Promise<void> {
  return invoke("delete_stage", { id });
}

/** Persist a new stage order. `orderedIds` must be exactly the pitch's stages
 *  with the messaging stage first. Returns the reordered list. */
export function reorderStages(
  pitchId: number,
  orderedIds: number[],
): Promise<Stage[]> {
  return invoke("reorder_stages", { pitchId, orderedIds });
}

/** Subscribe to backend "stages changed" pushes — fired whenever a stage is
 *  created, renamed, recolored, reordered, or deleted. Lets a view editing the
 *  pipeline in one place (Settings) and a view rendering it in another (the
 *  Prospects board) stay in sync; a delete also reassigns prospects, so
 *  listeners should re-fetch prospects too. */
export function onStagesChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("stages://changed", () => cb());
}
