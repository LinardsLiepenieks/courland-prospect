import { useEffect, useRef, useState } from "react";
import {
  MAX_STALENESS_DAYS,
  MIN_STALENESS_DAYS,
  STAGE_COLORS,
  type StageColor,
  type StageKind,
} from "../api/stages";
import { Popover } from "../components/Popover";
import { stageAccentStyle } from "../lib/stageColor";
import styles from "./StageEditor.module.css";

/** A row in the editor. `id` is the persisted stage's id, absent on a row the
 *  user just added — which is how the parent spots the new row to create (see
 *  `PipelineSection.persist`). `key` is stable for React + change routing. */
export interface DraftStage {
  key: string;
  id?: number;
  name: string;
  kind: StageKind;
  color: StageColor;
  /** What this step is for — the exit condition the advance analyzer tests each
   *  new message against, and the near target a draft aims at. Empty opts the
   *  stage out of both. */
  goal: string;
  /** Days of silence before a card here is nudged, then flagged as rotting. */
  warnDays: number;
  staleDays: number;
}

/** What changed, so a persisted parent (Settings) can fire the matching API call
 *  while a local parent (create flow) can just adopt `next`. */
export type StageChange =
  | { type: "add" }
  | { type: "remove"; key: string }
  | { type: "rename"; key: string; name: string }
  | { type: "color"; key: string; color: StageColor }
  | { type: "goal"; key: string; goal: string }
  | { type: "thresholds"; key: string; warnDays: number; staleDays: number }
  | { type: "reorder" };

let keyCounter = 0;
/** A fresh stable key for a newly-added (unsaved) row. */
function newStageKey(): string {
  keyCounter += 1;
  return `new-${keyCounter}`;
}

interface Props {
  stages: DraftStage[];
  /** Called with the resulting list and a description of the single edit. */
  onChange: (next: DraftStage[], change: StageChange) => void;
  /** Locks all controls while a persist is in flight. */
  disabled?: boolean;
}

/** Edit the pipeline: rename, add, remove, reorder. The first stage is the
 *  messaging stage — renameable, but locked to the top and never removable.
 *
 *  There is one shared pipeline, seeded by migration, so every edit here persists
 *  immediately; the editor stays purely presentational and routes each change to
 *  the parent, which owns the API calls. */
export default function StageEditor({
  stages,
  onChange,
  disabled = false,
}: Props) {
  function rename(key: string, name: string) {
    const next = stages.map((s) => (s.key === key ? { ...s, name } : s));
    onChange(next, { type: "rename", key, name });
  }

  function remove(key: string) {
    onChange(
      stages.filter((s) => s.key !== key),
      { type: "remove", key },
    );
  }

  function setColor(key: string, color: StageColor) {
    const next = stages.map((s) => (s.key === key ? { ...s, color } : s));
    onChange(next, { type: "color", key, color });
  }

  function setGoal(key: string, goal: string) {
    const next = stages.map((s) => (s.key === key ? { ...s, goal } : s));
    onChange(next, { type: "goal", key, goal });
  }

  function setThresholds(key: string, warnDays: number, staleDays: number) {
    const next = stages.map((s) =>
      s.key === key ? { ...s, warnDays, staleDays } : s,
    );
    onChange(next, { type: "thresholds", key, warnDays, staleDays });
  }

  // Swap a stage with its neighbour. The messaging stage (index 0) is pinned, so
  // moves never cross it (guarded by disabling the buttons at the edges).
  function move(index: number, dir: -1 | 1) {
    const target = index + dir;
    if (target < 1 || target >= stages.length) return;
    const next = [...stages];
    [next[index], next[target]] = [next[target], next[index]];
    onChange(next, { type: "reorder" });
  }

  function add() {
    // Rotate the palette by position so a new stage lands with a distinct color
    // (matches the backend's append default).
    const color = STAGE_COLORS[stages.length % STAGE_COLORS.length];
    onChange(
      [
        ...stages,
        {
          key: newStageKey(),
          name: "New stage",
          kind: "standard",
          color,
          // Matches the backend's append defaults, so the optimistic row and the
          // one that comes back from the server look identical.
          goal: "",
          warnDays: 3,
          staleDays: 7,
        },
      ],
      { type: "add" },
    );
  }

  return (
    <div className={styles.editor}>
      <ul className={styles.list}>
        {stages.map((stage, index) => (
          <StageRow
            key={stage.key}
            stage={stage}
            first={index === 0}
            // Index 1 sits directly under the pinned messaging stage, so it has
            // nowhere to move up to — `move` refuses it, and an enabled button
            // that does nothing reads as a bug.
            firstMovable={index === 1}
            last={index === stages.length - 1}
            disabled={disabled}
            onRename={(name) => rename(stage.key, name)}
            onSetColor={(color) => setColor(stage.key, color)}
            onSetGoal={(goal) => setGoal(stage.key, goal)}
            onSetThresholds={(warn, stale) => setThresholds(stage.key, warn, stale)}
            onRemove={() => remove(stage.key)}
            onMoveUp={() => move(index, -1)}
            onMoveDown={() => move(index, 1)}
          />
        ))}
      </ul>
      <button
        type="button"
        className={styles.addBtn}
        onClick={add}
        disabled={disabled}
      >
        <PlusIcon />
        Add stage
      </button>
    </div>
  );
}

