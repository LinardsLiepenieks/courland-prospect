// The LinkedIn "comment run" worker roles that live in the page. The SERVICE
// WORKER orchestrates a run — it polls the app, opens these tabs, and paces the
// posting (see background.ts). The two roles here are the DOM-facing halves it
// can't do itself:
//
//  1. SCRAPE tab (opened by the SW on the feed home or a watched profile's
//     recent-activity, tagged `#cpscrape`): scrape the page's posts and report
//     them back, then the SW closes the tab. No drafting/persistence happens here.
//
//  2. POST tab (opened by the SW on a post's permalink, tagged
//     `#cppost=<permalink>`): read the approved comment cached for this post, open
//     its comment box, write the comment, and SUBMIT it — the auto-post — then
//     report the outcome (posted / failed) and let the SW close the tab.
//
// All DOM heuristics live in ./linkedin.

import {
  canonicalPostPermalink,
  clickCopyLinkItem,
  closeOpenMenu,
  commentAlreadyPresent,
  commentBoxHasText,
  findCommentEditor,
  isPromotedBlock,
  nextUnseenPost,
  normalizePostUrl,
  openCommentComposer,
  openPostControlMenu,
  postAuthorName,
  postBodyText,
  postContainerForUrl,
  postPermalink,
  submitComment,
  writeCommentBox,
} from "./linkedin";
import { POST_KEYS, requestHeal } from "./heal";
import { send } from "./bridge";
import { whenTabForeground } from "./dom";
import { showToast } from "./toast";
import { hideDraftOverlay, showDraftOverlay } from "./overlay";
import { delay } from "../lib/delay";
import { peekDraft } from "../lib/draftStore";
import { DRAFT_NS_POST } from "../lib/storageKeys";
import type { ScrapedPost } from "../lib/types";

// ── Scrape tab: collect posts + their permalinks ─────────────────────────────
// Walk posts ONE AT A TIME — find the next post, read its text/author, capture its
// link, then move on, scrolling to load more, until we hit the target count or run
// out. Link capture is DOM-FIRST: the post's activity URN is read straight off the
// DOM (its own / ancestor `data-urn`, or a `/feed/update/`·`/posts/` anchor), which
// needs no clipboard and yields the real `urn:li:activity:<id>`. The ⋯ "Copy link to
// post" → clipboard route is only a last-resort fallback: a content script can't get
// the user-activation the async clipboard API demands, so those reads time out here
// (the reference tool avoids this by driving Chrome over CDP with granted clipboard
// permission — a path our no-CDP design doesn't have).

/** Scrape diagnostics — when true, logs per-post capture outcomes to the scrape
 *  tab's console (prefix `[cp-scrape]`) so a low yield / bad link can be traced to
 *  its cause (no DOM URN, menu didn't open, clipboard didn't change, etc.). Kept in
 *  place (off by default) because post capture is a fragile surface worth being able
 *  to inspect quickly — flip to `true` and rebuild to trace a bad run. */
const SCRAPE_DEBUG = false;
function dbg(...args: unknown[]): void {
  if (SCRAPE_DEBUG) console.debug("[cp-scrape]", ...args);
}

/** Let a freshly-opened scrape tab settle (SPA render) before reading it. */
const SCRAPE_SETTLE_MS = 3000;
/** Cap on scroll passes before giving up looking for more posts. */
const SCRAPE_MAX_SCROLLS = 40;
/** Wall-clock ceiling on one page's scrape, so the tab always reports (and the SW
 *  moves on) well within the SW's own scrape timeout, however many posts remain. */
const SCRAPE_DEADLINE_MS = 90_000;
/** Waits around the ⋯-menu / copy-link interaction (LinkedIn animates the menu). */
const MENU_OPEN_WAIT_MS = 800;
/** How long to wait for the clipboard fallback to yield a FRESH value, and how often
 *  to poll. Kept short because this path reliably fails in a content script (no user
 *  activation for the async clipboard API) — the DOM path is the real source, so we
 *  don't want a dead fallback eating the scrape's wall-clock budget per post.
 *  We accept only a value that CHANGED from the pre-click snapshot, so a
 *  slow copy can't leave us reading (and mis-pairing) a stale URL. */
const COPY_CHANGE_DEADLINE_MS = 1_200;
const COPY_POLL_MS = 150;

/**
 * Scrape-tab entry, run in a FOREGROUND tab the SW opened on the feed or a profile's
 * recent-activity (tagged `#cpscrape=<target>`). Collects up to `target` posts —
 * each with a captured permalink — and reports them to the SW, which then closes the
 * tab. Always reports (whatever it gathered, or `[]` on failure) so the SW's waiter
 * resolves instead of waiting out its timeout.
 */
