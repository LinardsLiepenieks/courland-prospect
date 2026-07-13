// LinkedIn DOM heuristics — the single most fragile surface in this project.
// LinkedIn ships obfuscated, frequently-changing class names, so everything here
// is layered and defensive: prefer stable/semantic hooks, fall back gracefully,
// and never throw into the host page. When capture breaks, it's almost always
// here — update the selectors, keep the fallbacks.

import type { CapturedMessage, MessageDirection } from "../lib/types";

/** A person resolved from an open conversation. */
export interface Identity {
  name: string;
  url: string;
}

/** Result of trying to identify who a conversation is with. */
export type IdentityResult =
  | { kind: "person"; identity: Identity }
  | { kind: "group" } // more than one participant — can't attribute to one prospect
  | { kind: "unknown" }; // couldn't find a profile at all

/**
 * Every message-compose surface currently mounted: the full messaging pane AND
 * each overlay chat bubble. We anchor on the `msg-form` *component root* via the
 * `~=` (whitespace-token) selector, which matches the exact `msg-form` class
 * token whether it's a `<form>` or a `<div>`. LinkedIn's overlay bubbles render
 * the compose as a `div.msg-form` (no `<form>` element), which is why the old
 * `<form>`-anchored detection missed them entirely.
 */
export function findComposeRoots(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('[class~="msg-form"]'));
}

/**
 * Whether a button is a message-send control — the same layered heuristics
 * {@link findSendButton} uses, exposed so a delegated click handler can decide
 * "was this a send?" for a button that may have been re-rendered since mount.
 */
export function looksLikeSend(b: HTMLButtonElement): boolean {
  if (
    b.matches(".msg-form__send-button, .msg-form__send-btn, .msg-form__send-toggle") ||
    b.matches('button[type="submit"]')
  ) {
    return true;
  }
  const text = (b.textContent ?? "").trim().toLowerCase();
  const aria = (b.getAttribute("aria-label") ?? "").trim().toLowerCase();
  return text === "send" || aria === "send" || aria.startsWith("send ");
}

/**
 * The send control inside one compose root, matched by several fallbacks so a
 * class rename or an icon-only (aria-label) button doesn't drop it: known
 * classes → a submit button → any button labelled "send" (text or aria-label).
 */
export function findSendButton(root: HTMLElement): HTMLButtonElement | null {
  const byClass = root.querySelector<HTMLButtonElement>(
    ".msg-form__send-button, .msg-form__send-btn, .msg-form__send-toggle",
  );
  if (byClass) return byClass;

  const submit = root.querySelector<HTMLButtonElement>('button[type="submit"]');
  if (submit) return submit;

  return Array.from(root.querySelectorAll<HTMLButtonElement>("button")).find(looksLikeSend) ?? null;
}

/** Where to inject our row: a reference node plus which side of it to insert on
 *  (mirrors {@link Element.insertAdjacentElement} positions). */
export interface ComposeAnchor {
  ref: HTMLElement;
  where: "afterend" | "beforebegin";
}

/** The direct child of `root` that contains `node` — promotes a deeply-nested
 *  hit up to a sibling we can insert next to in the form's own flow. `null` when
 *  `node` isn't actually inside `root`. */
function rootChildContaining(node: HTMLElement, root: HTMLElement): HTMLElement | null {
  let cur: HTMLElement = node;
  while (cur.parentElement && cur.parentElement !== root) {
    cur = cur.parentElement;
  }
  return cur.parentElement === root ? cur : null;
}

/**
 * Where to inject our row so it sits directly below the message text area and
 * above LinkedIn's send/toolbar row. Layered like the rest of this file:
 *  1. after the text-area's own container — the contenteditable promoted to a
 *     direct child of the compose root — unless the footer is nested inside it;
 *  2. otherwise immediately before the send-button footer;
 *  3. otherwise `null` → the caller appends to the root (the original behavior),
 *     so the button is never lost when selectors drift.
 */
