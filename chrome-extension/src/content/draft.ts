// The "Draft for [N]" batch feature, injected as a row in LinkedIn's messaging
// header. The control picks a pitch + a thread count and asks the service worker
// to open N background tabs on the messaging inbox, each tagged with the pitch
// and a conversation index. Each tab then runs `runDraftMode`: open its assigned
// conversation (by list position — the rows carry no URL to hand off), scrape the
// thread, ask the app to compose a reply from the pitch's snippets, and write it
// into the compose box (never sending). All DOM heuristics live in ./linkedin.

import {
  activateRow,
  activeInboxFilter,
  clickFilterPill,
  composerHasText,
  conversationScope,
  currentThreadUrl,
  findComposeRoots,
  findSendButton,
  identify,
  inboxFilterPill,
  isInboxFilterActive,
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
import { LAST_PITCH_KEY } from "../lib/storageKeys";
import type { DraftResult, Pitch } from "../lib/types";

/** The last thread count the user drafted for. */
const LAST_COUNT_KEY = "lastDraftCount";

/** The thread-count choices; 20 is the default. `1` drafts for just the
 *  selected thread (a quick single-reply pass). */
const COUNTS = [1, 5, 10, 20, 50];
const DEFAULT_COUNT = 20;

/** How long a freshly-opened thread tab keeps retrying to open its conversation
 *  and draft before giving up. Generous because these are background tabs: Chrome
 *  throttles their timers and deprioritizes rendering, so the inbox + thread can
 *  take a while to appear. */
const DRAFT_DEADLINE_MS = 40_000;
const DRAFT_POLL_MS = 800;
/** Cap on the "open the conversation" phase, so confirming which thread opened
 *  can't consume the whole budget and leave no time to actually draft. */
const OPEN_PHASE_MS = 20_000;

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
  function showFeedback(kind: "info" | "success" | "error", text: string): void {
    feedback.textContent = text;
    feedback.dataset.kind = kind;
    feedback.dataset.show = "true";
    window.clearTimeout(feedbackTimer);
    feedbackTimer = window.setTimeout(() => delete feedback.dataset.show, 4000);
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
      // Count down from whatever conversation is selected right now: the batch
      // drafts for it and the N-1 below it. Each tab opens its own conversation
      // (by list position) and drafts — the rows have no URL to hand off, so we
      // pass a start offset + count + pitch and let each tab find its conversation
      // itself. `filter` carries the active inbox filter (e.g. Unread) so each
      // tab's list matches the view these indices came from. The pane never navigates.
      const res = await send<{ opened: number }>({
        type: "openThreads",
        payload: { pitchId, count: n, start: selectedRowIndex(), filter: activeInboxFilter() },
      });
      if (!res.ok) {
        showFeedback("error", friendlyError(res.error));
        return;
      }
      showFeedback(
        "info",
        `Opening ${res.data.opened} draft tab${res.data.opened === 1 ? "" : "s"} — replies fill in as they're ready.`,
      );
    } finally {
      delete button.dataset.busy;
      button.disabled = false;
    }
  });

  return root;
}

/** Cap on the "re-apply the inbox filter" phase — clicking the filter pill and
 *  waiting for the filtered list to actually load and settle before we resolve the
 *  index against it. Generous because a background tab fetches the filtered set
 *  over the network under timer throttling. */
const FILTER_PHASE_MS = 10_000;
/** How often to poll while waiting for the filter to take. */
const FILTER_POLL_MS = 200;
/** Re-click the filter pill at most this often. The pill is a toggle and we only
 *  ever click it while it's still unselected, but a click can take a moment to
 *  register; this spacing must stay comfortably above Chrome's ~1s background-tab
 *  timer clamp so a laggy `aria-pressed` can't trick us into clicking twice. */
const FILTER_REACTIVATE_MS = 2_500;
/** Consecutive unchanged polls required before the filtered list counts as settled
 *  (so the index isn't resolved against a mid-re-render list). */
