import type { ProspectViewProps } from "./ProspectsView";
import { formatDate } from "../lib/date";
import DeleteControl from "./DeleteControl";
import { effectiveStageId } from "./effectiveStage";
import {
  AdvanceSuggestion,
  AgePill,
  MessageCount,
  AwaitingReplyBadge,
  CustomerMenu,
  StageMenu,
} from "./ProspectControls";
import { stalenessOf } from "./staleness";
import styles from "./ProspectsList.module.css";

/** Flat list view: one row per prospect with its customer profile, stage,
 *  outreach counter (in the messaging stage), captured date, and delete. Dense
 *  and scannable. */
export default function ProspectsList({
  prospects,
  stages,
  customers,
  messagingStageId,
  busyIds,
  checkingIds,
  now,
  onOpen,
  onMove,
  onSetCustomer,
  onAcceptSuggestion,
  onDismissSuggestion,
  onRecheck,
  onDelete,
}: ProspectViewProps) {
  return (
    <ul className={styles.list}>
      {prospects.map((p) => {
        const busy = busyIds.has(p.id);
        const effectiveStage = effectiveStageId(p, stages, messagingStageId);
        const inMessaging = messagingStageId != null && effectiveStage === messagingStageId;
        const stage = stages.find((s) => s.id === effectiveStage);
        const staleness = stalenessOf(p, stage, now);
        const suggested = stages.find((s) => s.id === p.suggested_stage_id);
        return (
          <li
            key={p.id}
            className={styles.row}
            data-awaiting-reply={p.awaiting_reply || undefined}
            data-staleness={staleness.level === "fresh" ? undefined : staleness.level}
          >
            <button
              type="button"
              className={styles.open}
              onClick={() => onOpen(p.linkedin_url)}
              title={`Open ${p.name} on LinkedIn`}
            >
              <span className={styles.rowMain}>
                <span className={styles.name}>{p.name}</span>
                {p.headline && (
                  <span className={styles.headline}>{p.headline}</span>
                )}
              </span>
            </button>

            {suggested && (
              // Between the name and the controls, spanning the row: the list is
              // dense, so a suggestion has to break the line to be noticed at all.
              <div className={styles.rowSuggestion}>
                <AdvanceSuggestion
                  stageName={suggested.name}
                  reason={p.suggested_reason}
                  busy={busy}
                  onAccept={() => onAcceptSuggestion(p.id)}
                  onDismiss={() => onDismissSuggestion(p.id)}
                />
              </div>
            )}

            <div className={styles.aside}>
              {inMessaging && <MessageCount value={p.messages_sent} />}
              <AgePill reading={staleness} />
              {p.awaiting_reply && <AwaitingReplyBadge />}
              <CustomerMenu
                customers={customers}
                currentCustomerId={p.customer_id}
                onSet={(customerId) => onSetCustomer(p.id, customerId)}
                busy={busy}
              />
              <StageMenu
                stages={stages}
                currentStageId={effectiveStage}
                onMove={(stageId) => onMove(p.id, stageId)}
                onRecheck={() => onRecheck(p.id)}
                checking={checkingIds.has(p.id)}
                busy={busy}
              />
              <span className={styles.date}>{formatDate(p.created_at)}</span>
              <DeleteControl name={p.name} onDelete={() => onDelete(p.id)} />
            </div>
          </li>
        );
      })}
    </ul>
  );
}
