// Service worker: the only place cross-origin fetches to the loopback server
// happen. The content script messages us; we attach the token and call the app.

import { baseUrl, getConfig, invalidateConfig } from "./lib/config";
import { nudgeDraftQueue, releaseSlot, startDraftBatch } from "./lib/draftQueue";
import { drain, enqueue, remove } from "./lib/outbox";
import type { CaptureOutcome, OutboxItem, Request, Response } from "./lib/types";

const TOKEN_HEADER = "x-courland-token";

/** chrome.alarms name for the periodic heartbeat (which also flushes the outbox).
 *  Kept as the original name so upgrading installs don't orphan a stale alarm. */
const DRAIN_ALARM = "courland-drain-outbox";

/** One fetch against the loopback server with the shared token attached. */
async function fetchWith(cfg: Awaited<ReturnType<typeof getConfig>>, path: string, init: RequestInit) {
  return fetch(`${baseUrl(cfg)}${path}`, {
    ...init,
    headers: {
      ...(init.headers ?? {}),
      [TOKEN_HEADER]: cfg.token,
      "content-type": "application/json",
    },
  });
}

/** Fetch against the loopback server, retrying once with a freshly-read config
 *  when the cached port/token looks stale. A connection refusal (the app
 *  restarted on a different port) rejects; a rotated token comes back 401/403.
 *  In both cases the app has rewritten config.json, so re-reading it and
 *  retrying recovers without waiting for the service worker to be evicted. */
async function call(path: string, init: RequestInit = {}): Promise<globalThis.Response> {
  try {
    const res = await fetchWith(await getConfig(), path, init);
    if (res.status === 401 || res.status === 403) {
      invalidateConfig();
      return fetchWith(await getConfig(), path, init);
    }
    return res;
  } catch {
    invalidateConfig();
    return fetchWith(await getConfig(), path, init);
  }
}

/** Check in with the app so it knows we're alive, and flush anything the outbox
 *  is holding from while it was closed. */
async function ping(): Promise<void> {
  try {
    await call("/health");
  } catch {
    // App may not be running yet — harmless.
  }
  void drainOutbox();
}

/** Deliver a batch to `POST /messages`. Returns the backend's `{stored, skipped}`
 *  counts on any 2xx (batch accepted, safe to clear), or `null` on a non-2xx or a
 *  connection refusal (app down) so the rows stay queued for the next drain. */
async function postMessages(
  items: OutboxItem[],
): Promise<{ stored: number; skipped: number } | null> {
  try {
    const res = await call("/messages", { method: "POST", body: JSON.stringify(items) });
    if (!res.ok) return null;
    return (await res.json()) as { stored: number; skipped: number };
  } catch {
    return null;
  }
}

function drainOutbox(): Promise<void> {
  return drain(async (items) => (await postMessages(items)) !== null);
}

// Wake-ups: (re)arm the periodic heartbeat alarm, then check in + flush. The
// alarm is both the liveness pulse the app's gate watches for and the backstop
// that eventually delivers captures made while the app was closed. ~30s so the
// gate learns we're alive quickly; Chrome may clamp toward 60s, which the app's
// freshness window tolerates.
function wake(): void {
  chrome.alarms.create(DRAIN_ALARM, { periodInMinutes: 0.5 });
  void ping();
}
chrome.runtime.onInstalled.addListener(wake);
chrome.runtime.onStartup.addListener(wake);
chrome.alarms.onAlarm.addListener((alarm) => {
  // ping() hits /health (the heartbeat the gate reads) then drains the outbox.
  // nudgeDraftQueue() is the backstop that reopens a stalled draft batch.
  if (alarm.name === DRAIN_ALARM) {
    void ping();
    void nudgeDraftQueue();
  }
});

async function handle(msg: Request): Promise<Response<unknown>> {
  try {
    if (msg.type === "checkin") {
      // A content script on a LinkedIn tab telling us it's alive. Ping /health
      // so the app's gate verifies readiness immediately instead of waiting for
      // the next alarm. Fire-and-forget; the caller ignores the response.
      void ping();
      return { ok: true, data: null };
    }
    if (msg.type === "listPitches") {
      const res = await call("/pitches");
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "addProspect") {
      const res = await call("/prospects", {
        method: "POST",
        body: JSON.stringify(msg.payload),
      });
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "openThreads") {
      // Queue `count` background tabs on the messaging inbox, each tagged (via the
      // URL hash) with the pitch + its target conversation index. The batch counts
      // down from the selected conversation, so the indices run `start … start +
      // count - 1`. Each tab opens that conversation itself (LinkedIn's list rows
      // carry no thread URL to hand off) and drafts. Content scripts can't open
      // tabs (popup-blocked); the SW can, with no extra permission for tabs.create.
      // The queue caps how many load at once — LinkedIn soft-throttles a burst of
      // parallel tab loads — opening the rest as each reports done (draftSlotFree).
      const { pitchId, count, start, filter } = msg.payload;
      const opened = await startDraftBatch(pitchId, filter, start, count);
      return { ok: true, data: { opened } };
    }
    if (msg.type === "draftReply") {
      const res = await call("/draft", { method: "POST", body: JSON.stringify(msg.payload) });
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "draftSlotFree") {
      // A draft tab finished its heavy load+generate phase — open the next queued
      // tab. Fire-and-forget; the tab ignores the response.
      void releaseSlot();
      return { ok: true, data: null };
    }
    // queueMessages — durably queue first (so a closed app never loses a
    // capture), then attempt an immediate targeted delivery of THESE items so we
    // can report a precise outcome for the toast. On success clear them and kick
    // a background drain for any older backlog; on failure they stay queued and
    // the periodic drain retries.
    const { linkedin_url, messages } = msg.payload;
    const items: OutboxItem[] = messages.map((m) => ({ ...m, linkedin_url }));
    await enqueue(items);
    const counts = await postMessages(items);
    if (counts) {
      await remove(items);
      void drainOutbox();
      const outcome: CaptureOutcome = {
        outcome: counts.stored > 0 ? "stored" : "skipped",
        ...counts,
      };
      return { ok: true, data: outcome };
    }
    const offline: CaptureOutcome = { outcome: "offline", stored: 0, skipped: 0 };
    return { ok: true, data: offline };
  } catch (e) {
    // A connection refusal (app closed) lands here too.
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

chrome.runtime.onMessage.addListener((msg: Request, _sender, sendResponse) => {
  handle(msg).then(sendResponse);
  return true; // keep the message channel open for the async response
});
