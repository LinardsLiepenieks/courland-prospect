import { useRef, useState } from "react";
import type { Customer } from "../api/customers";
import type { Stage } from "../api/stages";
import { MenuDivider, MenuItem, MenuNote, Popover } from "../components/Popover";
import { stageAccentStyle } from "../lib/stageColor";
import { describeStaleness, formatAge, type StalenessReading } from "./staleness";
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

/** "They replied and you owe them an answer" marker — read-only, shown on any
 *  prospect whose newest message is incoming, in any stage. It clears once you
 *  answer. Green to match the card's awaiting-reply wash; a reply arrow
 *  reinforces the meaning at a glance. Not interactive. */
export function AwaitingReplyBadge() {
  return (
    <span className={styles.awaitingReply} title="Replied — you haven't answered yet">
      <ReplyIcon />
      Awaiting reply
    </span>
  );
}

/** How long this prospect has been waiting on you, shown only once it's worth
 *  knowing. A card that's inside its stage's tolerance renders nothing at all —
 *  an age on every card would be noise, and the whole point of the grade is that
 *  a colored pill means "this one". Read-only. */
export function AgePill({ reading }: { reading: StalenessReading }) {
  if (reading.level === "fresh") return null;
  return (
    <span
      className={styles.age}
      data-level={reading.level}
      title={describeStaleness(reading)}
    >
      <ClockIcon />
      {formatAge(reading.days)}
    </span>
  );
}

/**
 * The advance analyzer's pending verdict: this prospect looks ready for the next
 * stage, here's why, take it or leave it.
 *
 * Accept and dismiss are equally weighted, deliberately. The suggestion is a
 * guess made by a model reading a scraped thread — presenting it as the obvious
 * action (a single prominent button) would train the user to accept without
 * reading, which is exactly the failure mode that makes an auto-advancing board
 * untrustworthy. The reason line is the point of the whole chip: it's what lets
 * you judge without reopening LinkedIn.
 */
export function AdvanceSuggestion({
  stageName,
  reason,
  busy,
  onAccept,
  onDismiss,
}: {
  stageName: string;
  reason: string;
  busy: boolean;
  onAccept: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className={styles.suggestion}>
      <div className={styles.suggestionHead}>
        <SparkIcon />
        <span className={styles.suggestionTitle}>
          Ready for <strong>{stageName}</strong>
        </span>
        <span className={styles.suggestionActions}>
          <button
            type="button"
            className={styles.suggestionAccept}
            onClick={(e) => {
              e.stopPropagation();
              onAccept();
            }}
            disabled={busy}
            title={`Move to ${stageName}`}
            aria-label={`Move to ${stageName}`}
          >
            <CheckIcon />
          </button>
          <button
            type="button"
            className={styles.suggestionDismiss}
            onClick={(e) => {
              e.stopPropagation();
              onDismiss();
            }}
            disabled={busy}
            title="Dismiss this suggestion"
            aria-label="Dismiss this suggestion"
          >
            <CloseIcon />
          </button>
        </span>
      </div>
      {reason && <p className={styles.suggestionReason}>{reason}</p>}
    </div>
  );
}

/** A pill showing the prospect's current stage that opens a menu to move them to
 *  another stage. The keyboard/click path for reassignment (drag is the primary
 *  in the pipeline; this is always available and is the only mover in the list). */
