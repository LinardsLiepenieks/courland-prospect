// Service worker: the only place cross-origin fetches to the loopback server
// happen. The content script messages us; we attach the token and call the app.

import { baseUrl, getConfig, invalidateConfig } from "./lib/config";
import {
  enqueueReviewTab,
  nudgeReviewQueue,
  releaseReviewSlot,
  resetReviewQueue,
} from "./lib/reviewQueue";
import { drain, enqueue, remove } from "./lib/outbox";
import { delay } from "./lib/delay";
import { putDraft, removeDraft } from "./lib/draftStore";
import { DRAFT_NS_POST, POST_COMMENT_HASH, SCRAPE_HASH } from "./lib/storageKeys";
import type {
  CaptureOutcome,
  OutboxItem,
  Request,
  Response,
  ScrapedPost,
  WatchedProfile,
} from "./lib/types";

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
  // nudgeReviewQueue() is the backstop that reopens a stalled review-tab batch.
  if (alarm.name === DRAIN_ALARM) {
    void ping();
    void nudgeReviewQueue();
    // Backstop for the commenter when no LinkedIn tab is focused to nudge us.
    void driveCommentWork();
  }
});

/** How long to wait for a scrape worker tab to collect its posts and report before
 *  giving up (and closing the tab). The scrape is interactive (open each post's menu,
 *  copy its link, scroll, repeat) AND may self-heal + retry once, so this sits above
 *  the tab's worst case: a fast-failed first pass + a heal + a full second pass. */
const SCRAPE_TIMEOUT_MS = 180_000;

/** In-flight page scrapes, keyed by the worker tab's id. A scrape opens a
 *  background tab on the feed / a profile's recent-activity (tagged `#cpscrape`);
 *  that tab's content script scrapes the posts and sends `postsScraped`, which the
 *  onMessage listener routes here by the sender tab's id to resolve the waiter. */
const pendingScrapes = new Map<number, (posts: ScrapedPost[]) => void>();

/** Open a page (the feed home or a profile's recent-activity) in a worker tab, wait
 *  for it to scrape and report its posts, then close it. Resolves to the scraped
 *  posts, or `[]` on any failure/timeout (a scrape is best-effort — the run just gets
 *  fewer candidates from this page). Always removes the tab.
 *
 *  The tab is opened FOREGROUND (`active: true`): LinkedIn only lays out and
 *  lazy-loads its feed/activity posts in a rendered tab, so a background tab reads as
 *  empty — the reference scraper brings the page to front for the same reason. It's
 *  briefly disruptive (the tab takes focus for a few seconds, then closes). */
async function scrapePage(url: string, target: number): Promise<ScrapedPost[]> {
  let tabId: number | undefined;
  try {
    const tab = await chrome.tabs.create({ url: `${url}#${SCRAPE_HASH}=${target}`, active: true });
    tabId = tab.id;
  } catch {
    return [];
  }
  if (tabId == null) return [];
  const id = tabId;
  return new Promise<ScrapedPost[]>((resolve) => {
    let settled = false;
    const finish = (posts: ScrapedPost[]): void => {
      if (settled) return;
      settled = true;
      pendingScrapes.delete(id);
      clearTimeout(timer);
      void chrome.tabs.remove(id).catch(() => {});
      resolve(posts);
    };
    const timer = setTimeout(() => finish([]), SCRAPE_TIMEOUT_MS);
    pendingScrapes.set(id, finish);
  });
}

// ── Comment worker ───────────────────────────────────────────────────────────
// The service worker orchestrates a whole comment run: the app can't push to the
// extension, so we POLL the app — on the periodic alarm and whenever a LinkedIn tab
// nudges us (`pollCommentWork`). Two phases, both idempotent and safe to re-run:
//
//   Scrape: claim a requested run, scrape the feed + each watched profile in a
//   background tab, and ask the app to draft + persist a comment per new post (the
//   app dedups and enforces the placed-draft budget via its response).
//
//   Post: claim ONE queued draft at a time, open its post in a focused tab,
//   auto-submit the approved comment, report the outcome, then pace with a jittered
//   gap before the next. Claiming one at a time means an evicted worker strands
//   nothing mid-post; the alarm resumes the rest.

/** Feed home — scraped like a profile page (the scrape tab reads whatever posts
 *  are rendered, so one mechanism covers both). */
const FEED_URL = "https://www.linkedin.com/feed/";

