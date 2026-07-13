// The "Draft for [N]" batch feature. Two roles live here:
//
//  1. The INBOX tab (where the control is mounted) runs `runCycle`: it opens each
//     of the next N conversations in its own reading pane, scrapes the thread,
//     captures the thread URL, and fires a draft request per conversation. As each
//     draft comes back it's cached (by URL) and the service worker is asked to open
//     that conversation as a pre-filled review tab. Generation is the bottleneck,
//     so it runs concurrently (server-capped) while the cycle keeps scanning.
//
//  2. Each REVIEW tab the SW opens runs `runFillMode`: it resolves the thread it
//     was navigated to, reads the cached draft for that URL, and — once the tab is
//     brought to the foreground (execCommand only takes in the active document) —
//     pastes it into the composer. No generation happens here; the draft is already
//     made, so the paste is instant.
//
// All DOM heuristics live in ./linkedin.

import {
  activateRow,
  composerHasText,
  conversationScope,
  currentThreadUrl,
  findComposeRoots,
  findSendButton,
  identify,
  isRowActive,
  scrapeMessages,
  selectedRowIndex,
  topThreadRows,
  writeComposer,
} from "./linkedin";
import { friendlyError, send } from "./bridge";
import { el } from "./dom";
import { showToast } from "./toast";
import { hideDraftOverlay, showDraftOverlay } from "./overlay";
import { delay } from "../lib/delay";
import { FILL_HASH, LAST_PITCH_KEY } from "../lib/storageKeys";
import { clearDrafts, peekDraft, putDraft, removeDraft } from "../lib/draftStore";
import type { DraftMessageInput, DraftResult, Pitch } from "../lib/types";

/** Whether this tab was opened by the review-tab queue to be pre-filled with a
 *  cached draft — detected by the `#cpfill` hash marker the queue appends. Read
 *  once at load, before the hash is stripped. */