export function StageMenu({
  stages,
  currentStageId,
  onMove,
  onRecheck,
  checking,
  busy,
}: {
  stages: Stage[];
  currentStageId: number | null;
  onMove: (stageId: number) => void;
  /** Ask the analyzer to reconsider this prospect now. */
  onRecheck: () => void;
  /** A re-check is in flight — it shells out to Claude Code, so this can run for
   *  several seconds and the pill says so rather than just going inert. */
  checking: boolean;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const current = stages.find((s) => s.id === currentStageId);
  // Only offer a re-check where one could change something: the last stage has
  // nowhere to advance to, and a stage with no goal has no test to run.
  const canAdvance =
    current != null &&
    current.goal.trim() !== "" &&
    stages.some((s) => s.position > current.position);

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
        aria-busy={checking}
        title="Move to stage"
      >
        <span className={styles.stagePillDot} data-checking={checking || undefined} />
        <span className={styles.stagePillName}>
          {checking ? "Checking…" : (current?.name ?? "Unassigned")}
        </span>
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
        {canAdvance && (
          <>
            <MenuDivider />
            <MenuItem
              label="Re-check this stage"
              leading={<SparkIcon />}
              onSelect={() => {
                setOpen(false);
                onRecheck();
              }}
            />
            <MenuNote>
              Ask Claude whether they've met “{current.name}”. Runs on its own
              after every new message — this is for when none is coming.
            </MenuNote>
          </>
        )}
      </Popover>
    </div>
  );
}

/**
 * A pill showing which customer profile this prospect matches, opening a menu to
 * re-tag them (or clear it).
 *
 * Deliberately quieter than the stage pill beside it: the stage is *where they
 * are*, which the whole board is organized around, while the profile is *how
 * their drafts get steered* — important, but not a position. Re-tagging is
 * instant and reversible (it never moves them on the board), so there's no
 * confirmation.
 */
export function CustomerMenu({
  customers,
  currentCustomerId,
  onSet,
  busy,
}: {
  customers: Customer[];
  currentCustomerId: number | null;
  onSet: (customerId: number | null) => void;
  busy: boolean;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const current = customers.find((c) => c.id === currentCustomerId);
  const unassigned = !current;

  function pick(customerId: number | null) {
    setOpen(false);
    if (customerId !== currentCustomerId) onSet(customerId);
  }

  return (
    <div className={styles.customerMenu}>
      <button
        ref={triggerRef}
        type="button"
        className={styles.customerPill}
        data-unassigned={unassigned || undefined}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        title={
          current
            ? `Customer profile: ${current.name}`
            : "No customer profile — drafts for this person won't have a goal"
        }
      >
        <PersonIcon />
        <span className={styles.customerPillName}>
          {current?.name ?? "No profile"}
        </span>
        <ChevronDown />
      </button>
      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        {customers.map((c) => (
          <MenuItem
            key={c.id}
            label={c.name}
            checked={c.id === currentCustomerId}
            onSelect={() => pick(c.id)}
          />
        ))}
        {customers.length === 0 && <MenuNote>No customer profiles yet</MenuNote>}
        <MenuDivider />
        <MenuItem
          label="No profile"
          checked={unassigned}
          onSelect={() => pick(null)}
        />
      </Popover>
    </div>
  );
}

function PersonIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.personIcon}>
      <circle cx="12" cy="8" r="3.4" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
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

function ClockIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="8.4" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M12 7.6V12l2.8 2.2"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A four-point sparkle — the app's mark for "the model decided this". */
function SparkIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.sparkIcon}>
      <path
        d="M12 3.2c.6 3.9 1.9 5.2 5.8 5.8-3.9.6-5.2 1.9-5.8 5.8-.6-3.9-1.9-5.2-5.8-5.8 3.9-.6 5.2-1.9 5.8-5.8Z"
        fill="currentColor"
      />
      <path
        d="M18.2 14.6c.3 1.9.9 2.5 2.8 2.8-1.9.3-2.5.9-2.8 2.8-.3-1.9-.9-2.5-2.8-2.8 1.9-.3 2.5-.9 2.8-2.8Z"
        fill="currentColor"
        opacity="0.65"
      />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="m5 12.5 4.5 4.5L19 7"
        stroke="currentColor"
        strokeWidth="2.1"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M6.5 6.5l11 11M17.5 6.5l-11 11"
        stroke="currentColor"
        strokeWidth="2.1"
        strokeLinecap="round"
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
