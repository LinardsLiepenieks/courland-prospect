// Builds the "[Pitch ▼] [Add to Prospects]" cluster injected as its own row
// below the message text area (above LinkedIn's send row — placement lives in
// main.ts), plus the floating feedback element above it. All network work is
// delegated to the service worker; this file is pure DOM + interaction.

import {
  conversationScope,
  findSendButton,
  identify,
  looksLikeSend,
  scrapeMessages,
} from "./linkedin";
import { friendlyError, send } from "./bridge";
import { el } from "./dom";
import { IDENTITY_KEYS, MESSAGE_KEYS, requestHeal } from "./heal";
import { selStr } from "./selectors";
import { showToast, type ToastKind } from "./toast";
import { LAST_PITCH_KEY } from "../lib/storageKeys";
import type {
  AddProspectResult,
  CaptureOutcome,
  Pitch,
  ProspectLookup,
  Response,
} from "../lib/types";

/** After a send, LinkedIn renders the message (and assigns its stable urn)
 *  asynchronously. We scrape on two passes — a quick one, then a backstop — so we
 *  capture with a stable key rather than a content hash that would re-key later. */
const CAPTURE_PASSES_MS = [1200, 3500];

/** Turn a capture attempt's result into a toast — only for explicit user sends
 *  (background backfill is silent). "skipped" means the person isn't a tracked
 *  prospect, which is normal, so it says nothing. */
function reportCapture(res: Response<CaptureOutcome>): void {
  if (!res.ok) {
    showToast("error", friendlyError(res.error));
    return;
  }
  if (res.data.outcome === "stored") {
    showToast("success", "Message tracked ✓");
  } else if (res.data.outcome === "offline") {
    showToast("info", "Courland offline — will sync when it's open.");
  }
}

/**
 * Capture the messages in a conversation — both the ones you send and the
 * replies you receive — and hand them to the service worker (which durably
 * queues + delivers them). Each message carries its direction: outgoing bumps
 * the prospect's sent-count; whichever message is newest decides whether they're
 * "awaiting reply" (their reply is the latest, unanswered).
 *
 * Two triggers, both reusing the same scrape + post path:
 *  - SEND: a click on Send or Enter-to-send captures your just-sent message
 *    (and any replies already visible), scraping once the message renders.
 *  - VIEW: on mount and on `cp:rescan` (fired when you open/switch threads),
 *    a silent backfill scrapes whatever's on screen — this is how a reply gets
 *    noticed when you open a thread to read it.
 *
 * Identity is resolved live from `composeRoot` each time (never a stale captured
 * scope), reading only the open thread — never the sidebar list, which used to
 * poison attribution. If it can't resolve a single person on an explicit send we
 * show an error toast and post nothing (fail safe — never mis-attribute).
 *
 * `sent` dedups within this mount so the passes / re-scrapes don't repost the
 * same message; it's not persistent — the SW + backend dedup by key, which gives
 * free backfill when someone becomes a prospect after messages already exist.
 */