export function findComposeAnchor(root: HTMLElement): ComposeAnchor | null {
  const footer = root.querySelector<HTMLElement>('[class~="msg-form__footer"]');

  const editable = root.querySelector<HTMLElement>(
    '[contenteditable="true"], [role="textbox"]',
  );
  if (editable) {
    const container = rootChildContaining(editable, root);
    // If the footer lives *inside* this container, inserting after it would land
    // us below Send — fall through to the footer anchor instead.
    if (container && !(footer && container.contains(footer))) {
      return { ref: container, where: "afterend" };
    }
  }

  if (footer && footer.parentElement) return { ref: footer, where: "beforebegin" };

  return null;
}

/** The conversation container enclosing a compose button — the scope we scrape
 *  identity and messages from. Crucially it never resolves to `main`: `main` also
 *  contains the sidebar conversation list, whose entity-lockups poisoned identity
 *  (the first list item won, so we attributed to whoever sat atop the sidebar,
 *  with a junk name). We anchor to an explicit conversation container, or failing
 *  that the nearest ancestor that also encloses THIS thread's message stream. */
export function conversationScope(sendBtn: Element): Element {
  const explicit = sendBtn.closest(
    [
      ".msg-overlay-conversation-bubble",
      '[class*="msg-overlay-conversation"]',
      ".msg-convo-wrapper",
      ".msg-thread",
    ].join(","),
  );
  if (explicit) return explicit;

  // The open conversation is the tightest ancestor that also holds the message
  // stream — never the sidebar (it has no message list) and tighter than `main`
  // whenever an intermediate wrapper exists.
  let el: Element | null = sendBtn.parentElement;
  while (el) {
    if (el.querySelector('[class*="msg-s-message-list"]')) return el;
    el = el.parentElement;
  }
  return sendBtn.closest("form") ?? sendBtn.ownerDocument.body;
}

/**
 * Canonicalize a LinkedIn profile href to `https://www.linkedin.com/in/<slug>/`
 * — forcing the `www` host and a trailing slash, and dropping query/fragment.
 *
 * This is the SOLE producer of the `linkedin_url` the backend keys on. A
 * prospect and every message captured for them must carry the byte-identical
 * string, because the backend resolves a message to its prospect by *exact*
 * equality with no canonicalization of its own (see
 * `resolution_requires_the_exact_canonical_url` in the Rust messages
 * repository). Never build a profile URL any other way — a divergent form makes
 * captured messages resolve to nobody, get skipped, and — since the outbox
 * clears on a 2xx — be lost for good.
 */
function normalizeProfileUrl(href: string): string | null {
  try {
    const u = new URL(href, location.origin);
    const m = u.pathname.match(/^\/in\/([^/]+)\/?/);
    return m ? `https://www.linkedin.com/in/${m[1]}/` : null;
  } catch {
    return null;
  }
}

/** Strip the junk LinkedIn hangs off a name inside lockups/aria-labels —
 *  connection degree, presence ("Status is reachable"), "Mobile • 8m ago",
 *  action labels ("Manage prospect") — keeping the leading real name. This is why
 *  the broken row read "Artemi Vaarakallio Manage prospect Status is reachable
 *  Mobile • 8m ago": the whole lockup text was taken verbatim. Best-effort; a
 *  dedicated title node (see {@link nameFrom}) is preferred over this. */
function cleanText(raw: string): string {
  let s = raw.replace(/\s+/g, " ").trim();
  // Cut at the first marker LinkedIn appends after the name.
  s = s.split(
    /\s+(?:Status is\b|is reachable\b|Active\b|Mobile\b|Manage prospect\b|View profile\b|Message\b)/i,
  )[0];
  s = s.replace(/\s*•.*$/, ""); // "• 1st", "• 8m ago", "• Online"
  s = s.replace(/\s*\b(?:1st|2nd|3rd)\b.*$/i, ""); // stray connection degree
  return s.trim();
}