const FILTER_SETTLE_POLLS = 2;

/** How often to poll while confirming the target conversation opened. */
const NAV_POLL_MS = 150;
/** Let the opened thread settle (its DOM swap in) before drafting into it. */
const SETTLE_MS = 300;
/** Re-fire the row activation at most this often. LinkedIn attaches the list's
 *  click handlers a beat after the rows first render, so a single early
 *  activation can no-op — keep nudging until the target is the open conversation. */
const REACTIVATE_MS = 800;

/**
 * Re-apply the inbox filter pill `token` (e.g. "UNREAD") in this tab so its
 * conversation list matches the filtered view the batch indices came from. Clicks
 * the pill and waits until it reports selected (`aria-pressed`). The pill is a
 * toggle, so we click ONLY while it's still unselected — never twice, which would
 * turn it back off. Best-effort: returns true once confirmed active, false if the
 * pill never appeared or wouldn't take within `deadline` (the caller then proceeds
 * on the default, unfiltered view). Runs fine in a background tab.
 */
async function applyInboxFilter(token: string, deadline: number): Promise<boolean> {
  let lastClick = 0;
  let didClick = false;
  // The list as it looked before we selected the filter — so we can tell when the
  // list has actually switched to the filtered set (not just that the pill lit up,
  // which happens before the filtered rows finish loading).
  let beforeSig: string | null = null;
  let settledSig: string | null = null;
  let settledPolls = 0;
  while (Date.now() < deadline) {
    const pill = inboxFilterPill(token);
    if (pill) {
      if (isInboxFilterActive(pill)) {
        // Pill is selected — NEVER click again (it's a toggle; a second click
        // would turn the filter back off). Instead wait for the conversation list
        // to reflect the filter: it must be non-empty and, if we were the ones who
        // clicked, differ from the pre-filter list, then hold steady for a couple
        // of polls before we let the index be resolved against it.
        const sig = listSignature();
        const reflectsFilter = sig !== "" && (!didClick || (beforeSig !== null && sig !== beforeSig));
        if (reflectsFilter) {
          if (sig === settledSig) {
            if (++settledPolls >= FILTER_SETTLE_POLLS) return true;
          } else {
            settledSig = sig;
            settledPolls = 1;
          }
        }
      } else {
        // Not selected yet. Capture the pre-filter list once, then click to select
        // — only ever while inactive, so a landed selection is never toggled off.
        if (beforeSig === null) {
          const s = listSignature();
          if (s !== "") beforeSig = s;
        }
        if (Date.now() - lastClick > FILTER_REACTIVATE_MS) {
          clickFilterPill(pill);
          lastClick = Date.now();
          didClick = true;
        }
      }
    }
    await delay(FILTER_POLL_MS);
  }
  return false;
}

/** A cheap fingerprint of the top of the conversation list — its first rows'
 *  text — used to tell when applying a filter has swapped the list to the filtered
 *  set and when that list has stopped changing. Empty string when no rows render. */
function listSignature(): string {
  return topThreadRows(3)
    .map((r) => (r.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 60))
    .join("||");
}

/**
 * Open the conversation at list position `index` in this tab: wait for the list
 * to render, scroll to load more rows when the list is virtualized (scroll-on-
 * miss), keyboard-activate the target row, then CONFIRM LinkedIn actually opened
 * THAT conversation — its row is marked active AND a thread is open — before
 * returning. Without the confirmation, a slow SPA navigation or LinkedIn
 * auto-opening its default (top) thread could leave the drafter writing into the
 * wrong conversation. Returns false if the target row never rendered (the inbox
 * has fewer than `index + 1` conversations) within `deadline`. Runs inside a
 * background draft tab opened by the batch.
 */
