// The content script's one channel to the service worker. All cross-origin
// fetches to the loopback app happen in the SW (MV3 forbids them in content
// scripts), so every request goes through here.

import type { Request, Response } from "../lib/types";

/**
 * Send a request to the service worker and await its response.
 *
 * `chrome.runtime.sendMessage` *rejects* (rather than resolving to `{ok:false}`)
 * when the extension context is invalidated or the SW port closes — which happens
 * routinely, since the extension reloads on update and invalidates already-open
 * LinkedIn tabs. We convert that into the normal `{ok:false}` path so callers
 * never face an unhandled rejection.
 */
export async function send<T>(msg: Request): Promise<Response<T>> {
  try {
    return (await chrome.runtime.sendMessage(msg)) as Response<T>;
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** Turn a raw messaging error into a line the user can act on. Distinguishes a
 *  dead extension context (needs a tab reload) from the app being down. Shared by
 *  the capture widget and the draft control. */
export function friendlyError(error: string): string {
  const e = error.toLowerCase();
  if (
    e.includes("context invalidated") ||
    e.includes("receiving end") ||
    e.includes("message port closed")
  ) {
    return "Reload this LinkedIn tab to reconnect.";
  }
  if (e.includes("failed to fetch") || e.includes("networkerror") || e.includes("refused")) {
    return "Courland isn't running.";
  }
  return error;
}
