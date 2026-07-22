// Content-script entry: keep an "Add to Prospects" widget present on every
// message-compose surface (full messaging pane + overlay chat bubbles), across
// SPA navigations. Fails quiet — a missing anchor or a thrown selector never
// breaks the LinkedIn page.

import "./widget.css";
import {
  findComposeAnchor,
  findComposeRoots,
  findSendButton,
  inboxFilterRow,
  inboxTopBar,
  threadIsOpen,
} from "./linkedin";
import { buildWidget } from "./widget";
import { buildDraftControl, isFillTab, runFillMode } from "./draft";
import { runCommentPostMode, runScrapeMode } from "./comment";
import { bootstrapSelectors, MOUNT_KEYS, requestHeal } from "./heal";
import { selStr } from "./selectors";
import { POST_COMMENT_HASH, SCRAPE_HASH } from "../lib/storageKeys";

// If this tab was opened by the review-tab queue, it carries the `#cpfill` marker
// so it can pre-fill the cached draft. Captured once at load (before we clear the
// hash below) so the injection guard can see it even after it's stripped.
const fillTab = isFillTab();
// The commenter's two SW-opened tab kinds, likewise detected from the URL hash at
// load: a POST tab (auto-submit an approved comment) and a SCRAPE worker tab
// (report the feed's / a profile's posts). Captured before the hash is stripped.
const hash = location.hash.replace(/^#/, "");
// A post tab carries `cppost=<encoded canonical permalink>` — the key the approved
// comment is cached under. (Bare `cppost` is tolerated as a fallback.)
const postTab = hash === POST_COMMENT_HASH || hash.startsWith(`${POST_COMMENT_HASH}=`);
let postKey: string | null = null;
if (hash.startsWith(`${POST_COMMENT_HASH}=`)) {
  try {
    postKey = decodeURIComponent(hash.slice(POST_COMMENT_HASH.length + 1));
  } catch {
    // A malformed %-escape (only reachable via a hand-crafted URL — the SW always
    // encodes) must not throw during content-script init and take down the page's
    // widgets/capture. Fall back to the location-derived key in runCommentPostMode.
    postKey = null;
  }
}
// A scrape worker tab carries `cpscrape=<target>` — how many posts to collect
// (bare `cpscrape` tolerated as a fallback → a default target).
const scrapeTab = hash === SCRAPE_HASH || hash.startsWith(`${SCRAPE_HASH}=`);
let scrapeTarget = 20;
if (hash.startsWith(`${SCRAPE_HASH}=`)) {
  const n = parseInt(hash.slice(SCRAPE_HASH.length + 1), 10);
  if (Number.isFinite(n) && n > 0) scrapeTarget = n;
}

function inject(): void {
  injectComposeWidgets();
  injectDraftControl();
  checkComposeMount();
}

// A compose surface is "healthy" only if we can find a compose root AND resolve a
// send button inside it — so a rotated send-button selector (root found, button
// not) counts as broken too, not just a missing root. `findSendButton` already
// falls back to text/aria, so a false negative here means the selectors really did
// drift.
function composeSurfacesHealthy(): boolean {
  const roots = findComposeRoots();
  return roots.length > 0 && roots.some((r) => findSendButton(r) !== null);
}

// Mount preflight: if a conversation is OPEN and has a composer editor but we can't
// find a healthy compose surface, the compose/send selectors have likely rotated.
// Confirm it's not just a mid-load flicker by re-checking after a delay, then heal
// and re-inject once it lands. Gated on an actual editor being present so read-only
// threads (InMail teasers, blocked, "conversation unavailable") — which legitimately
// have no composer — don't trigger a spurious heal. `requestHeal` dedups and caps.
let mountProbe: number | undefined;
function composerExpected(): boolean {
  return threadIsOpen() && document.querySelector(selStr("composeEditable")) !== null;
}
function checkComposeMount(): void {
  window.clearTimeout(mountProbe);
  if (!composerExpected() || composeSurfacesHealthy()) return;
  mountProbe = window.setTimeout(() => {
    if (composerExpected() && !composeSurfacesHealthy()) {
      void requestHeal(MOUNT_KEYS).then((healed) => {
        if (healed) schedule();
      });
    }
  }, 3000);
}

function injectComposeWidgets(): void {
  for (const root of findComposeRoots()) {
    // One widget per compose surface. Guarding on the root (not the button's
    // parent) also prevents duplicates when LinkedIn re-lays-out the footer.
    if (root.querySelector(".cp-widget")) continue;
    // Require a real send button so we only attach to actual compose surfaces
    // (and to scope identity).
    const sendBtn = findSendButton(root);
    if (!sendBtn) continue;
    const widget = buildWidget(sendBtn);
    // Mount our row directly below the message text area and above LinkedIn's
    // send/toolbar row. Fall back to appending at the end of the form if no
    // anchor is found, so the button is never lost when selectors drift.
    const anchor = findComposeAnchor(root);
    if (anchor) {
      anchor.ref.insertAdjacentElement(anchor.where, widget);
    } else {
      root.append(widget);
    }
  }
}

// The batch "Draft for" control, mounted once as its own full-width row in the
// messaging header stack — after the filter row (Inbox / Jobs / …) when present,
// otherwise just under the top bar — so it sits above the thread list and never
// inside one of the header's buttons. Guarded document-wide (one control per
// page) so the MutationObserver's re-runs don't stack copies.
function injectDraftControl(): void {
  // A review tab (opened to pre-fill a draft) shouldn't sprout its own control.
  if (fillTab) return;
  if (document.querySelector(".cp-draft-control")) return;
  const anchor = inboxFilterRow() ?? inboxTopBar();
  if (!anchor) return;
  anchor.insertAdjacentElement("afterend", buildDraftControl());
}

// LinkedIn mutates the DOM constantly; re-check on every batch, debounced to one
// pass per frame. The per-root guard above makes re-runs cheap and safe.
let scheduled = false;
function schedule(): void {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    try {
      inject();
    } catch {
      // Never propagate into the host page.
    }
  });
}

