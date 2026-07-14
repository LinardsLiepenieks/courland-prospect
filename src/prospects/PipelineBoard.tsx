import { useState } from "react";
import type { Prospect } from "../api/prospects";
import type { Stage } from "../api/stages";
import type { ProspectViewProps } from "./ProspectsView";
import { stageAccentStyle } from "../lib/stageColor";
import DeleteControl from "./DeleteControl";
import { effectiveStageId } from "./effectiveStage";
import { MessageCount, AwaitingReplyBadge, StageMenu } from "./ProspectControls";
import styles from "./PipelineBoard.module.css";

/** Kanban board: one column per stage, cards dragged between columns to move a
 *  prospect. A per-card stage menu is the keyboard/click fallback for the drag.
 *  Prospects with no stage (transient, e.g. a just-deleted stage) bucket into
 *  the messaging column so they're never hidden. */
export default function PipelineBoard({
  prospects,
  stages,
  messagingStageId,
  busyIds,
  onOpen,
  onMove,
  onDelete,
}: ProspectViewProps) {
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const [overStageId, setOverStageId] = useState<number | null>(null);

  // Effective column for a prospect: its stage, or the messaging stage if unset
  // or dangling. Also drives the card's stage pill, so the pill and the column
  // never disagree (an unassigned card in Messaging reads "Messaging").
  const columnOf = (p: Prospect) => effectiveStageId(p, stages, messagingStageId);

  function endDrag() {
    setDraggingId(null);
    setOverStageId(null);
  }

  // `id` comes from the drop event's dataTransfer (set at drag start) so it
  // doesn't depend on React state having re-rendered; falls back to state.
  function dropOn(stageId: number, id: number | null) {
    endDrag();
    if (id == null) return;
    const prospect = prospects.find((p) => p.id === id);
    if (prospect && columnOf(prospect) !== stageId) onMove(id, stageId);
  }

  return (
    <div className={styles.board}>
      {stages.map((stage) => {
        const cards = prospects.filter((p) => columnOf(p) === stage.id);
        const isMessaging = stage.id === messagingStageId;
        return (
          <section
            key={stage.id}
            className={styles.column}
            style={stageAccentStyle(stage.color)}
            data-over={overStageId === stage.id || undefined}
            onDragOver={(e) => {
              // Must preventDefault to mark this a valid drop target — do it
              // unconditionally so a not-yet-committed drag state can't block it.
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              setOverStageId(stage.id);
            }}
            onDrop={(e) => {
              e.preventDefault();
              const raw = e.dataTransfer.getData("text/plain");
              dropOn(stage.id, raw ? Number(raw) : draggingId);
            }}
          >
            <header className={styles.columnHead}>
              <span className={styles.columnName}>
                <span
                  className={styles.colorDot}
                  data-messaging={isMessaging || undefined}
                  title={isMessaging ? "Messaging stage" : undefined}
                />
                {stage.name}
              </span>
              <span className={styles.columnCount}>{cards.length}</span>
            </header>

            <div className={styles.cards}>
              {cards.map((p) => (
                <ProspectCard
                  key={p.id}
                  prospect={p}
                  stages={stages}
                  currentStageId={columnOf(p)}
                  showCount={isMessaging}
                  busy={busyIds.has(p.id)}
                  dragging={draggingId === p.id}
                  onDragStart={() => setDraggingId(p.id)}
                  onDragEnd={endDrag}
                  onOpen={onOpen}
                  onMove={onMove}
                  onDelete={onDelete}
                />
              ))}
              {cards.length === 0 && (
                <div className={styles.columnEmpty} aria-hidden="true">
                  Drop here
                </div>
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function ProspectCard({
  prospect: p,
  stages,
  currentStageId,
  showCount,
  busy,
  dragging,
  onDragStart,
  onDragEnd,
  onOpen,
  onMove,
  onDelete,
}: {
  prospect: Prospect;
  stages: Stage[];
  currentStageId: number | null;
  showCount: boolean;
  busy: boolean;
  dragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  onOpen: (url: string) => void;
  onMove: (id: number, stageId: number) => void;
  onDelete: (id: number) => Promise<void>;
}) {
  return (
    <article
      className={styles.card}
      data-dragging={dragging || undefined}
      data-awaiting-reply={p.awaiting_reply || undefined}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", String(p.id));
        onDragStart();
      }}
      onDragEnd={onDragEnd}
    >
      <div className={styles.cardTop}>
        <button
          type="button"
          className={styles.cardName}
          onClick={() => onOpen(p.linkedin_url)}
          title={`Open ${p.name} on LinkedIn`}
        >
          {p.name}
        </button>
        <DeleteControl name={p.name} onDelete={() => onDelete(p.id)} />
      </div>
      {p.headline && <span className={styles.cardHeadline}>{p.headline}</span>}
      <div className={styles.cardFoot}>
        {(showCount || p.awaiting_reply) && (
          <div className={styles.cardTags}>
            {showCount && <MessageCount value={p.messages_sent} />}
            {p.awaiting_reply && <AwaitingReplyBadge />}
          </div>
        )}
        <StageMenu
          stages={stages}
          currentStageId={currentStageId}
          onMove={(stageId) => onMove(p.id, stageId)}
          busy={busy}
        />
      </div>
    </article>
  );
}
