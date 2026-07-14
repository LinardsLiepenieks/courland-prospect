// A concurrency-capped, eviction-safe queue for opening pre-filled "review" tabs.
// The main inbox tab cycles the conversations and generates their drafts; as each
// draft becomes ready it asks the service worker (via `enqueueReviewTab`) to open
// that conversation in its own tab, where the cached draft gets pasted in. URLs
// therefore ARRIVE OVER TIME (pipelined behind generation), not all at once.
//
// LinkedIn soft-throttles a burst of parallel page loads — past roughly the fifth
// simultaneous tab, new tabs fail to load — so we keep at most MAX_CONCURRENT tabs
// in their (now brief) load phase and open the next only when a tab reports it has
// loaded and read its draft (`releaseReviewSlot`). Unlike the old drafter, these
// tabs do no generation; a slot frees as soon as the page has loaded, not after an
// AI round-trip.
//
// The job lives in chrome.storage.session so it survives the MV3 service worker
// being evicted between signals. `nudgeReviewQueue` is a periodic backstop: if an
// in-flight tab died without signalling, it tops the queue back up so the batch
// still drains.

import { delay } from "./delay";
import { FILL_HASH } from "./storageKeys";

const QUEUE_KEY = "reviewQueue";

// Serialize every queue mutation within this service-worker instance. Message
// handlers run concurrently and interleave at awaits, so an unguarded
// read-modify-write of the persisted job could double-open a URL or drop one when
// several signals arrive at once. The chain resets on SW eviction, which is
// harmless — storage is the source of truth and a fresh worker has no concurrent
// handlers to race.
let lock: Promise<unknown> = Promise.resolve();
function withLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = lock.then(fn, fn);
  lock = run.catch(() => {});
  return run;
}

/** Max review tabs loading at once. Kept well below the ~5-tab point where LinkedIn
 *  starts failing to load new tabs: LinkedIn's throttling is behavioural (burst
 *  volume + timing), so a low ceiling plus the wide gap below keeps a batch reading
 *  as a human-paced trickle rather than an automated burst. */
const MAX_CONCURRENT = 3;

/** Jittered gap between opening tabs. Deliberately wide so a batch of ready drafts
 *  drips out over minutes instead of seconds — the burst signature, not a hard
 *  numeric cap, is what LinkedIn's 2026 detection flags — and repeated runs never
 *  settle into a fixed, detectable rhythm. */
const OPEN_JITTER_MIN_MS = 3_000;
const OPEN_JITTER_MAX_MS = 7_000;

/** If work remains but nothing has opened in this long, assume an in-flight tab
 *  died without releasing its slot and open one anyway (stall backstop). Kept well
 *  above a tab's load phase so it never races a still-loading tab. */
const STALL_MS = 60_000;

interface ReviewJob {
  /** Thread URLs queued to open but not yet opened, in arrival order. */
  remaining: string[];
  /** Free concurrency slots (starts at MAX_CONCURRENT, spent per open, returned on release). */
  slots: number;
  /** Timestamp of the last tab open, for the stall backstop. */
  lastOpenAt: number;
}

/** The queue's canonical starting state: nothing queued, every slot free. */
function freshJob(): ReviewJob {
  return { remaining: [], slots: MAX_CONCURRENT, lastOpenAt: 0 };
}

async function readJob(): Promise<ReviewJob | null> {
  const stored = await chrome.storage.session
    .get(QUEUE_KEY)
    .catch(() => ({}) as Record<string, unknown>);
  return (stored[QUEUE_KEY] as ReviewJob | undefined) ?? null;
}

async function writeJob(job: ReviewJob): Promise<void> {
  await chrome.storage.session.set({ [QUEUE_KEY]: job }).catch(() => {});
}

function tabUrl(url: string): string {
  return `${url}#${FILL_HASH}`;
}

/** Open tabs while free slots and work remain, popping URLs off `remaining`,
 *  jittered between opens. Persists after each open so an eviction mid-drain
 *  resumes from the right place. The job is left in place (not cleared) even when
 *  `remaining` empties, because more URLs arrive over the life of a batch as
 *  generation completes; the next batch resets it via `resetReviewQueue`. */
async function pump(job: ReviewJob): Promise<void> {
  while (job.slots > 0 && job.remaining.length > 0) {
    const url = job.remaining.shift() as string;
    job.slots -= 1;
    job.lastOpenAt = Date.now();
    try {
      await chrome.tabs.create({ url: tabUrl(url), active: false });
    } catch {
      // A tabs.create failure shouldn't stall the rest — the slot is spent and
      // will be reclaimed by the next release or the stall backstop.
    }
    await writeJob(job);
    await delay(OPEN_JITTER_MIN_MS + Math.random() * (OPEN_JITTER_MAX_MS - OPEN_JITTER_MIN_MS));
  }
  await writeJob(job);
}

/** Reset the queue for a new batch: drop any leftover URLs and restore every slot.
 *  Called when the main tab starts a fresh "Draft for" cycle, so a new run never
 *  inherits the last one's queue state. */
export function resetReviewQueue(): Promise<void> {
  return withLock(async () => {
    await writeJob(freshJob());
  });
}

/** Queue one conversation's thread URL to open as a review tab and pump. Creates a
 *  fresh job if none exists (e.g. the SW was evicted and lost the reset). */
export function enqueueReviewTab(url: string): Promise<void> {
  return withLock(async () => {
    const job = (await readJob()) ?? freshJob();
    job.remaining.push(url);
    await pump(job);
  });
}

/** A review tab finished loading and read its draft: return its slot and open the
 *  next queued tab. No-op when no batch is active. */
export function releaseReviewSlot(): Promise<void> {
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
export function nudgeReviewQueue(): Promise<void> {
  return withLock(async () => {
    const job = await readJob();
    if (!job || job.remaining.length === 0) return;
    if (Date.now() - job.lastOpenAt > STALL_MS) {
      job.slots = Math.max(job.slots, 1);
      await pump(job);
    }
  });
}
