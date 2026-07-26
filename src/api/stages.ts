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

/** A stage of the one shared pipeline — one step of the funnel every prospect
 *  moves through, whichever customer profile they match. */
export interface Stage {
  id: number;
  name: string;
  /** 0-based order within the pipeline. */
  position: number;
  kind: StageKind;
  color: StageColor;
  created_at: string;
}

// Typed wrappers over the Rust stage commands. All SQL lives in the backend.
// There is one pipeline, so none of these take an owner.

export function listStages(): Promise<Stage[]> {
  return invoke("list_stages");
}

/** Append a new standard stage to the end of the pipeline. */
export function createStage(name: string): Promise<Stage> {
  return invoke("create_stage", { name });
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

/** Persist a new stage order. `orderedIds` must be exactly the pipeline's stages
 *  with the messaging stage first. Returns the reordered list. */
export function reorderStages(orderedIds: number[]): Promise<Stage[]> {
  return invoke("reorder_stages", { orderedIds });
}

/** Subscribe to backend "stages changed" pushes — fired whenever a stage is
 *  created, renamed, recolored, reordered, or deleted. A delete also reassigns
 *  prospects, so listeners should re-fetch prospects too. */
export function onStagesChanged(cb: () => void): Promise<UnlistenFn> {
  return listen("stages://changed", () => cb());
}
