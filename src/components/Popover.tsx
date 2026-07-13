import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import styles from "./Popover.module.css";

/** A floating surface anchored to a trigger, rendered in a portal on
 *  `document.body` so it escapes any `overflow: hidden/auto` ancestor (cards,
 *  scrolling columns) that would otherwise clip it. The consumer owns the
 *  trigger button and the `open` state; this owns positioning and dismissal.
 *
 *  Placement: opens below-left of the trigger, flips above when there isn't room
 *  below, and clamps to the viewport so it never crosses an edge. Dismisses on
 *  outside-click, Escape, and any scroll/resize (the anchor rect is measured once
 *  on open, so a scroll would otherwise leave it stranded). */
export function Popover({
  open,
  onClose,
  anchorRef,
  children,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  const popRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  // Measure the anchor + surface and place the menu before the browser paints,
  // so it never flashes at the wrong spot. Runs while `pos` is still null (the
  // surface renders hidden), then reveals it at the computed coordinates.
  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const anchor = anchorRef.current;
    const pop = popRef.current;
    if (!anchor || !pop) return;

    const a = anchor.getBoundingClientRect();
    const p = pop.getBoundingClientRect();
    const gap = 4;
    const margin = 8;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    // Vertical: below by default; flip above only if it doesn't fit below but
    // does above. Then clamp so it stays on screen either way.
    let top = a.bottom + gap;
    const fitsBelow = top + p.height <= vh - margin;
    const fitsAbove = a.top - gap - p.height >= margin;
    if (!fitsBelow && fitsAbove) top = a.top - gap - p.height;
    top = Math.max(margin, Math.min(top, vh - margin - p.height));

    // Horizontal: align to the trigger's left edge, clamp to the viewport.
    let left = a.left;
    if (left + p.width > vw - margin) left = vw - margin - p.width;
    left = Math.max(margin, left);

    setPos({ top, left });
  }, [open, anchorRef]);

  // Dismiss on outside-click, Escape, and any scroll/resize.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(e: MouseEvent) {
      const t = e.target as Node;
      if (popRef.current?.contains(t)) return;
      // Ignore the trigger: its own onClick toggles `open`, so closing here too
      // would immediately reopen (or cancel the toggle).
      if (anchorRef.current?.contains(t)) return;
      onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKey);
    // `true` = capture, so a scroll on any ancestor (not just window) closes it.
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("resize", onClose);
    };
  }, [open, onClose, anchorRef]);

  if (!open) return null;

  return createPortal(
    <div
      ref={popRef}
      className={styles.surface}
      role="menu"
      style={
        pos
          ? { top: pos.top, left: pos.left }
          : { top: 0, left: 0, visibility: "hidden" }
      }
    >
      {children}
    </div>,
    document.body,
  );
}

/** A row inside a Popover menu. `checked` undefined → a plain action item;
 *  defined → a radio item that shows a check when true. `leading` is optional
 *  content (a color dot, an icon) shown before the label. */
export function MenuItem({
  label,
  onSelect,
  danger = false,
  checked,
  disabled = false,
  leading,
}: {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
  leading?: ReactNode;
}) {
  const radio = checked !== undefined;
  return (
    <button
      type="button"
      role={radio ? "menuitemradio" : "menuitem"}
      aria-checked={radio ? checked : undefined}
      className={styles.item}
      data-danger={danger || undefined}
      data-current={checked || undefined}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onSelect();
      }}
    >
      {leading !== undefined && <span className={styles.itemLeading}>{leading}</span>}
      <span className={styles.itemLabel}>{label}</span>
      {radio && <CheckIcon visible={checked} />}
    </button>
  );
}

function CheckIcon({ visible }: { visible: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className={styles.check}
      data-visible={visible || undefined}
    >
      <path
        d="m5 12 5 5 9-11"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