function slugName(url: string): string {
  const m = url.match(/\/in\/([^/]+)/);
  if (!m) return "LinkedIn member";
  // A malformed percent-escape in the slug makes decodeURIComponent throw
  // URIError; identify() runs from unguarded event handlers, so degrade to the
  // raw slug rather than throwing into the LinkedIn page.
  let raw: string;
  try {
    raw = decodeURIComponent(m[1]);
  } catch {
    raw = m[1];
  }
  return raw
    .replace(/-[0-9a-f]{6,}$/i, "") // strip trailing id hash
    .replace(/-/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

/** Containers whose links must NEVER be treated as the open thread's identity:
 *  the sidebar conversation list and the message stream itself. Reading these was
 *  the capture bug — the sidebar's first lockup won, so identity became whoever
 *  sat atop the list rather than who the thread is actually with. */
const NON_IDENTITY = [
  '[class*="msg-conversations-container"]',
  '[class*="conversation-listitem"]',
  '[class*="msg-overlay-list-bubble"]',
  '[class*="msg-s-message-list"]',
  '[class*="msg-s-event-listitem"]',
].join(",");

function inNonIdentity(el: Element): boolean {
  return el.closest(NON_IDENTITY) !== null;
}

/** Distinct normalized profile URLs → the anchor first seen for each, from links
 *  already filtered to the header region. */
function distinctProfiles(links: HTMLAnchorElement[]): Map<string, HTMLAnchorElement> {
  const byUrl = new Map<string, HTMLAnchorElement>();
  for (const a of links) {
    const url = normalizeProfileUrl(a.getAttribute("href") ?? "");
    if (url && !byUrl.has(url)) byUrl.set(url, a);
  }
  return byUrl;
}

/** A clean display name for the resolved person: prefer a dedicated title node in
 *  the thread header, then the anchor's aria-label, then its text — each run
 *  through {@link cleanText} — and finally the URL slug. Never the lockup blob. */
function nameFrom(anchor: HTMLAnchorElement, url: string): string {
  const header = anchor.closest(
    '[class*="msg-title-bar"], [class*="msg-entity-lockup"], [class*="msg-thread"], [class*="overlay-conversation"]',
  );
  const titleNode = header?.querySelector(
    'h2, [class*="entity-title"], [class*="title-bar__title"], [class*="thread__title"]',
  );
  for (const raw of [
    titleNode?.textContent ?? "",
    anchor.getAttribute("aria-label") ?? "",
    anchor.textContent ?? "",
  ]) {
    const name = cleanText(raw);
    if (name) return name;
  }
  return slugName(url);
}

/**
 * Identify who an open conversation is with, reading ONLY the thread header —
 * never the sidebar list or the message stream (see {@link NON_IDENTITY}). A 1:1
 * thread resolves to exactly one distinct profile; more than one means a group we
 * can't attribute to a single prospect; none means we couldn't tell.
 */
export function identify(scope: Element): IdentityResult {
  const headerSelectors = [
    "a.msg-thread__link-to-profile",
    "a.msg-connection-card__link",
    '[class*="msg-title-bar"] a[href*="/in/"]',
    '[class*="msg-entity-lockup"] a[href*="/in/"]',
    '[class*="overlay-conversation"] a[href*="/in/"]',
  ];
  for (const sel of headerSelectors) {
    const links = Array.from(scope.querySelectorAll<HTMLAnchorElement>(sel)).filter(
      (a) => !inNonIdentity(a),
    );
    const byUrl = distinctProfiles(links);
    if (byUrl.size === 1) {
      const [[url, anchor]] = [...byUrl];
      return { kind: "person", identity: { name: nameFrom(anchor, url), url } };
    }
    if (byUrl.size > 1) return { kind: "group" };
  }

  // Fallback: any profile link in scope that isn't in the sidebar list or stream.
  const links = Array.from(scope.querySelectorAll<HTMLAnchorElement>('a[href*="/in/"]')).filter(
    (a) => !inNonIdentity(a),
  );
  const byUrl = distinctProfiles(links);
  if (byUrl.size === 0) return { kind: "unknown" };
  if (byUrl.size > 1) return { kind: "group" };
  const [[url, anchor]] = [...byUrl];
  return { kind: "person", identity: { name: nameFrom(anchor, url), url } };
}

// ── Conversation list (for batch drafting) ───────────────────────────────────
// The messaging sidebar: the list of threads we walk "top to bottom" to pick the
// N most-recent conversations, plus the header we mount the "Draft for" control
// into. Same fragility rules as the rest of this file — layered, fail-quiet.

/** The top `n` conversation rows' clickable elements, in list order (most-recent
 *  first). LinkedIn's list rows carry NO thread href — the clickable is a
 *  `<div class="msg-conversation-listitem__link" tabindex="0">` that navigates via
 *  a JS handler — so callers click these to open a thread rather than reading a
 *  URL. Re-query per step (don't cache the whole set): the list can re-render on
 *  navigation and invalidate stale node references. */
export function topThreadRows(n: number): HTMLElement[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>('[class*="msg-conversation-listitem__link"]'),
  ).slice(0, n);
}

/** The current conversation's canonical URL when the messaging pane is on a
 *  thread (`/messaging/thread/<id>/`), else `null`. Normalized to origin+pathname
 *  (no query/hash). Used to confirm a row activation actually navigated the pane
 *  to a (new) thread before drafting. */
export function currentThreadUrl(): string | null {
  if (!location.pathname.includes("/messaging/thread/")) return null;
  return `${location.origin}${location.pathname}`;
}

/** Open a conversation row by LinkedIn's own keyboard affordance: focus it and
 *  press Enter (rows are `tabindex=0` and advertise "Press return to go to
 *  conversation details"). A bare `.click()` doesn't trigger their SPA router;
 *  the keydown does. Fail-quiet — a detached node just no-ops. */
export function activateRow(row: HTMLElement): void {
  try {
    row.scrollIntoView({ block: "nearest" });
    row.focus();
    // Drive both the primary click path and the keyboard affordance — whichever
    // LinkedIn's handler is bound to. All bubbling so the framework's delegated
    // listener at the document root receives them.
    const mouse: MouseEventInit = { bubbles: true, cancelable: true, view: window };
    for (const type of ["mousedown", "mouseup", "click"] as const) {
      row.dispatchEvent(new MouseEvent(type, mouse));
    }
    for (const type of ["keydown", "keyup"] as const) {
      row.dispatchEvent(
        new KeyboardEvent(type, { key: "Enter", code: "Enter", bubbles: true, cancelable: true }),
      );
    }
  } catch {
    // Never propagate into the host page.
  }
}

/** Whether this conversation row is the one currently open in the reading pane.
 *  LinkedIn marks the open row's link with a `…convo-item-link--active` class.
 *  Matched on the specific token (not a bare `--active`) so a focus/hover state
 *  can't false-positive. Lets the drafter confirm the intended thread actually
 *  opened before writing into it. */
export function isRowActive(row: HTMLElement): boolean {
  return row.className.includes("convo-item-link--active");
}

/** The list position of the conversation currently open in the reading pane —
 *  the row marked `…convo-item-link--active`. `0` when none is active (a fresh
 *  inbox before any thread is opened), so a batch started there counts from the
 *  top. This is the "Draft for N" start offset: the batch drafts for the selected
 *  conversation and the N-1 below it. Read in the foreground tab at click time,
 *  where the live list reflects the user's current selection. */
export function selectedRowIndex(): number {
  const idx = topThreadRows(Number.MAX_SAFE_INTEGER).findIndex(isRowActive);
  return idx < 0 ? 0 : idx;
}

/** LinkedIn's messaging inbox top bar — the row holding the "Messaging" title,
 *  search, the overflow (⋯) dropdown, and the compose button. The "Draft for"
 *  control mounts as its own full-width row directly *after* this (below the bar,
 *  above the thread list). Addressed by its `data-test` hook (obfuscated classes
 *  churn; the test id is steadier), with a class-substring fallback. */
export function inboxTopBar(): HTMLElement | null {
  return (
    document.querySelector<HTMLElement>("[data-test-msg-cross-pillar-inbox-top-bar-wrapper]") ??
    document.querySelector<HTMLElement>('[class*="inbox-top-bar-wrapper__container"]')
  );
}

/** The inbox filter row (Inbox / Jobs / Unread / …) — the second row in the
 *  messaging header stack, below the top bar. The "Draft for" control mounts as
 *  a new row directly *after* this, so it sits at the bottom of the header,
 *  right above the thread list. */
export function inboxFilterRow(): HTMLElement | null {
  return document.querySelector<HTMLElement>(
    '[class*="msg-conversations-container__title-row"]',
  );
}

// ── Inbox filter pills ────────────────────────────────────────────────────────
// The single-select pill row (Jobs / Unread / Connections / InMail / Starred) in
// the messaging header. Each pill carries a stable
// `data-test-messaging-inbox-filters__filter-pill="<TOKEN>"` and marks its
// selected state with `aria-pressed="true"` (and `aria-checked="true"`). A batch
// reads which pill is active in the foreground and re-applies it in each draft tab,
// so a filtered view's
// list positions line up with the tab's list.

/** The stable per-pill token attribute (e.g. value "UNREAD"). */
const FILTER_PILL_ATTR = "data-test-messaging-inbox-filters__filter-pill";
const FILTER_PILL_GROUP = "[data-test-messaging-inbox-filters__filter-pill-group]";

/** The single-select pill row container, or null when the header isn't rendered. */
function filterPillGroup(): HTMLElement | null {
  return document.querySelector<HTMLElement>(FILTER_PILL_GROUP);
}

/** Whether a filter pill is the currently-selected one. LinkedIn marks it with
 *  `aria-pressed="true"` (and `aria-checked="true"`); either counts. */
export function isInboxFilterActive(pill: HTMLElement): boolean {
  return pill.getAttribute("aria-pressed") === "true" || pill.getAttribute("aria-checked") === "true";
}

/** The token of the inbox filter pill currently active (e.g. `"UNREAD"`), or
 *  `null` when none is selected (the default, unfiltered inbox). Read in the
 *  foreground at click time so a batch can carry the same filter into its tabs. */
export function activeInboxFilter(): string | null {
  const group = filterPillGroup();
  if (!group) return null;
  for (const pill of Array.from(group.querySelectorAll<HTMLElement>(`[${FILTER_PILL_ATTR}]`))) {
    if (isInboxFilterActive(pill)) return pill.getAttribute(FILTER_PILL_ATTR);
  }
  return null;
}

/** The filter pill button for a token (e.g. `"UNREAD"`), or `null` if the pill
 *  row isn't rendered yet / the token isn't offered. `token` is caller-sanitized
 *  to `[A-Z_]+` before it reaches this attribute selector. */
export function inboxFilterPill(token: string): HTMLElement | null {
  return filterPillGroup()?.querySelector<HTMLElement>(`[${FILTER_PILL_ATTR}="${token}"]`) ?? null;
}

/** Select a filter pill by clicking it. Fail-quiet (a detached node just no-ops),
 *  keeping every LinkedIn DOM write inside this module. */
export function clickFilterPill(pill: HTMLElement): void {
  try {
    pill.click();
  } catch {
    // Never propagate into the host page.
  }
}

/** Whether a thread's composer already holds user-entered text — so the drafter
 *  can skip it rather than overwrite a hand-typed unsent reply (LinkedIn restores
 *  a per-conversation draft into the box when you open the thread). Reads the same
 *  editor `writeComposer` targets; strips zero-width/NBSP so only real visible
 *  text counts (an empty composer's `textContent` is empty/whitespace). */
export function composerHasText(root: HTMLElement): boolean {
  const editable = root.querySelector<HTMLElement>(
    '[contenteditable="true"], [role="textbox"]',
  );
  const text = (editable?.textContent ?? "").replace(/[\u200B-\u200D\uFEFF\u00A0]/g, "").trim();
  return text.length > 0;
}

/** Write `text` into a compose root's editor, replacing whatever's there, and
 *  fire the input events LinkedIn's editor listens for so its Send button
 *  enables. Returns whether the text was written (false when no editor was
 *  found). Uses `execCommand("insertText")` — deprecated but still the one
 *  reliable way to drive a contenteditable so the page's own framework registers
 *  the change (a raw `textContent` set does not); the caller runs this only once
 *  the tab is focused, since execCommand acts on the active document. Never
 *  sends. */
export function writeComposer(root: HTMLElement, text: string): boolean {
  const editable = root.querySelector<HTMLElement>(
    '[contenteditable="true"], [role="textbox"]',
  );
  if (!editable) return false;

  editable.focus();
  const sel = window.getSelection();
  if (sel) {
    const range = document.createRange();
    range.selectNodeContents(editable);
    sel.removeAllRanges();
    sel.addRange(range);
  }
  // Replace the (now-selected) contents with the draft. execCommand dispatches the
  // beforeinput/input events LinkedIn's editor depends on.
  const inserted = document.execCommand("insertText", false, text);
  if (inserted) return true;

  // Fallback for the day execCommand finally goes away: set text and dispatch a
  // best-effort input event so the framework still sees a change. Report success
  // only if the text actually landed in the DOM — so a write that didn't take
  // surfaces an error rather than a false "Draft ready".
  editable.textContent = text;
  editable.dispatchEvent(
    new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }),
  );
  return editable.textContent === text;
}

