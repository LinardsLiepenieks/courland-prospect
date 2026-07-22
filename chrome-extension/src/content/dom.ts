// Tiny DOM helpers shared by the injected content-script UIs (the capture widget
// and the draft control). Content-side only — uses `document`, so it must never
// be imported by the service worker.

/** Create an element, optionally with a class. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

/** Resolve once this tab is in the foreground — immediately if it already is,
 *  else on the next visibility/focus gain. Shared by the review-tab fill flows
 *  (message replies and post comments): `execCommand` acts only on the active
 *  document, so a paste waits for the user to view the tab. */
export function whenTabForeground(): Promise<void> {
  const ready = (): boolean => document.visibilityState === "visible" && document.hasFocus();
  if (ready()) return Promise.resolve();
  return new Promise((resolve) => {
    const check = (): void => {
      if (!ready()) return;
      document.removeEventListener("visibilitychange", check);
      window.removeEventListener("focus", check);
      resolve();
    };
    document.addEventListener("visibilitychange", check);
    window.addEventListener("focus", check);
  });
}