/** Jittered gap between auto-posts. Wide + randomized so a run reads as a human
 *  trickle, not an automated burst (the signature LinkedIn's detection flags). */
const POST_JITTER_MIN_MS = 12_000;
const POST_JITTER_MAX_MS = 30_000;

/** How long to wait for a post tab to open the box, submit, and report back. */
const POST_TAB_TIMEOUT_MS = 45_000;

/** Serialize the whole work pump: the alarm and every `pollCommentWork` call route
 *  into `driveCommentWork`, which must not run twice at once (it would double-scrape
 *  or race the post pacing). In-memory, so it resets to idle on SW eviction — the DB
 *  claims (requested→scraping, queued→posting) are the real cross-restart guard. */
let driving = false;

async function driveCommentWork(): Promise<void> {
  if (driving) return;
  driving = true;
  try {
    await runScrapePhase();
    await runPostPhase();
  } catch {
    // Best-effort; the next poll/alarm retries.
  } finally {
    driving = false;
  }
}

/** Claim a requested scrape (if any) and run it, always releasing the run to `idle`
 *  afterwards so the UI never sticks on "Scraping…" — even if scraping threw. */
async function runScrapePhase(): Promise<void> {
  const run = await claimRun();
  if (!run) return;
  try {
    await scrapeAndDraft(run.count, run.include_watchlist);
  } finally {
    await setRunStatus("idle");
  }
}

/** Per watched-profile scrape cap — a handful of a person's newest posts is all a
 *  run would ever reach; the feed carries the bulk of the budget. */
const PROFILE_SCRAPE_CAP = 5;

/** Scrape watched profiles (each in its own tab) then the feed, and drive the app
 *  to draft + persist a comment per candidate until the placed-draft budget is met
 *  or candidates run out. Only a freshly `created` draft consumes budget — an
 *  `exists`/`skipped` doesn't — so skips can't shrink the run while viable posts
 *  remain. */
async function scrapeAndDraft(count: number, includeWatchlist: boolean): Promise<void> {
  const perProfile: ScrapedPost[][] = [];
  if (includeWatchlist) {
    for (const w of await getWatched()) {
      const posts = await scrapePage(recentActivityUrl(w.linkedin_url), PROFILE_SCRAPE_CAP);
      if (posts.length > 0) perProfile.push(posts);
    }
  }
  // Over-gather so the model's skips don't starve the budget: scrape ~2× the target
  // (the reference's "gather more, then qualify"), then draft in priority order until
  // `count` comments are actually created. Watched profiles already contribute their
  // own candidates on top.
  const feed = await scrapePage(FEED_URL, count * 2);
  const candidates = prioritize(perProfile, feed);

  let created = 0;
  for (const post of candidates) {
    if (created >= count) break;
    if ((await createCommentDraft(post)) === "created") created += 1;
  }
}

/** Drain queued drafts, posting one at a time with a jittered gap. The gap sits
 *  AFTER a draft has been posted + reported (before the next claim), so an eviction
 *  during the gap leaves nothing mid-flight. */
async function runPostPhase(): Promise<void> {
  for (;;) {
    const drafts = await claimDrafts(1);
    if (drafts.length === 0) break;
    await postOneDraft(drafts[0]);
    await delay(POST_JITTER_MIN_MS + Math.random() * (POST_JITTER_MAX_MS - POST_JITTER_MIN_MS));
  }
}

/** Post one claimed draft: cache its comment for the post tab to read, open the
 *  post, wait for the tab to submit + report, then record the outcome. Always
 *  reports SOMETHING (defaulting to failed) so a claimed (`posting`) draft never
 *  stays stuck when the tab dies silently. */
async function postOneDraft(d: { id: number; permalink: string; comment: string }): Promise<void> {
  let status: "posted" | "failed" = "failed";
  let error = "the post tab didn't complete";
  try {
    await putDraft(DRAFT_NS_POST, d.permalink, d.comment);
    const outcome = await runPostTab(d.permalink);
    status = outcome.status;
    error = outcome.error;
  } catch (e) {
    error = e instanceof Error ? e.message : String(e);
  } finally {
    await removeDraft(DRAFT_NS_POST, d.permalink).catch(() => {});
    await reportDraftStatus(d.id, status, error);
  }
}

/** In-flight post tabs, keyed by tab id (like {@link pendingScrapes}). The post tab
 *  reports `commentPosted`; the onMessage listener routes it here. */
const pendingPosts = new Map<
  number,
  (o: { status: "posted" | "failed"; error: string }) => void