async function openConversationAt(index: number, deadline: number): Promise<boolean> {
  let lastActivate = 0;
  while (Date.now() < deadline) {
    const rows = topThreadRows(index + 1);
    if (rows.length > index) {
      const row = rows[index];
      // Draft only once THIS row is the open conversation (exactly one is active
      // at a time) and a thread URL is present.
      if (isRowActive(row) && currentThreadUrl()) {
        await delay(SETTLE_MS);
        return true;
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
  // Couldn't confirm the target row became the open conversation in time. Fail
  // safe: report failure so the drafter does NOT write into whatever thread
  // happens to be open (LinkedIn auto-opens the top thread on load) — that would
  // put this tab's reply in the wrong prospect's box. A missed draft is far
  // better than a misdirected one.
  return false;
}

/**
 * Draft-mode entry, run in a tab the batch opened on the messaging inbox
 * (detected via the URL hash in main.ts). Opens this tab's assigned conversation
 * by list position, GENERATES the reply in the background (scraping the thread +
 * the AI round-trip both work in an unfocused tab), then holds the finished draft
 * and writes it into the composer the moment the tab is brought to the
 * foreground. Splitting it this way lets the slow part (generation) run while the
 * tab sits in the background, so the reply drops in the instant the user switches
 * to the tab — the write itself must wait for focus, since `execCommand` only
 * takes in the active document.
 */
export async function runDraftMode(
  pitchId: number,
  index: number,
  filter: string | null,
): Promise<void> {
  const deadline = Date.now() + DRAFT_DEADLINE_MS;
  // The batch opens tabs a few at a time and waits for each to finish its heavy
  // phase before opening the next. Signal that completion exactly once, as soon as
  // the load+generate work is done (before the idle wait for foreground), so the
  // service worker can open the next queued tab.
  let slotFreed = false;
  const freeDraftSlot = (): void => {
    if (slotFreed) return;
    slotFreed = true;
    void send({ type: "draftSlotFree" });
  };
  // Cover the tab while it applies the filter, opens its conversation, and
  // generates. All run in the background, so this overlay is only ever seen if the
  // user clicks into the tab mid-work; it lifts the moment generation finishes.
  showDraftOverlay();
  try {
    // Re-apply the same inbox filter the user was viewing (best-effort) BEFORE
    // resolving the index, so `index` counts into the same filtered list the
    // foreground did. On failure we fall through to the default view.
    if (filter) await applyInboxFilter(filter, Math.min(Date.now() + FILTER_PHASE_MS, deadline));
    // Cap the open phase so it can't eat the whole budget — generating below needs
    // time left even if opening the conversation was slow.
    const openDeadline = Math.min(Date.now() + OPEN_PHASE_MS, deadline);
    if (!(await openConversationAt(index, openDeadline))) {
      showToast("error", "Couldn't open the conversation to draft for.");
      return;
    }
    const result = await generateReply(pitchId, deadline);
    // Heavy load+generate phase is over (whatever the outcome) — release the batch
    // slot now so the next tab starts, rather than holding it through the idle wait.
    freeDraftSlot();
    if (result.status === "unreadable") {
      showToast("error", "Couldn't read this thread to draft a reply.");
      return;
    }
    if (result.status !== "ready") return; // "skip" / "error" already surfaced a toast

    // Generation is done. Writing only takes in the ACTIVE document, so lift the
    // overlay and hold the draft until the tab is viewed, then drop it straight
    // in — no waiting on the AI once the user is actually looking at the tab.
    hideDraftOverlay();
    await whenTabForeground();
    const thread = resolvePersonThread();
    if (!thread) {
      showToast("error", "Couldn't find the conversation to write the draft into.");
      return;
    }
    // Re-guard on the fresh root: the user may have started typing here while the
    // tab sat in the background. Never clobber a hand-written reply.
    if (composerHasText(thread.root)) {
      showToast("info", "You already have a draft here — left it untouched.");
      return;
    }
    if (writeComposer(thread.root, result.draft)) {
      showToast("success", "Draft ready — review before sending.");
    } else {
      showToast("error", "Couldn't write the draft into the message box.");
    }
  } finally {
    hideDraftOverlay();
    // Covers the early return when the conversation never opened, and any throw:
    // the heavy phase is over, so the slot must be returned to the batch queue.
    freeDraftSlot();
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
    // Never draft into a persisted overlay chat bubble — it's a different
    // conversation than the one this tab opened in the main pane.
    if (root.closest('[class*="msg-overlay-conversation"]')) continue;
    const scope = conversationScope(findSendButton(root) ?? root);
    const who = identify(scope);
    if (who.kind !== "person") continue;
    return { root, scope, name: who.identity.name, url: who.identity.url };
  }
  return null;
}

/** The result of the background generation phase. Only `ready` carries a draft to
 *  write; `skip`/`error` have already shown the user a toast, and `unreadable`
 *  means the deadline passed before the thread became readable. */
type GenerateResult =
  | { status: "ready"; draft: string }
  | { status: "skip" }
  | { status: "error" }
  | { status: "unreadable" };

/** Generate the reply for this tab's open thread, retrying until it's readable or
 *  the deadline passes. Pure background work — DOM reads plus one message
 *  round-trip, no focus required — so it completes while the tab sits unfocused.
 *  Does NOT write into the composer (that waits for the foreground). */
async function generateReply(pitchId: number, deadline: number): Promise<GenerateResult> {
  while (Date.now() < deadline) {
    const thread = resolvePersonThread();
    if (thread) {
      // A reply the user already typed here (LinkedIn restores a per-conversation
      // unsent draft on open) means there's nothing for us to do.
      if (composerHasText(thread.root)) {
        showToast("info", "You already have a draft here — left it untouched.");
        return { status: "skip" };
      }
      const messages = scrapeMessages(thread.scope, thread.url).map((m) => ({
        direction: m.direction,
        body: m.body,
      }));
      const res = await send<DraftResult>({
        type: "draftReply",
        payload: { prospect_name: thread.name, pitch_id: pitchId, messages },
      });
      if (!res.ok) {
        showToast("error", friendlyError(res.error));
        return { status: "error" };
      }
      return { status: "ready", draft: res.data.draft };
    }
    await delay(DRAFT_POLL_MS);
  }
  return { status: "unreadable" };
}

/** Resolve once this tab is in the foreground — immediately if it already is,
 *  otherwise on the next visibility/focus gain. The draft tab generates in the
 *  background but can only WRITE into the composer once focused (execCommand acts
 *  on the active document), so the finished reply is held until the user switches
 *  to the tab and then dropped straight in. */
function whenTabForeground(): Promise<void> {
  const ready = (): boolean =>
    document.visibilityState === "visible" && document.hasFocus();
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

/** The pitch id, target conversation index, and inbox filter this tab was opened
 *  to draft for, or `null` when this isn't a draft-mode tab. Parsed from
 *  `#cpdraft=<pitchId>&i=<index>&filter=<TOKEN>`. `filter` is `null` when absent,
 *  and only an uppercase-token shape is accepted (it flows into an attribute
 *  selector), so a junk value degrades to the default view rather than misbehaving. */
export function parseDraftHash(): { pitchId: number; index: number; filter: string | null } | null {
  const hash = location.hash.replace(/^#/, "");
  if (!hash.includes("cpdraft")) return null;
  const params = new URLSearchParams(hash);
  const pitchId = Number(params.get("cpdraft"));
  const index = Number(params.get("i"));
  if (!Number.isFinite(pitchId) || pitchId <= 0) return null;
  if (!Number.isInteger(index) || index < 0) return null;
  const rawFilter = params.get("filter");
  const filter = rawFilter && /^[A-Z_]+$/.test(rawFilter) ? rawFilter : null;
  return { pitchId, index, filter };
}