// ── Message scraping ─────────────────────────────────────────────────────────
// The other fragile surface (see the file header). We read the messages already
// rendered in the thread and classify each as sent-by-YOU or a reply. Everything
// here is best-effort and must never throw into the page.

const MESSAGE_ITEM = ".msg-s-event-listitem";
/** LinkedIn's own modifier marking the *counterparty's* messages. */
const OTHER_MODIFIER = "msg-s-event-listitem--other";
const MESSAGE_BODY = ".msg-s-event-listitem__body";

/** Stable, order-independent fallback key (djb2 → hex) for when no DOM id/urn is
 *  present. Keyed on content + timestamp so a re-scrape of the same message
 *  produces the same key and dedups. */
function hashKey(input: string): string {
  let h = 5381;
  for (let i = 0; i < input.length; i++) h = (Math.imul(h, 33) + input.charCodeAt(i)) | 0;
  return (h >>> 0).toString(16);
}

/** The profile URL of a message's sender, if one is discoverable — used to
 *  classify direction when LinkedIn's `--other` class is absent. */
function senderUrl(item: Element): string | null {
  const link =
    item.querySelector<HTMLAnchorElement>('a[href*="/in/"]') ??
    item
      .closest('.msg-s-message-group, li[class*="msg-s-message-list__event"]')
      ?.querySelector<HTMLAnchorElement>('a[href*="/in/"]') ??
    null;
  return link ? normalizeProfileUrl(link.getAttribute("href") ?? "") : null;
}