>();

/** Open a post's permalink in a FOREGROUND tab (execCommand needs the active
 *  document, so the auto-post tab must be focused), let its content script write +
 *  submit the approved comment, and resolve with the outcome it reports — or a
 *  failure on timeout. Always removes the tab it opened. */
async function runPostTab(
  permalink: string,
): Promise<{ status: "posted" | "failed"; error: string }> {
  let tabId: number | undefined;
  try {
    const url = `${permalink}#${POST_COMMENT_HASH}=${encodeURIComponent(permalink)}`;
    const tab = await chrome.tabs.create({ url, active: true });
    tabId = tab.id;
  } catch {
    return { status: "failed", error: "couldn't open the post tab" };
  }
  if (tabId == null) return { status: "failed", error: "post tab has no id" };
  const id = tabId;
  return new Promise((resolve) => {
    let settled = false;
    const finish = (o: { status: "posted" | "failed"; error: string }): void => {
      if (settled) return;
      settled = true;
      pendingPosts.delete(id);
      clearTimeout(timer);
      void chrome.tabs.remove(id).catch(() => {});
      resolve(o);
    };
    const timer = setTimeout(
      () => finish({ status: "failed", error: "timed out opening the comment box" }),
      POST_TAB_TIMEOUT_MS,
    );
    pendingPosts.set(id, finish);
  });
}

/** Order candidates so watched people come first (round-robin their newest posts,
 *  so one prolific person can't crowd out the rest), then the feed — deduped by
 *  permalink. */
function prioritize(perProfile: ScrapedPost[][], feed: ScrapedPost[]): ScrapedPost[] {
  const ordered: ScrapedPost[] = [];
  const seen = new Set<string>();
  const pushUnique = (p: ScrapedPost): void => {
    if (p.permalink.length > 0 && !seen.has(p.permalink)) {
      seen.add(p.permalink);
      ordered.push(p);
    }
  };
  const maxLen = perProfile.reduce((m, posts) => Math.max(m, posts.length), 0);
  for (let i = 0; i < maxLen; i++) {
    for (const posts of perProfile) if (i < posts.length) pushUnique(posts[i]);
  }
  for (const p of feed) pushUnique(p);
  return ordered;
}

/** Build a profile's recent-activity URL from its (possibly messy) profile URL,
 *  canonicalizing to `/in/<slug>/recent-activity/all/`. */
function recentActivityUrl(profileUrl: string): string {
  try {
    const u = new URL(profileUrl, "https://www.linkedin.com");
    const m = u.pathname.match(/\/in\/([^/]+)/);
    if (m) return `https://www.linkedin.com/in/${m[1]}/recent-activity/all/`;
  } catch {
    // fall through to the string fallback
  }
  const base = profileUrl.endsWith("/") ? profileUrl : `${profileUrl}/`;
  return `${base}recent-activity/all/`;
}

// HTTP wrappers for the commenter endpoints. All fail soft — a run just does less
// (skips the phase, posts fewer) rather than throwing out of the work pump.

async function claimRun(): Promise<{ count: number; include_watchlist: boolean } | null> {
  try {
    const res = await call("/comment-run/claim", { method: "POST" });
    if (!res.ok) return null;
    const data = (await res.json()) as {
      run: { count: number; include_watchlist: boolean } | null;
    };
    return data.run;
  } catch {
    return null;
  }
}

async function setRunStatus(status: "scraping" | "idle"): Promise<void> {
  try {
    await call("/comment-run/status", { method: "POST", body: JSON.stringify({ status }) });
  } catch {
    // The next claim/alarm reconciles.
  }
}

async function getWatched(): Promise<WatchedProfile[]> {
  try {
    const res = await call("/watched-profiles");
    if (!res.ok) return [];
    return (await res.json()) as WatchedProfile[];
  } catch {
    return [];
  }
}

/** Draft + persist a comment for one scraped post; returns the app's result string
 *  (`created` | `exists` | `skipped` | `error`). The caller counts `created`
 *  against the run's budget. */
async function createCommentDraft(post: ScrapedPost): Promise<string> {
  try {
    const res = await call("/comment-drafts", {
      method: "POST",
      body: JSON.stringify({
        permalink: post.permalink,
        author_name: post.author_name,
        post_text: post.text,
      }),
    });
    if (!res.ok) return "error";
    const data = (await res.json()) as { result?: string };
    return data.result ?? "error";
  } catch {
    return "error";
  }
}

