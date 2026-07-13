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