export async function runScrapeMode(target: number): Promise<void> {
  let posts: ScrapedPost[] = [];
  try {
    // The clipboard read + menu interaction need the tab focused; the SW opens it
    // active, so this resolves promptly.
    await whenTabForeground();
    showToast("info", "Scanning the feed for posts…");
    await delay(SCRAPE_SETTLE_MS);

    posts = await collectPosts(target);
    if (posts.length === 0) {
      // Found nothing — the post selectors have likely rotated, or this feed is in a
      // language our compiled "Feed post" anchor doesn't match. Hand the live DOM to
      // the app's Claude Code to repair the post selectors, then retry the scrape
      // once. `requestHeal` is capped/deduped per session, so this can't loop.
      const healed = await requestHeal(POST_KEYS);
      if (healed) posts = await collectPosts(target);
    }
    if (posts.length > 0) {
      showToast("success", `Found ${posts.length} post${posts.length === 1 ? "" : "s"} to draft.`);
    } else {
      showToast("info", "No posts found on this page.");
    }
  } catch {
    // Best-effort — report whatever we gathered.
  } finally {
    await send({ type: "postsScraped", payload: { posts } });
  }
}

/** Walk the page collecting up to `target` posts (each with a captured permalink),
 *  scrolling to load more, bounded by a scroll cap, an iteration guard, and a
 *  wall-clock deadline so it always returns. Deduped by permalink. */
async function collectPosts(target: number): Promise<ScrapedPost[]> {
  const posts: ScrapedPost[] = [];
  const seenPermalinks = new Set<string>();
  const seenBlocks = new Set<string>();
  const deadline = Date.now() + SCRAPE_DEADLINE_MS;
  let scrolls = 0;
  let guard = 0;
  const guardMax = target * 8 + 60;
  // Whether we've found ANY post block yet. If we haven't after a couple of scrolls,
  // the selectors are broken (not just "scrolled past the end") — bail fast so the
  // caller heals quickly, instead of scrolling uselessly toward the deadline.
  let sawAnyContainer = false;
  while (
    posts.length < target &&
    scrolls < SCRAPE_MAX_SCROLLS &&
    guard < guardMax &&
    Date.now() < deadline
  ) {
    guard += 1;
    const block = nextUnseenPost(seenBlocks);
    if (!block) {
      if (!sawAnyContainer && scrolls >= 2) break; // nothing here to find — heal.
      scrollFeed();
      await delay(2000 + Math.random() * 800);
      scrolls += 1;
      continue;
    }
    sawAnyContainer = true;
    if (isPromotedBlock(block)) {
      dbg("skip: promoted/ad block");
      continue; // ad — skip, no link
    }
    const text = postBodyText(block);
    if (!text || text.length < 30) {
      dbg("skip: text too thin", { len: text?.length ?? 0 });
      continue; // non-post / too thin to comment on
    }
    const permalink = await capturePostLink(block);
    if (!permalink) {
      dbg("skip: no permalink captured", { textStart: text.slice(0, 60) });
      continue;
    }
    if (seenPermalinks.has(permalink)) {
      dbg("skip: duplicate permalink", { permalink });
      continue;
    }
    seenPermalinks.add(permalink);
    dbg("captured", { n: posts.length + 1, permalink });
    posts.push({ permalink, author_name: postAuthorName(block), text });
    await delay(400 + Math.random() * 300);
  }
  dbg("done", { captured: posts.length, target, scrolls, guard, sawAnyContainer });
  return posts;
}

/**
 * Capture one post's permalink. Fast path: straight from the DOM when LinkedIn
 * exposes it. Otherwise the reference's path — open the post's ⋯ menu, click "Copy
 * link to post", read the clipboard, Esc — canonicalized to the `/feed/update/urn:…`
 * form the post tab matches on. Empty string when it can't be captured (the post is
 * then skipped, since we couldn't open it to comment).
 */