async function claimDrafts(
  limit: number,
): Promise<Array<{ id: number; permalink: string; comment: string }>> {
  try {
    const res = await call("/comment-drafts/claim", {
      method: "POST",
      body: JSON.stringify({ limit }),
    });
    if (!res.ok) return [];
    const data = (await res.json()) as {
      drafts: Array<{ id: number; permalink: string; comment: string }>;
    };
    return data.drafts ?? [];
  } catch {
    return [];
  }
}

async function reportDraftStatus(
  id: number,
  status: "posted" | "failed",
  error: string,
): Promise<void> {
  try {
    await call("/comment-draft-status", {
      method: "POST",
      body: JSON.stringify({ id, status, error }),
    });
  } catch {
    // Best-effort; a lost report leaves the draft `posting` (visible in the app).
  }
}

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
    if (msg.type === "lookupProspect") {
      // Is the open thread's person already a prospect, and on which pitch? Read
      // by URL so the widget can show "Prospect of <pitch>" instead of the add
      // control, and drafting can use that prospect's own pitch.
      const res = await call(`/prospect?url=${encodeURIComponent(msg.payload.linkedin_url)}`);
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
    if (msg.type === "draftReply") {
      const res = await call("/draft", { method: "POST", body: JSON.stringify(msg.payload) });
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "resetReviewQueue") {
      // A tab is starting a fresh cycle — drop only THIS feature's leftover
      // review-tab URLs (by fill-hash), leaving the other feature's queued tabs and
      // the shared slot budget intact.
      await resetReviewQueue(msg.payload.hash);
      return { ok: true, data: null };
    }
    if (msg.type === "openReviewTab") {
      // One conversation's draft is ready and cached — open a pre-filled review tab
      // for it (content scripts can't open tabs; the SW can, no extra permission
      // for tabs.create). The queue caps how many load at once — LinkedIn
      // soft-throttles a burst of parallel tab loads — opening the rest as each
      // reports it loaded (reviewTabFilled).
      await enqueueReviewTab(msg.payload.url);
      return { ok: true, data: null };
    }
    if (msg.type === "reviewTabFilled") {
      // A review tab finished loading and read its draft — open the next queued
      // tab. Fire-and-forget; the tab ignores the response.
      void releaseReviewSlot();
      return { ok: true, data: null };
    }
    if (msg.type === "getSelectors") {
      // Persisted LinkedIn-selector overrides the content script merges over its
      // defaults at startup. Empty `{}` when nothing's been healed.
      const res = await call("/selectors");
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "healSelectors") {
      // A selector broke — hand the live page to the app so Claude Code can repair
      // it; returns the merged overrides for the content script to apply.
      const res = await call("/heal-selectors", {
        method: "POST",
        body: JSON.stringify(msg.payload),
      });
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      return { ok: true, data: await res.json() };
    }
    if (msg.type === "pollCommentWork") {
      // A LinkedIn tab nudging us to check the app for pending comment work (a
      // requested scrape or queued posts). Fire-and-forget; `driveCommentWork`
      // dedups concurrent calls.
      void driveCommentWork();
      return { ok: true, data: null };
    }
    if (msg.type === "postsScraped" || msg.type === "commentPosted") {
      // A worker tab (scrape / post) reporting its result. Normally intercepted in
      // the onMessage listener (which has the sender tab id to correlate); handled
      // here only so the message type is exhaustively narrowed. No-op.
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

chrome.runtime.onMessage.addListener((msg: Request, sender, sendResponse) => {
  // A scrape worker tab reporting its posts: route to the pending scrape by the
  // sender tab's id (which scrapePage keyed the waiter on), then close it.
  if (msg.type === "postsScraped") {
    const tabId = sender.tab?.id;
    if (tabId != null) pendingScrapes.get(tabId)?.(msg.payload.posts);
    sendResponse({ ok: true, data: null });
    return false; // handled synchronously
  }
  // A post tab reporting its auto-submit outcome: route to the pending post by the
  // sender tab's id (which runPostTab keyed the waiter on), then it closes the tab.
  if (msg.type === "commentPosted") {
    const tabId = sender.tab?.id;
    if (tabId != null) pendingPosts.get(tabId)?.(msg.payload);
    sendResponse({ ok: true, data: null });
    return false; // handled synchronously
  }
  handle(msg).then(sendResponse);
  return true; // keep the message channel open for the async response
});