function setupCapture(composeRoot: HTMLElement): { resync: () => void } {
  const sent = new Set<string>();

  // The open conversation to scrape, resolved live from the (stable) compose root.
  function currentScope(): Element {
    return conversationScope(findSendButton(composeRoot) ?? composeRoot);
  }

  // Post not-yet-sent messages in `scope`, attributed to `url`. Before the
  // final pass we require a stable urn key, so a freshly-sent message isn't
  // captured under a content hash now and re-captured under its urn moments later
  // (a double count). The final pass accepts a hash as a last resort.
  //
  // Accepted tail risk: a message that only gets its urn AFTER the final pass can
  // still be counted twice (hash row + later urn row). It needs a slow/unconfirmed
  // send plus a later re-scrape and is vanishingly rare (in practice urns land well
  // under 3.5s). A stricter urn-only rule would silently break capture anywhere
  // LinkedIn omits the urn (e.g. possibly overlay bubbles), so we keep the fallback.
  async function flush(
    scope: Element,
    url: string,
    opts: { explicit: boolean; finalPass: boolean },
  ): Promise<void> {
    const all = scrapeMessages(scope, url);
    let fresh = all.filter((m) => !sent.has(m.li_key));
    if (!opts.finalPass) fresh = fresh.filter((m) => m.li_key.startsWith("urn:"));
    if (fresh.length === 0) {
      // An explicit send that scraped NOTHING — not even the message we just sent —
      // means the message selectors rotated. Heal, then re-scrape with the new ones.
      if (opts.finalPass && opts.explicit && all.length === 0) {
        void requestHeal(MESSAGE_KEYS, scope).then((healed) => {
          if (healed) backfill();
        });
      }
      return;
    }
    const res = await send<CaptureOutcome>({
      type: "queueMessages",
      payload: { linkedin_url: url, messages: fresh },
    });
    if (opts.explicit) reportCapture(res);
    // Mark as sent only once the SW has durably accepted them: res.ok covers both
    // "stored" and "offline" (enqueue succeeded). On a rejected round-trip — an
    // invalidated context, where the SW never ran enqueue — leave them unmarked so
    // a later pass retries, rather than silently dropping the capture this mount.
    if (res.ok) for (const m of fresh) sent.add(m.li_key);
  }

  // Schedule the post-send scrape passes for an already-resolved person.
  function capture(scope: Element, url: string, explicit: boolean): void {
    CAPTURE_PASSES_MS.forEach((ms, i) => {
      window.setTimeout(() => {
        void flush(scope, url, {
          explicit,
          finalPass: i === CAPTURE_PASSES_MS.length - 1,
        }).catch(() => {
          // Never propagate a selector/DOM error into the LinkedIn page.
        });
      }, ms);
    });
  }

  // Scrape whatever's already on screen — silent (backfill isn't a user send, so
  // no toast) and only when we can resolve a single person.
  function backfill(): void {
    const scope = currentScope();
    const who = identify(scope);
    if (who.kind === "person") capture(scope, who.identity.url, false);
  }

  // A send happened here. Resolve identity NOW (the header is present before the
  // message renders); fail safe + visible on ambiguity, else capture on render. An
  // `unknown` result on a real send is a strong signal the identity selectors
  // rotated — heal and retry once before giving up, mirroring the "Add to Prospects"
  // path so this automatic capture route self-heals too.
  async function onSend(): Promise<void> {
    const scope = currentScope();
    let who = identify(scope);
    if (who.kind === "unknown" && (await requestHeal(IDENTITY_KEYS, scope))) {
      who = identify(scope);
    }
    if (who.kind !== "person") {
      showToast("error", "Couldn't identify this chat — message not tracked.");
      return;
    }
    capture(scope, who.identity.url, true);
  }

  // Forget what we've posted this mount and re-scan the visible thread. Used
  // after a fresh "Add to Prospects" (backfill against the now-existing
  // prospect) and on thread open/switch (a persistent compose root doesn't
  // re-mount, so the newly-shown thread's replies need an explicit re-scrape).
  function resync(): void {
    sent.clear();
    backfill();
  }

  // Detect sends via delegation on the compose root, so a re-rendered Send button
  // never drops the hook. Guarded by a marker so a widget re-mount onto the same
  // compose root doesn't stack duplicate listeners (which would double-toast).
  if (composeRoot.dataset.cpSendHooked !== "1") {
    composeRoot.dataset.cpSendHooked = "1";
    composeRoot.addEventListener("click", (e) => {
      const btn = (e.target as Element | null)?.closest<HTMLButtonElement>("button");
      if (btn && looksLikeSend(btn)) void onSend();
    });
    composeRoot.addEventListener("keydown", (e) => {
      // LinkedIn sends on Enter; Shift+Enter is a newline. Only within the editor.
      if (e.key !== "Enter" || e.shiftKey) return;
      const target = e.target as Element | null;
      if (target?.closest(`${selStr("composeEditable")}, textarea`)) void onSend();
    });
    // main.ts fires this on SPA navigation (thread open/switch) so an already-
    // mounted widget re-scrapes the newly-visible thread for incoming replies.
    composeRoot.addEventListener("cp:rescan", () => resync());
  }

  backfill();

  return { resync };
}

/** Build the widget for one compose bar. `sendBtn` scopes the conversation for
 *  identity/capture; it does not affect where the row is placed. */