function StageRow({
  stage,
  first,
  firstMovable,
  last,
  disabled,
  onRename,
  onSetColor,
  onSetGoal,
  onSetThresholds,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  stage: DraftStage;
  first: boolean;
  firstMovable: boolean;
  last: boolean;
  disabled: boolean;
  onRename: (name: string) => void;
  onSetColor: (color: StageColor) => void;
  onSetGoal: (goal: string) => void;
  onSetThresholds: (warnDays: number, staleDays: number) => void;
  onRemove: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
}) {
  // Local editing buffer so we commit a rename once (on blur / Enter), not per
  // keystroke — the parent may persist each committed change.
  const [value, setValue] = useState(stage.name);
  // The goal + cadence live behind a disclosure. The list's job is to show the
  // shape of the funnel at a glance; four always-open textareas would bury that
  // under detail you set once and revisit rarely.
  const [open, setOpen] = useState(false);

  // Follow the committed name whenever the parent's copy changes underneath us.
  // Rows are keyed by a stable id, so this component survives a rejected save:
  // the parent reverts `stage.name` to the last good value while the buffer
  // still holds the text the server refused, leaving the field showing a name
  // that isn't saved — and re-firing the same doomed rename on the next blur.
  useEffect(() => {
    setValue(stage.name);
  }, [stage.name]);

  function commit() {
    const trimmed = value.trim();
    if (!trimmed) {
      setValue(stage.name); // reject empty; revert to the last good name
      return;
    }
    if (trimmed !== stage.name) onRename(trimmed);
  }

  return (
    <li className={styles.row} data-messaging={first || undefined} data-open={open || undefined}>
      <div className={styles.reorder}>
        <button
          type="button"
          className={styles.moveBtn}
          onClick={onMoveUp}
          disabled={disabled || first || firstMovable || undefined}
          aria-label={`Move ${stage.name} up`}
        >
          <ChevronIcon dir="up" />
        </button>
        <button
          type="button"
          className={styles.moveBtn}
          onClick={onMoveDown}
          disabled={disabled || first || last || undefined}
          aria-label={`Move ${stage.name} down`}
        >
          <ChevronIcon dir="down" />
        </button>
      </div>

      <div className={styles.field}>
        <input
          className={styles.input}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              e.currentTarget.blur();
            }
            if (e.key === "Escape") {
              setValue(stage.name);
              e.currentTarget.blur();
            }
          }}
          disabled={disabled}
          aria-label="Stage name"
        />
        {first && <span className={styles.badge}>Messaging</span>}
      </div>

      <ColorSwatch color={stage.color} disabled={disabled} onPick={onSetColor} />

      <button
        type="button"
        className={styles.disclosure}
        // A filled dot marks a stage that already has a goal, so you can see
        // which steps steer drafts (and auto-advance) without opening each one.
        data-has-goal={stage.goal.trim() !== "" || undefined}
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-label={`${open ? "Hide" : "Show"} goal and cadence for ${stage.name}`}
        title="Goal and cadence"
      >
        <TargetIcon />
        <ChevronIcon dir={open ? "up" : "down"} />
      </button>

      {first ? (
        // The messaging stage can't be removed; reserve the slot so rows align.
        <span className={styles.removeSpacer} aria-hidden="true" />
      ) : (
        <button
          type="button"
          className={styles.removeBtn}
          onClick={onRemove}
          disabled={disabled}
          aria-label={`Remove ${stage.name}`}
          title={`Remove ${stage.name}`}
        >
          <CloseIcon />
        </button>
      )}

      {open && (
        <StageDetails
          stage={stage}
          disabled={disabled}
          onSetGoal={onSetGoal}
          onSetThresholds={onSetThresholds}
        />
      )}
    </li>
  );
}

