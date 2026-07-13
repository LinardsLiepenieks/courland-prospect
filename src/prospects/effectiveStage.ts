import type { Prospect } from "../api/prospects";
import type { Stage } from "../api/stages";

/** The stage a prospect effectively lives in. A prospect is bucketed into the
 *  messaging stage whenever it has no stage of its own OR its `stage_id` points
 *  at a stage that isn't in the current pipeline (a dangling reference — e.g.
 *  the stage was just deleted, or the prospect/stage lists briefly disagree
 *  mid-refresh). Without the dangling check a stale non-null `stage_id` would
 *  match no column and the card would silently vanish from every view.
 *
 *  Shared by both prospect views so the "unassigned/dangling → messaging" rule
 *  lives in one place. */
export function effectiveStageId(
  prospect: Prospect,
  stages: Stage[],
  messagingStageId: number | null,
): number | null {
  if (prospect.stage_id != null && stages.some((s) => s.id === prospect.stage_id)) {
    return prospect.stage_id;
  }
  return messagingStageId;
}