export function buildWidget(sendBtn: Element): HTMLElement {
  const scope = conversationScope(sendBtn);

  const root = el("div", "cp-widget");
  const feedback = el("div", "cp-feedback");
  feedback.setAttribute("role", "status");
  const select = el("select", "cp-select");
  select.title = "Pitch to run on this prospect";
  const button = el("button", "cp-add");
  button.type = "button";
  button.textContent = "Add to Prospects";
  button.disabled = true; // enabled once pitches + identity are ready
  // Read-only status shown instead of the add cluster once this person is already
  // a prospect (see refreshProspectStatus). Hidden until a lookup confirms it.
  const label = el("div", "cp-prospect-of");
  label.hidden = true;

  root.append(feedback, select, button, label);

  // The pitch list, cached once loaded so a prospect-status lookup can resolve a
  // pitch id to its name (the lookup returns only the id).
  let pitches: Pitch[] = [];
  function pitchName(id: number | null): string | null {
    if (id == null) return null;
    return pitches.find((p) => p.id === id)?.name ?? null;
  }

  let feedbackTimer: number | undefined;
  function showFeedback(kind: ToastKind, text: string): void {
    feedback.textContent = text;
    feedback.dataset.kind = kind;
    feedback.dataset.show = "true";
    window.clearTimeout(feedbackTimer);
    feedbackTimer = window.setTimeout(() => {
      delete feedback.dataset.show;
    }, 3200);
  }

  // Reflect whether we can identify a single person right now.
  function refreshIdentityState(): void {
    const result = identify(scope);
    if (result.kind === "person") {
      button.dataset.identifiable = "true";
      button.removeAttribute("title");
    } else {
      delete button.dataset.identifiable;
      button.title =
        result.kind === "group"
          ? "This looks like a group chat — can't add a single prospect."
          : "Couldn't find this person's profile.";
    }
    syncDisabled();
  }

  function syncDisabled(): void {
    const hasPitches = select.options.length > 0 && !select.disabled;
    button.disabled = !hasPitches || button.dataset.identifiable !== "true";
  }

  // Put the select into a single disabled placeholder state (no pitches usable).
  function setPlaceholder(text: string): void {
    select.disabled = true;
    const opt = el("option");
    opt.textContent = text;
    select.append(opt);
    syncDisabled();
  }

  // Show the "[Pitch ▾] [Add to Prospects]" cluster (the default: this person
  // isn't a prospect yet, or we couldn't determine it — never block adding).
  function showAddMode(): void {
    label.hidden = true;
    select.hidden = false;
    button.hidden = false;
    refreshIdentityState();
  }

  // Show the read-only "Prospect of <pitch>" status instead of the add cluster.
  // Falls back to a generic label when the pitch is unknown (deleted, or not in
  // the loaded list).
  function showProspectMode(pitchId: number | null): void {
    const name = pitchName(pitchId);
    label.textContent = name ? `Prospect of ${name}` : "Already a prospect";
    select.hidden = true;
    button.hidden = true;
    label.hidden = false;
  }

  // Guards against a stale lookup landing after a rapid thread switch: each call
  // takes a token and only the newest one is allowed to touch the UI.
  let statusToken = 0;

  // Decide which face the widget shows for the CURRENTLY open thread: if its
  // person is already a prospect, the read-only label; otherwise the add cluster.
  // Runs on mount (after pitches load) and on every thread switch (cp:rescan) —
  // the compose root and this widget persist across switches, so a mount-only
  // check would go stale the moment you open another conversation.
  async function refreshProspectStatus(): Promise<void> {
    const token = ++statusToken;
    const who = identify(scope);
    if (who.kind !== "person") {
      // Can't look up a non-person; the add cluster's own disabled state already
      // reflects the unidentifiable thread.
      showAddMode();
      return;
    }
    const res = await send<ProspectLookup>({
      type: "lookupProspect",
      payload: { linkedin_url: who.identity.url },
    });
    // A newer refresh started (the user switched threads mid-flight) — drop this.
    if (token !== statusToken) return;
    if (res.ok && res.data.exists) {
      showProspectMode(res.data.pitch_id);
    } else {
      // Not a prospect, or the lookup failed (app closed) — show the add cluster
      // either way so the user is never blocked from adding them.
      showAddMode();
    }
  }

  // Load pitches for the dropdown (fresh each time a chat's widget mounts).
  void (async () => {
    const res = await send<Pitch[]>({ type: "listPitches" });
    if (!res.ok) {
      setPlaceholder("—");
      showFeedback("error", friendlyError(res.error));
      return;
    }
    pitches = res.data;
    if (res.data.length === 0) {
      setPlaceholder("No pitches yet");
      button.title = "Create a pitch in Courland first.";
      // A prospect on a since-deleted pitch can still exist with no pitches left.
      void refreshProspectStatus();
      return;
    }
    for (const pitch of res.data) {
      const opt = el("option");
      opt.value = String(pitch.id);
      opt.textContent = pitch.name;
      select.append(opt);
    }
    // Pre-select the last-used pitch, if it's still in the list. Storage can
    // reject on a torn-down extension context — degrade to "no pre-selection"
    // (never let it reject unhandled, and still fall through to syncDisabled).
    const stored = await chrome.storage.local.get(LAST_PITCH_KEY).catch(() => ({}) as Record<string, unknown>);
    const last = stored[LAST_PITCH_KEY] as number | undefined;
    if (last != null && res.data.some((p) => p.id === last)) {
      select.value = String(last);
    }
    syncDisabled();
    // Pitches are loaded, so a lookup can now resolve the pitch name — decide
    // whether this thread's person is already a prospect.
    void refreshProspectStatus();
  })();

  refreshIdentityState();

  // Start capturing messages you send in this conversation. Capture is keyed off
  // the compose form (stable across LinkedIn's re-renders), not our widget row.
  const composeRoot =
    (sendBtn.closest(selStr("composeRoot")) as HTMLElement | null) ??
    (sendBtn.parentElement as HTMLElement | null) ??
    (scope as HTMLElement);
  const capture = setupCapture(composeRoot);

  // A thread open/switch keeps this widget mounted (the compose root persists), so
  // re-check prospect status for the newly-visible thread — otherwise the label
  // would keep showing the previous person's pitch. main.ts fires cp:rescan on
  // SPA navigation. (One listener: main.ts mounts one widget per compose root.)
  composeRoot.addEventListener("cp:rescan", () => void refreshProspectStatus());

  button.addEventListener("click", async () => {
    let result = identify(scope);
    // An explicit add that can't find the person (but isn't a group) is a strong
    // signal the identity selectors rotated — heal, then retry identify once.
    if (result.kind === "unknown" && (await requestHeal(IDENTITY_KEYS, scope))) {
      result = identify(scope);
    }
    if (result.kind !== "person") {
      showFeedback(
        "error",
        result.kind === "group"
          ? "Group chat — can't add a single prospect."
          : "Couldn't identify this person.",
      );
      return;
    }

    const pitchId = select.value ? Number(select.value) : null;
    button.disabled = true;
    button.dataset.busy = "true";
    try {
      const res = await send<AddProspectResult>({
        type: "addProspect",
        payload: {
          name: result.identity.name,
          linkedin_url: result.identity.url,
          pitch_id: pitchId,
        },
      });
      if (!res.ok) {
        showFeedback("error", friendlyError(res.error));
        return;
      }
      if (pitchId != null) {
        // Best-effort; swallow a torn-down-context rejection.
        void chrome.storage.local.set({ [LAST_PITCH_KEY]: pitchId }).catch(() => {});
      }
      // Now that they're a prospect, backfill the visible thread's messages.
      capture.resync();
      showFeedback(
        res.data.existed ? "info" : "success",
        res.data.existed ? "Already a prospect — pitch updated." : "Added to Prospects ✓",
      );
      // They're a prospect now — swap the add cluster for the read-only status.
      // Invalidate any refresh started before the add (its in-flight lookup was
      // issued when they weren't a prospect and would revert us to the add cluster).
      statusToken++;
      showProspectMode(res.data.prospect.pitch_id ?? pitchId);
    } finally {
      delete button.dataset.busy;
      syncDisabled();
    }
  });

  return root;
}
