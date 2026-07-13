import { useRef, useState } from "react";
import { STAGE_COLORS, type StageColor, type StageKind } from "../api/stages";
import { Popover } from "../components/Popover";
import { stageAccentStyle } from "../lib/stageColor";
import styles from "./StageEditor.module.css";

/** A row in the editor. `id` is present once persisted (Settings); absent for
 *  unsaved rows in the create flow. `key` is stable for React + change routing. */
export interface DraftStage {
  key: string;
  id?: number;
  name: string;
  kind: StageKind;
  color: StageColor;
}

/** What changed, so a persisted parent (Settings) can fire the matching API call
 *  while a local parent (create flow) can just adopt `next`. */
export type StageChange =
  | { type: "add" }
  | { type: "remove"; key: string }
  | { type: "rename"; key: string; name: string }
  | { type: "color"; key: string; color: StageColor }
  | { type: "reorder" };

let keyCounter = 0;
/** A fresh stable key for a newly-added (unsaved) row. */
export function newStageKey(): string {
  keyCounter += 1;
  return `new-${keyCounter}`;
}

/** The built-in Full-cycle pipeline, as draft rows for the create flow. Mirrors
 *  the backend's `full_cycle_template`: the names must match, and colors come
 *  from the shared `STAGE_COLORS` rotation (same as the backend's
 *  `color_for_position`), so neither list needs hand-syncing. */
const FULL_CYCLE: { name: string; kind: StageKind }[] = [
  { name: "Messaged", kind: "messaging" },
  { name: "Meeting", kind: "standard" },
  { name: "Onboarding", kind: "standard" },
  { name: "Feedback", kind: "standard" },
];

export function fullCycleDraft(): DraftStage[] {
  return FULL_CYCLE.map((s, i) => ({
    key: newStageKey(),
    name: s.name,
    kind: s.kind,
    color: STAGE_COLORS[i % STAGE_COLORS.length],
  }));
}

interface Props {
  stages: DraftStage[];
  /** Called with the resulting list and a description of the single edit. */
  onChange: (next: DraftStage[], change: StageChange) => void;
  /** Locks all controls while a persist is in flight (Settings). */
  disabled?: boolean;
}

/** Edit a pitch's pipeline: rename, add, remove, reorder. The first stage is the
 *  messaging stage — renameable, but locked to the top and never removable. Used
 *  in both the create flow (local draft) and Settings (persisted). */
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
        { key: newStageKey(), name: "New stage", kind: "standard", color },
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
            last={index === stages.length - 1}
            disabled={disabled}
            onRename={(name) => rename(stage.key, name)}
            onSetColor={(color) => setColor(stage.key, color)}
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
  last,
  disabled,
  onRename,
  onSetColor,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  stage: DraftStage;
  first: boolean;
  last: boolean;
  disabled: boolean;
  onRename: (name: string) => void;
  onSetColor: (color: StageColor) => void;
  onRemove: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
}) {
  // Local editing buffer so we commit a rename once (on blur / Enter), not per
  // keystroke — the parent may persist each committed change.
  const [value, setValue] = useState(stage.name);

  function commit() {
    const trimmed = value.trim();
    if (!trimmed) {
      setValue(stage.name); // reject empty; revert to the last good name
      return;
    }
    if (trimmed !== stage.name) onRename(trimmed);
  }

  return (
    <li className={styles.row} data-messaging={first || undefined}>
      <div className={styles.reorder}>
        <button
          type="button"
          className={styles.moveBtn}
          onClick={onMoveUp}
          // index 1 is the first movable row; it can't move above messaging.
          disabled={disabled || first || undefined}
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
    </li>
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
