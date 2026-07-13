import { useRef, useState } from "react";
import type { Stage } from "../api/stages";
import { MenuItem, Popover } from "../components/Popover";
import { stageAccentStyle } from "../lib/stageColor";
import styles from "./ProspectControls.module.css";

/** The messaging-stage outreach count — read-only. Auto-tracked from messages
 *  captured by the Chrome extension (there's no manual entry). Shown only in the
 *  messaging stage, where outreach cadence is the metric that matters. */
export function MessageCount({ value }: { value: number }) {
  return (
    <span
      className={styles.count}
      title={`${value} ${value === 1 ? "message" : "messages"} sent`}
    >
      <MailIcon />
      <span className={styles.countValue}>{value}</span>
    </span>
  );
}

/** Durable "they replied" marker — read-only, shown on any prospect who has
 *  responded, in any stage. Green to match the card's responded wash; a reply
 *  arrow reinforces the meaning at a glance. Not interactive. */
export function RespondedBadge() {
  return (
    <span className={styles.responded} title="This prospect has replied to you">
      <ReplyIcon />
      Responded
    </span>
  );
}

/** A pill showing the prospect's current stage that opens a menu to move them to
 *  another stage. The keyboard/click path for reassignment (drag is the primary
 *  in the pipeline; this is always available and is the only mover in the list). */
export function StageMenu({
  stages,
  currentStageId,
  onMove,
  busy,
}: {
  stages: Stage[];
  currentStageId: number | null;
  onMove: (stageId: number) => void;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const current = stages.find((s) => s.id === currentStageId);

  function pick(stageId: number) {
    setOpen(false);
    if (stageId !== currentStageId) onMove(stageId);
  }

  return (
    <div className={styles.stageMenu}>
      <button
        ref={triggerRef}
        type="button"
        className={styles.stagePill}
        style={current ? stageAccentStyle(current.color) : undefined}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        title="Move to stage"
      >
        <span className={styles.stagePillDot} />
        <span className={styles.stagePillName}>{current?.name ?? "Unassigned"}</span>
        <ChevronDown />
      </button>
      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        {stages.map((s) => (
          <MenuItem
            key={s.id}
            label={s.name}
            checked={s.id === currentStageId}
            leading={<span className={styles.dot} style={stageAccentStyle(s.color)} />}
            onSelect={() => pick(s.id)}
          />
        ))}
      </Popover>
    </div>
  );
}

function ReplyIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M9 10 4 15l5 5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M4 15h9a7 7 0 0 0 7-7V6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function MailIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect
        x="3"
        y="5"
        width="18"
        height="14"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path
        d="m4 7 8 6 8-6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ChevronDown() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.chevron}>
      <path
        d="m6 9 6 6 6-6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
