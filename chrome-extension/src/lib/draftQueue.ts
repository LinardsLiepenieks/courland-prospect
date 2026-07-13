// A concurrency-capped, eviction-safe queue for opening "Draft for N" background
// tabs. LinkedIn soft-throttles a burst of parallel page loads — past roughly the
// fifth simultaneous tab, new tabs fail to load — so instead of opening the whole
// batch at once we keep at most MAX_CONCURRENT tabs in their heavy load+generate
// phase and open the next only when a tab reports it finished (`releaseSlot`,
// driven by the content script signalling the service worker).
//
// The job lives in chrome.storage.session so it survives the MV3 service worker
// being evicted between those signals (a long setTimeout loop in the worker would
// be killed mid-batch). `nudgeDraftQueue` is a periodic backstop: if an in-flight
// tab died without signalling, it tops the queue back up so the batch still drains.

import { delay } from "./delay";

const QUEUE_KEY = "draftQueue";

// Serialize every queue mutation within this service-worker instance. Message
// handlers run concurrently and interleave at awaits, so an unguarded
// read-modify-write of the persisted job could double-open an index or drop one
// when several tabs report done at once. The chain resets on SW eviction, which
// is harmless — storage is the source of truth and a fresh worker has no
// concurrent handlers to race.
let lock: Promise<unknown> = Promise.resolve();
function withLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = lock.then(fn, fn);
  lock = run.catch(() => {});
  return run;
}

/** Max draft tabs loading/generating at once — a margin under the ~5-tab point
 *  where LinkedIn starts failing to load new tabs. */
const MAX_CONCURRENT = 3;

/** Jittered gap between opening tabs, so even the initial fill isn't a hard burst
 *  and repeated runs don't settle into a fixed, detectable rhythm. */
const OPEN_JITTER_MIN_MS = 500;
const OPEN_JITTER_MAX_MS = 1500;

/** If no tab has been opened in this long while work remains, assume an in-flight
 *  tab died without releasing its slot and open one anyway (stall backstop). Kept
 *  well above a tab's heavy phase so it never races a still-working tab. */
const STALL_MS = 60_000;

interface DraftJob {
  pitchId: number;
  /** Inbox filter pill token to re-apply in each tab, or null for the default view. */
  filter: string | null;
  /** Absolute conversation indices not yet opened. */
  remaining: number[];
  /** Free concurrency slots (starts at MAX_CONCURRENT, spent per open, returned on release). */
  slots: number;
  /** Timestamp of the last tab open, for the stall backstop. */
  lastOpenAt: number;
}

async function readJob(): Promise<DraftJob | null> {
  const stored = await chrome.storage.session
    .get(QUEUE_KEY)
    .catch(() => ({}) as Record<string, unknown>);
  return (stored[QUEUE_KEY] as DraftJob | undefined) ?? null;
}

async function writeJob(job: DraftJob | null): Promise<void> {
  if (job) await chrome.storage.session.set({ [QUEUE_KEY]: job }).catch(() => {});
  else await chrome.storage.session.remove(QUEUE_KEY).catch(() => {});
}

function tabUrl(job: DraftJob, index: number): string {
  const filterHash = job.filter ? `&filter=${encodeURIComponent(job.filter)}` : "";
  return `https://www.linkedin.com/messaging/#cpdraft=${encodeURIComponent(
    String(job.pitchId),
  )}&i=${index}${filterHash}`;
}

/** Open tabs while free slots and work remain, popping indices off `remaining`,
 *  jittered between opens. Persists after each open so an eviction mid-drain
 *  resumes from the right place; clears the job once nothing remains to open. */
async function pump(job: DraftJob): Promise<void> {
  while (job.slots > 0 && job.remaining.length > 0) {
    const index = job.remaining.shift() as number;
    job.slots -= 1;
    job.lastOpenAt = Date.now();
    try {
      await chrome.tabs.create({ url: tabUrl(job, index), active: false });
    } catch {
      // A tabs.create failure shouldn't stall the rest — the slot is spent and
      // will be reclaimed by the next release or the stall backstop.
    }
    await writeJob(job);
    await delay(OPEN_JITTER_MIN_MS + Math.random() * (OPEN_JITTER_MAX_MS - OPEN_JITTER_MIN_MS));
  }
  await writeJob(job.remaining.length === 0 ? null : job);
}

/** Start a batch: queue conversation indices `start … start + count - 1` and open
 *  up to MAX_CONCURRENT immediately; the rest open as tabs report done. Returns
 *  how many were queued (the whole batch — they open over time, not all at once). */
export function startDraftBatch(
  pitchId: number,
  filter: string | null,
  start: number,
  count: number,
): Promise<number> {
  return withLock(async () => {
    const remaining: number[] = [];
    for (let i = 0; i < count; i++) remaining.push(start + i);
    const job: DraftJob = { pitchId, filter, remaining, slots: MAX_CONCURRENT, lastOpenAt: 0 };
    await writeJob(job);
    await pump(job);
    return count;
  });
}

/** A draft tab finished its heavy load+generate phase: return its slot and open
 *  the next queued tab. No-op when no batch is active. */
export function releaseSlot(): Promise<void> {
  return withLock(async () => {
    const job = await readJob();
    if (!job) return;
    job.slots = Math.min(MAX_CONCURRENT, job.slots + 1);
    await pump(job);
  });
}

/** Backstop (run from the periodic alarm): if work remains but nothing has opened
 *  in STALL_MS, an in-flight tab likely died without releasing — reclaim a slot
 *  and open one so the batch can't stall forever. */
export function nudgeDraftQueue(): Promise<void> {
  return withLock(async () => {
    const job = await readJob();
    if (!job || job.remaining.length === 0) return;
    if (Date.now() - job.lastOpenAt > STALL_MS) {
      job.slots = Math.max(job.slots, 1);
      await pump(job);
    }
  });
}