async function capturePostLink(block: HTMLElement): Promise<string> {
  const dom = postPermalink(block);
  if (dom) {
    dbg("link via DOM urn", { dom });
    return dom;
  }
  try {
    // Clear any menu still open from the previous post BEFORE opening this one, so
    // its "Copy link" item can't be the one that gets clicked.
    closeOpenMenu();
    // Snapshot the clipboard before the copy so we can require a FRESH value: if the
    // copy is slow (or clicked the wrong/stale menu item and did nothing), the
    // clipboard still holds the prior value — accepting that would pair this post's
    // text with another post's link. We take only a value that actually changed.
    const before = await readClipboard();
    if (!openPostControlMenu(block)) {
      dbg("copy path: ⋯ control-menu button not found");
      return "";
    }
    await delay(MENU_OPEN_WAIT_MS);
    if (!clickCopyLinkItem()) {
      dbg("copy path: 'Copy link to post' item not found in open menu");
      closeOpenMenu();
      return "";
    }
    const raw = await readClipboardCopied(before);
    closeOpenMenu();
    if (!raw) {
      // Either the clipboard never changed (copy silently failed) or reading it was
      // blocked. Clipboard reads from a content script need the tab focused and can
      // be denied — if this is the common case, the DOM-URN path is the reliable one.
      dbg("copy path: clipboard didn't yield a fresh value", { beforeLen: before.length });
      return "";
    }
    const url = raw.split("?")[0].trim();
    if (url.includes("linkedin.com/posts/") || url.includes("/feed/update/")) {
      const canonical = canonicalPostPermalink(url);
      dbg("link via clipboard", { raw: url, canonical });
      return canonical;
    }
    dbg("copy path: clipboard value isn't a post URL", { raw: url.slice(0, 80) });
    return "";
  } catch (e) {
    dbg("copy path: threw", { error: e instanceof Error ? e.message : String(e) });
    closeOpenMenu();
    return "";
  }
}

/** Poll the clipboard until it differs from `before` (its value just before we
 *  clicked "Copy link"), so we read the freshly-copied URL rather than a stale one.
 *  Returns the changed value, or "" if it never changed within the window (the post
 *  is then skipped rather than mis-paired with an old link). */
async function readClipboardCopied(before: string): Promise<string> {
  const deadline = Date.now() + COPY_CHANGE_DEADLINE_MS;
  while (Date.now() < deadline) {
    const cur = await readClipboard();
    if (cur && cur !== before) return cur;
    await delay(COPY_POLL_MS);
  }
  return "";
}

/** Read the clipboard (where "Copy link to post" just wrote the URL). Prefers the
 *  async Clipboard API — allowed here via the extension's `clipboardRead` permission
 *  in a focused tab — and falls back to an `execCommand("paste")` into a throwaway
 *  textarea if that's blocked. Returns "" on failure. */
async function readClipboard(): Promise<string> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.focus();
      const ok = document.execCommand("paste");
      const val = ta.value;
      ta.remove();
      return ok ? val : "";
    } catch {
      return "";
    }
  }
}

/** Scroll the largest scrollable container to its bottom to load more posts — the
 *  feed's scroll host isn't always the window (the reference finds it the same way).
 *  Falls back to the document scroller. */
function scrollFeed(): void {
  let best: Element | null = null;
  let bestHeight = 0;
  for (const el of Array.from(document.querySelectorAll<HTMLElement>("*"))) {
    const oy = getComputedStyle(el).overflowY;
    if (/(auto|scroll)/.test(oy) && el.scrollHeight > el.clientHeight + 200 && el.scrollHeight > bestHeight) {
      bestHeight = el.scrollHeight;
      best = el;
    }
  }
  const target = (best ?? document.scrollingElement ?? document.documentElement) as HTMLElement;
  target.scrollTop = target.scrollHeight;
}

// ── Post tab: write + submit the approved comment ────────────────────────────

/** How long to keep trying to open the comment box before giving up. */
const OPEN_DEADLINE_MS = 30_000;
const OPEN_POLL_MS = 500;
/** How long to wait for the submit button to enable after writing the comment. */
const SUBMIT_DEADLINE_MS = 4_000;
const SUBMIT_POLL_MS = 300;
/** How long to wait for confirmation the submit actually took — either our comment
 *  appearing in the thread (primary) or the box clearing (fallback). Generous, since
 *  a slow render that misses this window would record a false "failed"; the
 *  idempotency check on the next attempt is the backstop against a resulting
 *  duplicate. */
const CONFIRM_DEADLINE_MS = 10_000;
const CONFIRM_POLL_MS = 400;

/**
 * Post-tab entry, run in a tab the SW opened straight on a post's permalink
 * (tagged `#cppost=<permalink>`). Reads the approved comment cached for this post
 * (keyed by the canonical permalink the SW passed in the hash — read from the hash
 * rather than recomputed from `location.href` so a post-load URL rewrite can't
 * desync the key), opens the post's comment box, writes the comment, submits it,
 * and reports the outcome to the SW. Reports EXACTLY once so the SW's waiter always
 * resolves.
 *
 * The composer lookup is SCOPED to the target post's container: a permalink page
 * also renders recommended posts, each with its own comment button/editor, so a
 * document-wide lookup could submit into the wrong post's box.
 */
