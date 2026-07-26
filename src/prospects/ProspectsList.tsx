import type { ProspectViewProps } from "./ProspectsView";
import { formatDate } from "../lib/date";
import DeleteControl from "./DeleteControl";
import { effectiveStageId } from "./effectiveStage";
import {
  MessageCount,
  AwaitingReplyBadge,
  CustomerMenu,
  StageMenu,
} from "./ProspectControls";
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
  onOpen,
  onMove,
  onSetCustomer,
  onDelete,
}: ProspectViewProps) {
  return (
    <ul className={styles.list}>
      {prospects.map((p) => {
        const busy = busyIds.has(p.id);
        const effectiveStage = effectiveStageId(p, stages, messagingStageId);
        const inMessaging = messagingStageId != null && effectiveStage === messagingStageId;
        return (
          <li key={p.id} className={styles.row} data-awaiting-reply={p.awaiting_reply || undefined}>
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

            <div className={styles.aside}>
              {inMessaging && <MessageCount value={p.messages_sent} />}
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