export function isFillTab(): boolean {
  return location.hash.replace(/^#/, "") === FILL_HASH;
}

/** The last thread count the user drafted for. */
const LAST_COUNT_KEY = "lastDraftCount";

/** The thread-count choices; 20 is the default. `1` drafts for just the
 *  selected thread (a quick single-reply pass). */
const COUNTS = [1, 5, 10, 20, 50];
const DEFAULT_COUNT = 20;

/**
 * Build the "Draft for" control: a pitch dropdown stacked above a [Draft for]
 * button + a thread-count dropdown, plus a small feedback line. The pitch list is
 * fetched through the SW; the button is disabled until pitches are loaded.
 */
export function buildDraftControl(): HTMLElement {
  const root = el("div", "cp-draft-control");
  const feedback = el("div", "cp-draft-feedback");
  feedback.setAttribute("role", "status");

  const pitch = el("select", "cp-draft-pitch");
  pitch.title = "Pitch whose snippets the drafts are built from";
  pitch.disabled = true;

  const row = el("div", "cp-draft-row");
  const button = el("button", "cp-draft-btn");
  button.type = "button";
  button.textContent = "Draft for";
  button.disabled = true;

  const count = el("select", "cp-draft-count");
  count.title = "How many conversations to draft for, counting down from the selected one";
  for (const n of COUNTS) {
    const opt = el("option");
    opt.value = String(n);
    opt.textContent = String(n);
    count.append(opt);
  }
  count.value = String(DEFAULT_COUNT);

  row.append(button, count);
  root.append(pitch, row, feedback);

  let feedbackTimer: number | undefined;
  function showFeedback(kind: "info" | "success" | "error", text: string, sticky = false): void {
    feedback.textContent = text;
    feedback.dataset.kind = kind;
    feedback.dataset.show = "true";
    window.clearTimeout(feedbackTimer);
    // A running cycle keeps its progress line up (sticky); one-off notices fade.
    if (!sticky) feedbackTimer = window.setTimeout(() => delete feedback.dataset.show, 4000);
  }

  // Restore the last-used count (best-effort; a torn-down context just keeps the
  // default).
  void chrome.storage.local.get(LAST_COUNT_KEY).then(
    (stored) => {
      const last = stored[LAST_COUNT_KEY] as number | undefined;
      if (last != null && COUNTS.includes(last)) count.value = String(last);
    },
    () => {},
  );
  count.addEventListener("change", () => {
    void chrome.storage.local.set({ [LAST_COUNT_KEY]: Number(count.value) }).catch(() => {});
  });

  // Load pitches into the dropdown.
  void (async () => {
    const res = await send<Pitch[]>({ type: "listPitches" });
    if (!res.ok) {
      const opt = el("option");
      opt.textContent = "—";
      pitch.append(opt);
      showFeedback("error", friendlyError(res.error));
      return;
    }
    if (res.data.length === 0) {
      const opt = el("option");
      opt.textContent = "No pitches yet";
      pitch.append(opt);
      pitch.title = "Create a pitch in Courland first.";
      return;
    }
    for (const p of res.data) {
      const opt = el("option");
      opt.value = String(p.id);
      opt.textContent = p.name;
      pitch.append(opt);
    }
    pitch.disabled = false;
    button.disabled = false;

    const stored = await chrome.storage.local
      .get(LAST_PITCH_KEY)
      .catch(() => ({}) as Record<string, unknown>);
    const last = stored[LAST_PITCH_KEY] as number | undefined;
    if (last != null && res.data.some((p) => p.id === last)) pitch.value = String(last);
  })();

  button.addEventListener("click", async () => {
    const pitchId = pitch.value ? Number(pitch.value) : NaN;
    if (!Number.isFinite(pitchId)) {
      showFeedback("error", "Pick a pitch first.");
      return;
    }
    const n = Number(count.value) || DEFAULT_COUNT;

    button.disabled = true;
    button.dataset.busy = "true";
    try {
      void chrome.storage.local.set({ [LAST_PITCH_KEY]: pitchId }).catch(() => {});
      // Cycle from whatever conversation is selected right now: draft for it and
      // the N-1 below it. The cycle runs in THIS tab (the inbox), opening each
      // conversation in the reading pane to scrape it — the pane visibly steps
      // through them — and opens a pre-filled review tab as each draft is ready.
      await runCycle(pitchId, n, selectedRowIndex(), (p) => {
        if (p.done) {
          const bits = [`${p.ready} draft${p.ready === 1 ? "" : "s"} ready`];
          if (p.failed > 0) bits.push(`${p.failed} skipped`);
          showFeedback("success", `Done — ${bits.join(", ")}. Tabs open as they're ready.`);
        } else {
          showFeedback("info", `Scanning conversations… ${p.ready} ready`, true);
        }
      });
    } catch {
      showFeedback("error", "Drafting run failed. Try again.");
    } finally {
      delete button.dataset.busy;
      button.disabled = false;
    }
  });

  return root;
}

// ── Inbox tab: the cycle ─────────────────────────────────────────────────────

/** Cap on opening a single conversation in the reading pane before we treat the
 *  index as past the end of the list. Runs in the foreground inbox tab (not
 *  throttled), so this is a generous ceiling, not the common case. */
const OPEN_PHASE_MS = 20_000;
/** Give up the cycle after this many conversations in a row render but never
 *  confirm as open — that points at a structural failure (e.g. LinkedIn renamed
 *  the active-row class) rather than isolated lag, and bounds the worst-case spin
 *  to N × OPEN_PHASE_MS instead of letting every remaining index burn its budget. */
const MAX_CONSECUTIVE_UNCONFIRMED = 3;
/** How often to poll while confirming the target conversation opened. */
const NAV_POLL_MS = 150;
/** Let the opened thread settle (its DOM swap in) before scraping it. */
const SETTLE_MS = 300;
/** Re-fire the row activation at most this often. LinkedIn attaches the list's
 *  click handlers a beat after the rows first render, so a single early activation
 *  can no-op — keep nudging until the target is the open conversation. */
const REACTIVATE_MS = 800;

/** Progress reported back to the control's feedback line as the cycle runs. */
interface CycleProgress {
  /** Drafts successfully generated + cached so far. */
  ready: number;
  /** Conversations that couldn't be read or drafted. */
  failed: number;
  /** Whether the whole cycle (including outstanding generations) has finished. */
  done: boolean;
}

/**
 * The inbox-tab drafting pass. Steps through conversations `start … start+count-1`
 * in the reading pane, scrapes each, and fires a draft request per conversation
 * WITHOUT awaiting it — so the cycle scans at DOM speed while generations run
 * concurrently (server-capped). Each resolved draft is cached by thread URL and a
 * pre-filled review tab is opened for it. Resolves once every generation settles.
 */
export async function runCycle(
  pitchId: number,
  count: number,
  start: number,
  report: (p: CycleProgress) => void,
): Promise<void> {
  // Fresh batch: drop any leftover cached drafts and reset the review-tab queue so
  // this run never surfaces a previous run's drafts or inherits a spent slot count.
  await clearDrafts();
  await send({ type: "resetReviewQueue" });

  let ready = 0;
  let failed = 0;
  // Conversations that rendered but never confirmed as open, back to back. A few in
  // a row means something structural changed rather than isolated lag — bail then.
  let consecutiveUnconfirmed = 0;
  const pending: Promise<void>[] = [];
  const tick = (done: boolean): void => report({ ready, failed, done });

  for (let i = 0; i < count; i++) {
    const index = start + i;
    const outcome = await openConversationAt(index, Date.now() + OPEN_PHASE_MS);
    // The row never rendered even after scrolling — the inbox has fewer
    // conversations than requested, so we're genuinely at the end. Stop.
    if (outcome === "absent") break;
    // Rendered but wouldn't confirm as the open conversation. Skip it rather than
    // aborting the whole batch on a transient glitch — but give up once several in
    // a row fail, which is structural (e.g. an active-row class rename), not lag.
    if (outcome === "unconfirmed") {
      failed += 1;
      if (++consecutiveUnconfirmed >= MAX_CONSECUTIVE_UNCONFIRMED) break;
      tick(false);
      continue;
    }
    consecutiveUnconfirmed = 0;

    const thread = resolvePersonThread();
    const url = currentThreadUrl();
    if (!thread || !url) {
      failed += 1;
      tick(false);
      continue;
    }

    // Snapshot the thread into plain data now, while it's the open conversation —
    // the async generation below must not read the DOM after we've moved on.
    const messages = scrapeMessages(thread.scope, thread.url).map((m) => ({
      direction: m.direction,
      body: m.body,
    }));
    const name = thread.name;
    const p = generateAndQueue(pitchId, url, name, messages).then((ok) => {
      if (ok) ready += 1;
      else failed += 1;
      tick(false);
    });
    pending.push(p);
    tick(false);
  }

  // Let outstanding generations finish so the final tally reflects reality.
  await Promise.allSettled(pending);
  tick(true);
}

/** Generate one conversation's draft, cache it by URL, and ask the SW to open a
 *  pre-filled review tab for it. The cache write completes BEFORE the tab is asked
 *  to open, so the draft is present the moment that tab loads. Returns whether a
 *  draft was produced and queued. */
async function generateAndQueue(
  pitchId: number,
  url: string,
  name: string,
  messages: DraftMessageInput[],
): Promise<boolean> {
  const res = await send<DraftResult>({
    type: "draftReply",
    payload: { prospect_name: name, pitch_id: pitchId, messages },
  });
  if (!res.ok) return false;
  await putDraft(url, res.data.draft);
  await send({ type: "openReviewTab", payload: { url } });
  return true;
}

/**
 * Open the conversation at list position `index` in this tab: wait for the list to
 * render, scroll to load more rows when the list is virtualized (scroll-on-miss),
 * keyboard-activate the target row, then CONFIRM LinkedIn actually opened THAT
 * conversation — its row is marked active AND a thread is open — before returning.
 * Without the confirmation, a slow SPA navigation or LinkedIn auto-opening its
 * default (top) thread could leave us scraping the wrong conversation. Reports
 * `"opened"` on success, `"absent"` if the row never rendered within `deadline`
 * (end of the list), or `"unconfirmed"` if it rendered but never confirmed as open.
 */
type OpenOutcome = "opened" | "absent" | "unconfirmed";

async function openConversationAt(index: number, deadline: number): Promise<OpenOutcome> {
  let lastActivate = 0;
  // Whether the target row ever rendered — lets us tell "end of the list" (never
  // rendered) apart from "rendered but wouldn't confirm as open" at the deadline.
  let everRendered = false;
  while (Date.now() < deadline) {
    const rows = topThreadRows(index + 1);
    if (rows.length > index) {
      everRendered = true;
      const row = rows[index];
      // Proceed only once THIS row is the open conversation (exactly one is active
      // at a time) and a thread URL is present.
      if (isRowActive(row) && currentThreadUrl()) {
        await delay(SETTLE_MS);
        return "opened";
      }
      // (Re)activate periodically until it takes — the list's click handlers
      // hydrate slightly after the rows render, so the first nudge can no-op.
      if (Date.now() - lastActivate > REACTIVATE_MS) {
        activateRow(row);
        lastActivate = Date.now();
      }
    } else {
      // Fewer rows rendered than we need — scroll the last one into view so
      // LinkedIn loads more, then retry. If the list isn't up yet, just wait.
      rows[rows.length - 1]?.scrollIntoView({ block: "end" });
    }
    await delay(NAV_POLL_MS);
  }
  // Deadline hit: absent if the row never rendered (end of the list), else it
  // rendered but wouldn't confirm as open (transient lag or a changed marker).
  return everRendered ? "unconfirmed" : "absent";
}

// ── Review tab: the fill ─────────────────────────────────────────────────────

/** How long a freshly-opened review tab keeps trying to resolve its thread before
 *  giving up. Generous because it may open in the background, where Chrome
 *  throttles timers and deprioritizes rendering. */
const FILL_DEADLINE_MS = 40_000;
const FILL_POLL_MS = 500;

/**
 * Review-tab entry, run in a tab the SW opened straight on a thread URL (tagged
 * via the URL hash). Resolves the conversation, reads its cached draft, then — once
 * the tab is brought to the foreground (`execCommand` only takes in the active
 * document) — pastes the draft into the composer. No generation: the draft was
 * made during the inbox cycle, so the paste is instant the moment the tab is
 * viewed. Never sends.
 */
export async function runFillMode(): Promise<void> {
  const deadline = Date.now() + FILL_DEADLINE_MS;
  // Signal the review-tab queue exactly once, as soon as this tab has loaded far
  // enough to know its draft — so the next queued tab opens without waiting for the
  // user to focus this one.
  let released = false;
  const freeSlot = (): void => {
    if (released) return;
    released = true;
    void send({ type: "reviewTabFilled" });
  };
  // Mask LinkedIn's load/render churn while the tab comes up; lifts before the
  // paste. Only ever seen if the user clicks into the tab before it's ready.
  showDraftOverlay();
  try {
    // Wait for the thread this tab opened on to become readable.
    let url: string | null = null;
    while (Date.now() < deadline) {
      if (resolvePersonThread() && currentThreadUrl()) {
        url = currentThreadUrl();
        break;
      }
      await delay(FILL_POLL_MS);
    }
    if (!url) {
      showToast("error", "Couldn't open the conversation to fill the draft.");
      return;
    }

    // Peek only to decide whether this tab has work and to free the slot; the draft
    // itself is re-read AFTER the foreground wait, so a batch-start clear or a
    // regeneration during the (possibly long) idle wait is honored.
    const pending = await peekDraft(url);
    // Loaded far enough to know our draft — free the queue slot so the next tab
    // opens, rather than holding it through the idle wait for foreground.
    freeSlot();
    if (pending == null) return; // Nothing cached (expired / already consumed).

    // The draft is ready; writing only takes in the ACTIVE document, so lift the
    // overlay and hold until the tab is viewed, then drop it straight in.
    hideDraftOverlay();
    await whenTabForeground();
    const thread = resolvePersonThread();
    if (!thread) {
      showToast("error", "Couldn't find the conversation to write the draft into.");
      return;
    }
    // Re-read after the idle wait: a new batch may have cleared or superseded this
    // entry (the cache is shared and mutable, but our snapshot above is not). A null
    // now means it's stale or already consumed — never paste a superseded draft.
    const draft = await peekDraft(url);
    if (draft == null) return;
    // Re-guard on the fresh root: the user may have started typing here while the
    // tab sat in the background. Never clobber a hand-written reply.
    if (composerHasText(thread.root)) {
      showToast("info", "You already have a draft here — left it untouched.");
      await removeDraft(url);
      return;
    }
    if (writeComposer(thread.root, draft)) {
      showToast("success", "Draft ready — review before sending.");
    } else {
      showToast("error", "Couldn't write the draft into the message box.");
    }
    await removeDraft(url);
  } finally {
    hideDraftOverlay();
    freeSlot();
  }
}

/** The open MAIN-pane conversation resolved to a single person we can draft for:
 *  the first compose root that isn't a persisted overlay chat bubble and whose
 *  thread identifies as one person. `null` when the thread isn't a readable 1:1
 *  person thread yet, so callers retry (LinkedIn renders asynchronously). */
interface PersonThread {
  root: HTMLElement;
  scope: Element;
  name: string;
  url: string;
}

function resolvePersonThread(): PersonThread | null {
  for (const root of findComposeRoots()) {
    // Never touch a persisted overlay chat bubble — it's a different conversation
    // than the one open in the main pane.
    if (root.closest('[class*="msg-overlay-conversation"]')) continue;
    const scope = conversationScope(findSendButton(root) ?? root);
    const who = identify(scope);
    if (who.kind !== "person") continue;
    return { root, scope, name: who.identity.name, url: who.identity.url };
  }
  return null;
}

/** Resolve once this tab is in the foreground — immediately if it already is,
 *  otherwise on the next visibility/focus gain. The review tab holds its finished
 *  draft until the user switches to it, then pastes (execCommand acts on the active
 *  document). */
function whenTabForeground(): Promise<void> {
  const ready = (): boolean => document.visibilityState === "visible" && document.hasFocus();
  if (ready()) return Promise.resolve();
  return new Promise((resolve) => {
    const check = (): void => {
      if (!ready()) return;
      document.removeEventListener("visibilitychange", check);
      window.removeEventListener("focus", check);
      resolve();
    };
    document.addEventListener("visibilitychange", check);
    window.addEventListener("focus", check);
  });
}
