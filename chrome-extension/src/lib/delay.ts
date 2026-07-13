/** Resolve after `ms`. Shared by the content scripts and the service worker
 *  (they bundle separately, but the source lives in one place). */
export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