/**
 * The expanded half of a stage row: what this step is for, and how long someone
 * may sit in it untouched.
 *
 * Both commit on blur rather than per keystroke, matching the name field above —
 * the parent persists every committed change, and a write per character would
 * be a write per character.
 */
function StageDetails({
  stage,
  disabled,
  onSetGoal,
  onSetThresholds,
}: {
  stage: DraftStage;
  disabled: boolean;
  onSetGoal: (goal: string) => void;
  onSetThresholds: (warnDays: number, staleDays: number) => void;
}) {
  const [goal, setGoal] = useState(stage.goal);

  // Follow the committed value when the parent's copy changes underneath us
  // (a reload, or a rejected save reverting) — same reasoning as the name field.
  useEffect(() => {
    setGoal(stage.goal);
  }, [stage.goal]);

  function commitGoal() {
    const trimmed = goal.trim();
    if (trimmed !== stage.goal) onSetGoal(trimmed);
    else setGoal(stage.goal); // normalize away whitespace-only edits
  }

  return (
    <div className={styles.details}>
      <label className={styles.detailField}>
        <span className={styles.detailLabel}>Goal of this step</span>
        <textarea
          className={styles.goalInput}
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          onBlur={commitGoal}
          onKeyDown={(e) => {
            // Escape reverts; Enter is a newline here (unlike the name field) —
            // a goal is a sentence, and it may want two.
            if (e.key === "Escape") {
              setGoal(stage.goal);
              e.currentTarget.blur();
            }
          }}
          rows={2}
          disabled={disabled}
          placeholder="What has to be true before they move on? e.g. “They've agreed to a call and a time is set.”"
        />
        <span className={styles.detailHint}>
          Drafts for people here aim at this, and after each new message Claude
          checks whether it's been met. Leave it empty to turn both off.
        </span>
      </label>

      <fieldset className={styles.cadence} disabled={disabled}>
        <legend className={styles.detailLabel}>Cadence</legend>
        <div className={styles.cadenceRow}>
          <DayInput
            label="Nudge after"
            value={stage.warnDays}
            tone="warn"
            // Each field's own range leaves room for its partner, so the pair the
            // backend receives is always valid: warn stops one short of the
            // ceiling, stale starts one above the floor.
            min={MIN_STALENESS_DAYS}
            max={MAX_STALENESS_DAYS - 1}
            // The backend rejects warn >= stale. Rather than surface that as an
            // error, carry the other value along so the pair stays valid: pushing
            // the nudge past the stale mark pushes the stale mark with it.
            onCommit={(warn) =>
              onSetThresholds(warn, Math.max(stage.staleDays, warn + 1))
            }
          />
          <DayInput
            label="Stale after"
            value={stage.staleDays}
            tone="stale"
            min={MIN_STALENESS_DAYS + 1}
            max={MAX_STALENESS_DAYS}
            onCommit={(stale) =>
              onSetThresholds(Math.min(stage.warnDays, stale - 1), stale)
            }
          />
        </div>
        <span className={styles.detailHint}>
          Days without a message from you before a card here turns amber, then
          red.
        </span>
      </fieldset>
    </div>
  );
}

