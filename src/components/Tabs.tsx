import { useLayoutEffect, useRef, useState } from "react";
import styles from "./Tabs.module.css";

export interface TabItem<Id extends string = string> {
  id: Id;
  label: string;
}

interface Props<Id extends string> {
  items: TabItem<Id>[];
  active: Id;
  onChange: (id: Id) => void;
}

/**
 * Underline tabs with an indicator that slides between the active tab.
 * The indicator is measured from the active button so it tracks font/label
 * width exactly rather than assuming equal-width tabs.
 */
export default function Tabs<Id extends string>({
  items,
  active,
  onChange,
}: Props<Id>) {
  const listRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number }>({
    left: 0,
    width: 0,
  });

  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    // CSS.escape guards the generic Id: a future id with quotes/metachars
    // would otherwise make querySelector throw during layout.
    const el = list.querySelector<HTMLElement>(`[data-id="${CSS.escape(active)}"]`);
    if (el) setIndicator({ left: el.offsetLeft, width: el.offsetWidth });
  }, [active, items]);

  return (
    <div className={styles.tabs} role="tablist" ref={listRef}>
      {items.map((item) => (
        <button
          key={item.id}
          data-id={item.id}
          role="tab"
          aria-selected={item.id === active}
          className={styles.tab}
          data-active={item.id === active}
          onClick={() => onChange(item.id)}
        >
          {item.label}
        </button>
      ))}
      <span
        className={styles.indicator}
        aria-hidden="true"
        style={{
          transform: `translateX(${indicator.left}px)`,
          width: indicator.width,
        }}
      />
    </div>
  );
}