new MutationObserver(schedule).observe(document.documentElement, {
  childList: true,
  subtree: true,
});

// SPA route changes: LinkedIn navigates client-side, so re-injection can't rely
// on a page load. The MutationObserver catches most re-renders, but a URL watch
// is the reliable backstop for "navigate away and back to messages". We poll
// location.href (readable from the isolated world, unlike the page's main-world
// history.pushState) and also listen for back/forward.
let lastUrl = location.href;
function onMaybeNavigated(): void {
  if (location.href !== lastUrl) {
    lastUrl = location.href;
    schedule();
    // A thread open/switch keeps the compose root (and its widget) mounted, so
    // its one-time backfill won't re-run. Nudge every mounted widget to re-scan
    // the now-visible thread for incoming replies. Reuses this existing SPA-nav
    // watcher — no new observer. Widgets not yet mounted backfill on mount.
    for (const root of findComposeRoots()) root.dispatchEvent(new Event("cp:rescan"));
  }
}
window.addEventListener("popstate", onMaybeNavigated);
window.setInterval(onMaybeNavigated, 1000);

// Tell the service worker we're alive as soon as a LinkedIn tab is present, so
// the app's gate verifies readiness within seconds instead of waiting for the
// SW's periodic alarm. Fail-quiet (the SW context invalidates on extension
// reload, which rejects the send) and throttled so tab re-focus storms and the
// SPA churn above can't flood /health — the alarm covers steady-state liveness.
let lastCheckin = 0;
function checkin(): void {
  const now = Date.now();
  if (now - lastCheckin < 30_000) return;
  lastCheckin = now;
  try {
    void chrome.runtime.sendMessage({ type: "checkin" }).catch(() => {});
  } catch {
    // Extension context invalidated (reload/update) — the alarm backstops it.
  }
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") checkin();
});
checkin();

// Nudge the service worker to check the app for pending comment work — a requested
// scrape or queued posts. The app can't push to the extension, so this poll (from
// any real, focused LinkedIn tab) is what makes "Scrape" / "Post all" start
// promptly; the SW's periodic alarm backstops it when no tab is focused. Skipped on
// the SW's own worker/post/fill tabs (background + transient) and when hidden, so
// it never busies /health while LinkedIn is closed or backgrounded.
function pollCommentWork(): void {
  if (fillTab || scrapeTab || postTab) return;
  if (document.visibilityState !== "visible") return;
  try {
    void chrome.runtime.sendMessage({ type: "pollCommentWork" }).catch(() => {});
  } catch {
    // Extension context invalidated (reload/update) — the alarm backstops it.
  }
}
window.setInterval(pollCommentWork, 6_000);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") pollCommentWork();
});
window.addEventListener("focus", pollCommentWork);
pollCommentWork();

// Load any persisted selector overrides (from prior self-heals) before the first
// inject, then re-run once they land. The immediate schedule() below still mounts
// on the compiled defaults so there's no wait on the happy path.
void bootstrapSelectors().then(schedule);
schedule();

// Fill mode: this tab was opened by the review-tab queue (tagged via the `#cpfill`
// hash) straight on a thread URL. Clear the hash first — so an SPA re-render or a
// manual refresh can't re-trigger the fill — then paste the cached draft for this
// conversation into its composer once the tab is foregrounded.
if (fillTab) {
  history.replaceState(null, "", location.pathname + location.search);
  void runFillMode().catch(() => {
    // runFillMode already surfaces failures via a toast; never propagate.
  });
}

// Post tab: opened by the SW straight on a post's permalink (tagged `#cppost`) to
// AUTO-SUBMIT an approved comment. Clear the hash first so an SPA re-render/refresh
// can't re-trigger the post, then write + submit the cached comment and report the
// outcome back to the SW.
if (postTab) {
  history.replaceState(null, "", location.pathname + location.search);
  void runCommentPostMode(postKey).catch(() => {
    // runCommentPostMode reports its own outcome to the SW; never propagate.
  });
}

// Scrape worker tab: opened by the SW on a watched profile's recent-activity
// (tagged `#cpscrape`) to scrape its posts and report them back. The SW closes the
// tab once it reports; leave the hash in place (nothing keys off stripping it).
if (scrapeTab) {
  void runScrapeMode(scrapeTarget).catch(() => {
    // Best-effort; runScrapeMode reports [] on failure so the SW waiter resolves.
  });
}
