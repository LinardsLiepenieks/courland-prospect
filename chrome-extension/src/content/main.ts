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
} from "./linkedin";
import { buildWidget } from "./widget";
import { buildDraftControl, isFillTab, runFillMode } from "./draft";

// If this tab was opened by the review-tab queue, it carries the `#cpfill` marker
// so it can pre-fill the cached draft. Captured once at load (before we clear the
// hash below) so the injection guard can see it even after it's stripped.
const fillTab = isFillTab();

function inject(): void {
  injectComposeWidgets();
  injectDraftControl();
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