/** A small number field for one staleness threshold, clamped to `[min, max]` and
 *  committed on blur.
 *
 *  Clamping here rather than in the caller matters: the displayed text is
 *  corrected in the same tick as the write, so the field never shows a number
 *  that isn't what got saved. (The pair-keeping in `StageDetails` then only has
 *  to worry about ordering, never about range.) */
function DayInput({
  label,
  value,
  tone,
  min,
  max,
  onCommit,
}: {
  label: string;
  value: number;
  tone: "warn" | "stale";
  min: number;
  max: number;
  onCommit: (days: number) => void;
}) {
  const [text, setText] = useState(String(value));

  useEffect(() => {
    setText(String(value));
  }, [value]);

  function commit() {
    const parsed = Number.parseInt(text, 10);
    if (Number.isNaN(parsed)) {
      setText(String(value)); // reject junk; revert to the last good number
      return;
    }
    const clamped = Math.min(max, Math.max(min, parsed));
    setText(String(clamped));
    if (clamped !== value) onCommit(clamped);
  }

  return (
    <label className={styles.dayField} data-tone={tone}>
      <span className={styles.dayLabel}>{label}</span>
      <span className={styles.dayControl}>
        <input
          className={styles.dayInput}
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              e.currentTarget.blur();
            }
            if (e.key === "Escape") {
              setText(String(value));
              e.currentTarget.blur();
            }
          }}
        />
        <span className={styles.dayUnit}>days</span>
      </span>
    </label>
  );
}

/** A target/bullseye — the mark for a stage's goal. */
function TargetIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={styles.targetIcon}>
      <circle cx="12" cy="12" r="8.2" stroke="currentColor" strokeWidth="1.7" />
      <circle cx="12" cy="12" r="3.4" stroke="currentColor" strokeWidth="1.7" />
      <circle cx="12" cy="12" r="1.3" fill="currentColor" className={styles.targetPip} />
    </svg>
  );
}

/** A round swatch showing the stage's color; opens a palette grid to change it. */
function ColorSwatch({
  color,
  disabled,
  onPick,
}: {
  color: StageColor;
  disabled: boolean;
  onPick: (color: StageColor) => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  return (
    <div className={styles.swatchWrap}>
      <button
        ref={triggerRef}
        type="button"
        className={styles.swatch}
        style={stageAccentStyle(color)}
        onClick={() => setOpen((o) => !o)}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Stage color: ${color}. Change`}
        title="Stage color"
      />
      <Popover open={open} onClose={() => setOpen(false)} anchorRef={triggerRef}>
        <div className={styles.palette}>
          {STAGE_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              role="menuitemradio"
              aria-checked={c === color}
              className={styles.paletteSwatch}
              data-current={c === color || undefined}
              style={stageAccentStyle(c)}
              onClick={() => {
                setOpen(false);
                if (c !== color) onPick(c);
              }}
              aria-label={c}
              title={c}
            />
          ))}
        </div>
      </Popover>
    </div>
  );
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M12 5v14M5 12h14"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d="M6 6l12 12M18 6L6 18"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

function ChevronIcon({ dir }: { dir: "up" | "down" }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path
        d={dir === "up" ? "m6 15 6-6 6 6" : "m6 9 6 6 6-6"}
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
