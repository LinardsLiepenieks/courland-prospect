// Selector self-heal (content-script side).
//
// When a preflight/guard in the capture flow finds that a LinkedIn selector no
// longer matches, `requestHeal` captures the live DOM, shows the "updating…" toast,
// and asks the Courland app to repair it (the app runs the page through the local
// Claude Code CLI and returns new selector values — see the Rust `/heal-selectors`
// route). The returned overrides are applied live via `applyOverrides`, so the very
// next capture attempt uses the fixed selectors — no rebuild/reload.
//
// Guarded against storms and loops: one heal in flight at a time, a per-session
// attempt cap, and per-key dedup (a key that was already healed this session won't
// trigger another round). If Claude Code isn't reachable, it degrades to an error
// toast rather than capturing with stale selectors.

import { friendlyError, send } from "./bridge";
import {
  applyOverrides,
  currentSerialized,
  DESCRIPTIONS,
  type SelectorKey,
} from "./selectors";
import { showToast } from "./toast";
import type { SelectorOverrides } from "../lib/types";

/** Ceiling on heal attempts PER KEY — a backstop against a heal→still-broken loop
 *  when the DOM changed in a way Claude can't resolve. Tracked per key (not one
 *  global counter) so a single stubborn surface can't exhaust the whole tab's
 *  budget and block every other selector from ever healing. */
const MAX_ATTEMPTS = 3;

/** Trim the captured HTML client-side (the backend caps again). Scoped + stripped
 *  DOM keeps this well under the CLI arg limit and drops the noisiest nodes. */
const MAX_HTML_CHARS = 150_000;

/** Selector groups per heal trigger — the keys a given failure implicates. */
export const MOUNT_KEYS: SelectorKey[] = [
  "composeRoot",
  "sendButtonClasses",
  "sendSubmit",
  "composeFooter",
  "composeEditable",
];
export const IDENTITY_KEYS: SelectorKey[] = [
  "identityHeaders",
  "profileLink",
  "nonIdentity",
  "nameHeaderContainer",
  "nameTitleNode",
];
export const MESSAGE_KEYS: SelectorKey[] = [
  "messageItem",
  "messageBody",
  "messageGroup",
  "messageOtherModifier",
];
/** Selector groups for the batch-draft thread list: the clickable conversation
 *  rows and the token marking the open one. Rotation here silently breaks the
 *  "Draft for N" cycle (no rows found, or none ever confirms as open). */
export const THREAD_KEYS: SelectorKey[] = ["threadRow", "activeRowToken"];

let inFlight = false;
const healedKeys = new Set<SelectorKey>();
/** Failed heal attempts per key — each key has its own {@link MAX_ATTEMPTS}
 *  budget, so a broken surface that Claude can't resolve stops retrying without
 *  starving the others. */
const attemptsByKey = new Map<SelectorKey, number>();

/** Fetch persisted selector overrides from the app and apply them over the
 *  compiled defaults. Best-effort: on any failure the extension just runs on its
 *  defaults (today's behavior). Call once at content-script startup. */
export async function bootstrapSelectors(): Promise<void> {
  const res = await send<SelectorOverrides>({ type: "getSelectors" });
  if (res.ok) applyOverrides(res.data);
}

/** Scoped, denoised, size-capped HTML for the heal request. Scopes to `root` (or
 *  the whole document when none given), removes media/script/style nodes and inline
 *  styles — keeping the class/aria/role/data attributes and text selectors match on. */
function captureHtml(root?: Element | null): string {
  try {
    const base = root ?? document.body ?? document.documentElement;
    const clone = base.cloneNode(true) as Element;
    clone
      .querySelectorAll("svg, script, style, noscript, img, video, canvas, picture, source")
      .forEach((n) => n.remove());
    clone.querySelectorAll("[style]").forEach((n) => n.removeAttribute("style"));
    const html = clone.outerHTML ?? "";
    return html.length > MAX_HTML_CHARS ? html.slice(0, MAX_HTML_CHARS) : html;
  } catch {
    return "";
  }
}

/**
 * Ask the app to repair `brokenKeys`, given the DOM under `captureRoot`. Returns
 * true when new selectors were applied (the caller should retry the operation),
 * false otherwise (already healing, cap reached, nothing new, or Claude failed).
 * Never throws.
 */
export async function requestHeal(
  brokenKeys: SelectorKey[],
  captureRoot?: Element | null,
): Promise<boolean> {
  if (inFlight) return false;
  // Skip keys already healed this session (avoids a fix→still-fails loop churning
  // the same keys) and keys that have spent their per-key attempt budget.
  const keys = brokenKeys.filter(
    (k) => !healedKeys.has(k) && (attemptsByKey.get(k) ?? 0) < MAX_ATTEMPTS,
  );
  if (keys.length === 0) return false;

  inFlight = true;
  for (const k of keys) attemptsByKey.set(k, (attemptsByKey.get(k) ?? 0) + 1);
  showToast("info", "Updating selectors… please wait");
  try {
    const broken = keys.map((k) => ({
      key: k,
      description: DESCRIPTIONS[k],
      current: currentSerialized(k),
    }));
    const res = await send<{ selectors: SelectorOverrides }>({
      type: "healSelectors",
      payload: { html: captureHtml(captureRoot), url: location.href, broken },
    });
    if (!res.ok) {
      showToast("error", friendlyError(res.error));
      return false;
    }
    // Only the keys that actually applied count as healed. A reply may fix some
    // requested keys but omit or botch others; marking every requested key as done
    // (as long as one applied) would permanently block a retry for the still-broken
    // ones this session. Dedup only what genuinely landed.
    const applied = applyOverrides(res.data.selectors);
    if (applied.length === 0) {
      showToast("info", "Couldn't update selectors — try again.");
      return false;
    }
    for (const k of applied) healedKeys.add(k);
    showToast("success", "Selectors updated ✓");
    return true;
  } catch {
    showToast("error", "Couldn't update selectors.");
    return false;
  } finally {
    inFlight = false;
  }
}