/**
 * LinkedIn's per-message timestamp as a STABLE machine value — only the `time`
 * element's `datetime` attribute, never its visible text. The rendered text is a
 * relative label ("Just now" → "1m" → "5m") that mutates over time; folding that
 * into `li_key` below would make the same message re-key on every re-scrape and
 * double-count. `null` when no stable datetime is present.
 */
function messageTimestamp(item: Element): string | null {
  return item.querySelector("time")?.getAttribute("datetime") ?? null;
}

/**
 * Every message currently rendered in `scope`, in document order, each tagged
 * with its {@link MessageDirection}. `partnerUrl` is the conversation partner
 * from {@link identify} and drives the classification: a message is INCOMING
 * when it carries LinkedIn's `--other` modifier OR its visible sender is the
 * partner; otherwise it's OUTGOING (your side). Using both signals keeps a
 * reply-less outreach thread (all yours) correct while still recognising the
 * partner's messages if the `--other` class ever gets renamed.
 *
 * Only messages with body text are returned (system rows / bare attachments are
 * skipped). Virtualized-off-screen messages simply aren't seen — they're picked
 * up on the next scrape when scrolled into view.
 */
export function scrapeMessages(scope: Element, partnerUrl: string): CapturedMessage[] {
  const out: CapturedMessage[] = [];
  for (const item of Array.from(scope.querySelectorAll<HTMLElement>(MESSAGE_ITEM))) {
    const sender = senderUrl(item);
    const isIncoming =
      item.classList.contains(OTHER_MODIFIER) || (sender != null && sender === partnerUrl);
    const direction: MessageDirection = isIncoming ? "incoming" : "outgoing";

    const body = (item.querySelector(MESSAGE_BODY)?.textContent ?? "")
      .replace(/\s+/g, " ")
      .trim();
    if (!body) continue;

    const sent_at = messageTimestamp(item);
    const li_key =
      item.getAttribute("data-event-urn") ??
      item.querySelector("[data-event-urn]")?.getAttribute("data-event-urn") ??
      hashKey(`${body}|${sent_at ?? ""}`);
    out.push({ li_key, body, sent_at, direction });
  }
  return out;
}