export async function runCommentPostMode(key: string | null): Promise<void> {
  const url = key ?? normalizePostUrl(location.href);
  let reported = false;
  const report = (status: "posted" | "failed", error = ""): void => {
    if (reported) return;
    reported = true;
    void send({ type: "commentPosted", payload: { status, error } });
  };

  // Mask LinkedIn's load churn while the tab comes up; lifts before the write.
  showDraftOverlay();
  try {
    const comment = await peekDraft(DRAFT_NS_POST, url);
    if (comment == null) {
      report("failed", "no comment was cached for this post");
      return;
    }

    hideDraftOverlay();
    // The SW opens the post tab focused (execCommand acts on the active document),
    // so this resolves promptly.
    await whenTabForeground();

    // Resolve the target post's OWN container (the one whose URN matches this
    // permalink) and open its composer. Crucially we require that exact container —
    // never a document-wide fallback: a permalink page also renders recommended
    // posts, each with its own comment box, and auto-submitting is irreversible, so
    // if we can't positively identify THIS post's box we must not post into some
    // other one. The container + editor mount lazily, so re-resolve each pass.
    const openDeadline = Date.now() + OPEN_DEADLINE_MS;
    let scope: HTMLElement | null = null;
    let editor: HTMLElement | null = null;
    while (Date.now() < openDeadline) {
      const container = postContainerForUrl(url);
      if (container) {
        openCommentComposer(container);
        const found = findCommentEditor(container);
        if (found) {
          scope = container;
          editor = found;
          break;
        }
      }
      await delay(OPEN_POLL_MS);
    }
    if (!scope || !editor) {
      report("failed", "couldn't locate this post's comment box");
      return;
    }

    // Idempotency: if THIS comment is already in the post's thread, a prior attempt
    // already posted it — most likely one whose confirmation timed out and was
    // recorded "failed", then re-queued. Posting again would duplicate a public
    // comment, so don't: report it posted (idempotently) so it leaves the queue and
    // the app records the permalink in its durable ledger. This is the reliable
    // source of truth — LinkedIn's own thread — not a timing heuristic.
    if (commentAlreadyPresent(scope, comment)) {
      showToast("info", "This comment is already posted.");
      report("posted");
      return;
    }

    // Never clobber a comment the user is already typing in this box.
    if (commentBoxHasText(scope)) {
      report("failed", "a comment was already in progress here");
      return;
    }
    if (!writeCommentBox(comment, scope)) {
      report("failed", "couldn't write the comment into the box");
      return;
    }

    // Submit once the button enables (LinkedIn disables it until the box registers
    // the text). Poll rather than a fixed wait so a slow editor doesn't miss it.
    const submitDeadline = Date.now() + SUBMIT_DEADLINE_MS;
    let submitted = false;
    while (Date.now() < submitDeadline) {
      if (submitComment(scope)) {
        submitted = true;
        break;
      }
      await delay(SUBMIT_POLL_MS);
    }
    if (!submitted) {
      report("failed", "the submit button never enabled");
      return;
    }

    if (await confirmSubmitted(scope, comment)) {
      showToast("success", "Comment posted.");
      report("posted");
    } else {
      // Neither signal fired in time — treat as failed so it stays retryable. On a
      // retry the idempotency check above catches a comment that DID post but was
      // too slow to confirm, so this can't lead to a duplicate comment.
      report("failed", "the comment didn't confirm as posted");
    }
  } catch (e) {
    report("failed", e instanceof Error ? e.message : String(e));
  } finally {
    hideDraftOverlay();
    // A safety net: if some path above returned without reporting, mark it failed
    // so the SW never waits out its timeout on a claimed draft.
    report("failed", "the post tab ended without posting");
  }
}

/** Confirm a submit landed, strongest signal first: our comment actually appearing
 *  in the post's thread (the reliable, timing-independent signal), falling back to
 *  the comment box clearing (LinkedIn empties it once the comment posts). A final
 *  presence check after the window catches a comment that rendered right at the
 *  deadline. */
async function confirmSubmitted(scope: ParentNode, comment: string): Promise<boolean> {
  const deadline = Date.now() + CONFIRM_DEADLINE_MS;
  while (Date.now() < deadline) {
    await delay(CONFIRM_POLL_MS);
    if (commentAlreadyPresent(scope, comment)) return true;
    if (!commentBoxHasText(scope)) return true;
  }
  return commentAlreadyPresent(scope, comment);
}
