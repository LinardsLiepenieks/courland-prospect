// LinkedIn DOM heuristics — the single most fragile surface in this project.
// LinkedIn ships obfuscated, frequently-changing class names, so everything here
// is layered and defensive: prefer stable/semantic hooks, fall back gracefully,
// and never throw into the host page. When capture breaks, it's almost always
// here — update the selectors, keep the fallbacks.

import type { CapturedMessage, MessageDirection } from "../lib/types";
import { selList, selStr } from "./selectors";

/** True when a conversation is actually OPEN — a thread URL, or a rendered message
 *  stream. This is the signal that a compose form SHOULD exist: if none is found
 *  here, the compose selectors have likely rotated (vs. the inbox list, where no
 *  composer is expected). */
export function threadIsOpen(): boolean {
  return (
    location.pathname.includes("/messaging/thread/") ||
    document.querySelector(selStr("messageList")) !== null
  );
}

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
  return Array.from(document.querySelectorAll<HTMLElement>(selStr("composeRoot")));
}

/**
 * Whether a button is a message-send control — the same layered heuristics
 * {@link findSendButton} uses, exposed so a delegated click handler can decide
 * "was this a send?" for a button that may have been re-rendered since mount.
 */
export function looksLikeSend(b: HTMLButtonElement): boolean {
  if (b.matches(selStr("sendButtonClasses")) || b.matches(selStr("sendSubmit"))) {
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
  const byClass = root.querySelector<HTMLButtonElement>(selStr("sendButtonClasses"));
  if (byClass) return byClass;

  const submit = root.querySelector<HTMLButtonElement>(selStr("sendSubmit"));
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
  const footer = root.querySelector<HTMLElement>(selStr("composeFooter"));

  const editable = root.querySelector<HTMLElement>(selStr("composeEditable"));
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
  const explicit = sendBtn.closest(selStr("convoExplicit"));
  if (explicit) return explicit;

  // The open conversation is the tightest ancestor that also holds the message
  // stream — never the sidebar (it has no message list) and tighter than `main`
  // whenever an intermediate wrapper exists.
  const messageList = selStr("messageList");
  let el: Element | null = sendBtn.parentElement;
  while (el) {
    if (el.querySelector(messageList)) return el;
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
function inNonIdentity(el: Element): boolean {
  return el.closest(selStr("nonIdentity")) !== null;
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
  const header = anchor.closest(selStr("nameHeaderContainer"));
  const titleNode = header?.querySelector(selStr("nameTitleNode"));
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
 * never the sidebar list or the message stream (the `nonIdentity` selector). A 1:1
 * thread resolves to exactly one distinct profile; more than one means a group we
 * can't attribute to a single prospect; none means we couldn't tell.
 */
export function identify(scope: Element): IdentityResult {
  for (const headerSel of selList("identityHeaders")) {
    const links = Array.from(scope.querySelectorAll<HTMLAnchorElement>(headerSel)).filter(
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
  const links = Array.from(scope.querySelectorAll<HTMLAnchorElement>(selStr("profileLink"))).filter(
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
  return Array.from(document.querySelectorAll<HTMLElement>(selStr("threadRow"))).slice(0, n);
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
  return row.className.includes(selStr("activeRowToken"));
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
    document.querySelector<HTMLElement>(selStr("inboxTopBarPrimary")) ??
    document.querySelector<HTMLElement>(selStr("inboxTopBarFallback"))
  );
}

/** The inbox filter row (Inbox / Jobs / Unread / …) — the second row in the
 *  messaging header stack, below the top bar. The "Draft for" control mounts as
 *  a new row directly *after* this, so it sits at the bottom of the header,
 *  right above the thread list. */
export function inboxFilterRow(): HTMLElement | null {
  return document.querySelector<HTMLElement>(selStr("inboxFilterRow"));
}

/** Whether a thread's composer already holds user-entered text — so the drafter
 *  can skip it rather than overwrite a hand-typed unsent reply (LinkedIn restores
 *  a per-conversation draft into the box when you open the thread). Reads the same
 *  editor `writeComposer` targets; strips zero-width/NBSP so only real visible
 *  text counts (an empty composer's `textContent` is empty/whitespace). */
export function composerHasText(root: HTMLElement): boolean {
  return hasVisibleText(root.querySelector<HTMLElement>(selStr("composeEditable")));
}

/** Whether an editable holds real, visible text \u2014 ignoring the zero-width and
 *  NBSP characters an "empty" rich editor (LinkedIn's composer or comment box)
 *  can leave behind. Shared by the message-composer and comment-box checks so the
 *  non-obvious strip regex lives in one place. */
function hasVisibleText(editable: HTMLElement | null): boolean {
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
  const editable = root.querySelector<HTMLElement>(selStr("composeEditable"));
  if (!editable) return false;
  return writeIntoEditable(editable, text);
}

/** Drive `text` into a contenteditable so the page's own framework registers the
 *  change: focus, select all, then `execCommand("insertText")` (which dispatches
 *  the beforeinput/input events a rich editor like LinkedIn's ProseMirror depends
 *  on — a raw `textContent` set does not). Falls back to a direct set + input
 *  event, reporting success only if the text actually landed. Shared by the
 *  message composer and the comment box; the caller runs it only once the tab is
 *  focused (execCommand acts on the active document). Never sends. */
function writeIntoEditable(editable: HTMLElement, text: string): boolean {
  editable.focus();
  const sel = window.getSelection();
  if (sel) {
    const range = document.createRange();
    range.selectNodeContents(editable);
    sel.removeAllRanges();
    sel.addRange(range);
  }
  const inserted = document.execCommand("insertText", false, text);
  if (inserted) return true;

  editable.textContent = text;
  editable.dispatchEvent(
    new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }),
  );
  // Compare with ALL whitespace stripped: a rich editor (ProseMirror) wraps newlines
  // in <p> blocks, so a multi-line comment's `textContent` ("line1line2") never
  // equals the source ("line1\nline2") verbatim — a literal `=== text` check would
  // report a false failure on any multi-line text that actually landed correctly.
  const strip = (v: string): string => v.replace(/\s+/g, "");
  return strip(editable.textContent ?? "") === strip(text);
}

// ── Posts & comments (the LinkedIn commenter) ────────────────────────────────
// Reading posts from the feed / a profile's recent-activity, and placing a
// drafted comment into a post's comment box. Same fragility rules as the rest of
// this file — layered, fail-quiet, selectors centralized + healable.

/** The activity URN carried on a post container (`urn:li:activity:…`, or a
 *  `ugcPost`/`share` variant), read from its own `data-urn`/`data-id`, the nearest
 *  ANCESTOR carrying one, or a descendant. `null` when none is present.
 *
 *  The ancestor walk matters: the block we anchor scraping on is the
 *  `data-display-contents="true"` "Feed post" wrapper, but LinkedIn hangs the post's
 *  `data-urn` on the OUTER update container (`.feed-shared-update-v2`) that WRAPS
 *  that block — so a descendant-only search missed it on most feed posts, forcing
 *  the (unreliable) clipboard fallback. Checking `closest()` recovers the real
 *  activity URN straight from the DOM, no clipboard needed. */
function postUrn(container: HTMLElement): string | null {
  const re = /urn:li:(?:activity|ugcPost|share):\d+/;
  for (const attr of ["data-urn", "data-id"]) {
    const own = container.getAttribute(attr)?.match(re);
    if (own) return own[0];
    // The update container this block lives inside (or the block itself).
    const anc = container.closest(`[${attr}*="urn:li:"]`)?.getAttribute(attr)?.match(re);
    if (anc) return anc[0];
    // Scan ALL descendants carrying this attr, not just the first: the first
    // `data-urn` under a container can be a non-post URN (a member/comment), while
    // a deeper element carries the real post URN — inspecting only the first would
    // miss it and drop an otherwise-commentable post.
    for (const el of Array.from(container.querySelectorAll(`[${attr}*="urn:li:"]`))) {
      const m = el.getAttribute(attr)?.match(re);
      if (m) return m[0];
    }
  }
  return null;
}

/** The canonical permalink for a post from its activity URN. LinkedIn resolves
 *  `/feed/update/<urn>/` to the post, so we build the permalink from the stable
 *  URN rather than scraping (and clicking) the post's own "copy link" menu. */
function permalinkFromUrn(urn: string): string {
  return `https://www.linkedin.com/feed/update/${urn}/`;
}

/** Collapse an exact double of a string back to a single copy. LinkedIn's actor
 *  name renders twice — a visible span plus a visually-hidden a11y span — so a
 *  naive `textContent` yields "Ada LovelaceAda Lovelace"; this restores "Ada
 *  Lovelace". Handles the doubling with or without a single separating space, and
 *  leaves a non-doubled string untouched. */
function dedupeDoubled(s: string): string {
  const n = s.length;
  if (n >= 2 && n % 2 === 0 && s.slice(0, n / 2) === s.slice(n / 2)) {
    return s.slice(0, n / 2);
  }
  const mid = (n - 1) / 2;
  if (n % 2 === 1 && s[mid] === " " && s.slice(0, mid) === s.slice(mid + 1)) {
    // Only treat a space-separated exact double as the a11y artifact when the
    // repeated unit is itself multi-word ("Ada Lovelace Ada Lovelace"). A
    // single-token double like "Jan Jan" is far more likely a genuine name than the
    // visible+hidden-span doubling, so leave it intact rather than halve a real name.
    const unit = s.slice(0, mid);
    if (unit.includes(" ")) return unit;
  }
  return s;
}

/** The activity URN embedded in a post permalink, or `null` when the URL carries
 *  none. Handles both the `/feed/update/urn:li:<type>:<id>/` form (built by
 *  {@link permalinkFromUrn}) and a raw `/posts/…-<type>-<id>-<hash>/` slug, so the
 *  post tab can still match a container even if a `/posts/` URL was stored directly. */
function urnFromPostUrl(url: string): string | null {
  const direct = url.match(/urn:li:(?:activity|ugcPost|share):\d+/)?.[0];
  if (direct) return direct;
  const typed = url.match(/-(activity|ugcPost|share)-(\d{15,25})/);
  return typed ? `urn:li:${typed[1]}:${typed[2]}` : null;
}

/**
 * The post container on the page matching `url`'s activity URN, or `null` when it
 * isn't present (yet). A post permalink page (`/feed/update/<urn>/`) also renders
 * *recommended* posts, each with its own comment button and lazily-mounted editor,
 * so the target post is NOT reliably first in the DOM — scoping the composer lookup
 * to the container whose URN matches the permalink is what keeps a drafted comment
 * from landing in a neighbouring post's box. */
export function postContainerForUrl(url: string): HTMLElement | null {
  const urn = urnFromPostUrl(url);
  if (!urn) return null;
  const containers = Array.from(document.querySelectorAll<HTMLElement>(selStr("postContainer")));
  // Exact URN match — the reliable case.
  for (const c of containers) {
    if (postUrn(c) === urn) return c;
  }
  // Fallback: LinkedIn sometimes serves a share/ugcPost permalink under a different
  // URN *type* (e.g. an activity URN) for the same post, so an exact match misses and
  // the post is silently dropped. Match on the shared trailing numeric id instead —
  // but ONLY when EXACTLY ONE container carries it, so a permalink page's recommended
  // posts can never resolve to the WRONG post (an ambiguous page yields null, the
  // same safe outcome as before). When ids differ across URN types this matches
  // nothing, which is also safe.
  const id = urn.match(/:(\d+)$/)?.[1];
  if (id) {
    const byId = containers.filter((c) => postUrn(c)?.endsWith(`:${id}`));
    if (byId.length === 1) return byId[0];
  }
  // Last resort: on the post's OWN permalink page (`/feed/update/<urn>/` or
  // `/posts/<slug>/`) the target IS the primary post — the first update container in
  // document order; recommended posts render after it. This rescues a valid link
  // whose stored URN *type* differs from the container's (a `/posts/…-share-…` link
  // vs the container's activity URN — different ids, so exact/id match can't hit).
  // GATED to a real single-post permalink location so it can never fire on the feed
  // and grab an arbitrary post. Requires a non-empty post body so a header/promo
  // shell isn't mistaken for the post.
  const path = location.pathname;
  const onPermalinkPage = path.includes("/feed/update/") || path.includes("/posts/");
  if (onPermalinkPage) {
    const primary = containers.find((c) => postBodyText(c).length >= 30);
    if (primary) return primary;
  }
  return null;
}

/** Canonicalize a post URL to `origin + pathname` with a trailing slash (dropping
 *  query + fragment) — the key a drafted comment is cached under and that a review
 *  tab recomputes from its own location to read the draft back. Both sides run
 *  this identical normalization, so the keys match after navigation. */
export function normalizePostUrl(href: string): string {
  try {
    const u = new URL(href, location.origin);
    const path = u.pathname.endsWith("/") ? u.pathname : `${u.pathname}/`;
    return `${u.origin}${path}`;
  } catch {
    return href;
  }
}

/** The rendered text of an element — `innerText` (rendered, respects visibility)
 *  when available, falling back to `textContent` so it still works in a background
 *  tab that hasn't laid out. */
function renderedText(el: Element): string {
  return (el as HTMLElement).innerText || el.textContent || "";
}

/**
 * Post containers currently in the DOM. Primary anchor: LinkedIn tags each feed
 * item's wrapper `div[data-display-contents="true"]` and prefixes its text with the
 * a11y label "Feed post …" — a class-independent, rotation-proof hook (the same one
 * the reference scraper anchors on, which is why it keeps working when the obfuscated
 * class names churn). Unioned with the class/URN-based `postContainer` selector so a
 * profile's recent-activity page (no "Feed post" prefix) is still covered. Deduped,
 * in document order.
 */
function findPostContainers(): HTMLElement[] {
  const found: HTMLElement[] = [];
  const seen = new Set<Element>();
  const add = (el: HTMLElement): void => {
    if (!seen.has(el)) {
      seen.add(el);
      found.push(el);
    }
  };
  for (const d of Array.from(
    document.querySelectorAll<HTMLElement>('div[data-display-contents="true"]'),
  )) {
    if (renderedText(d).trimStart().startsWith("Feed post")) add(d);
  }
  for (const c of Array.from(document.querySelectorAll<HTMLElement>(selStr("postContainer")))) {
    add(c);
  }
  return found;
}

/** Canonicalize any post URL/string to `/feed/update/urn:li:<type>:<id>/` — the form
 *  the post tab matches on (see {@link postContainerForUrl}). Accepts a full
 *  `urn:li:…` token, or a `/posts/…-<type>-<id>-<hash>/` slug (LinkedIn's "Copy link
 *  to post" form), PRESERVING the URN type. This is the fix for the invalid-link bug:
 *  the copied `/posts/` slug carries the id as `-share-<id>-` or `-ugcPost-<id>-`, and
 *  a share/ugcPost id is NOT an activity id — rebuilding it as `activity` produced a
 *  `/feed/update/urn:li:activity:<shareId>/` that resolves to nothing and can't be
 *  matched on the post tab. Only a bare id with no type context falls back to
 *  `activity` (the common in-DOM feed case). Empty when no id is present. */
export function canonicalPostPermalink(raw: string): string {
  const urn = raw.match(/urn:li:(?:activity|ugcPost|share):\d+/)?.[0];
  if (urn) return normalizePostUrl(permalinkFromUrn(urn));
  const typed = raw.match(/-(activity|ugcPost|share)-(\d{15,25})/);
  if (typed) return normalizePostUrl(permalinkFromUrn(`urn:li:${typed[1]}:${typed[2]}`));
  const id = raw.match(/(\d{15,25})/)?.[1];
  if (id) return normalizePostUrl(permalinkFromUrn(`urn:li:activity:${id}`));
  return "";
}

/** A post's permalink straight from the DOM — the PRIMARY, clipboard-free path.
 *  Prefers the post's activity URN (own/ancestor/descendant `data-urn`), which gives
 *  the real `urn:li:activity:<id>` that both opens correctly and matches the post's
 *  container when we go to comment. Falls back to any explicit `/feed/update/…` ·
 *  `/posts/…` link exposed in the block or its enclosing update container (the
 *  timestamp/permalink anchor). Empty only when the post exposes no link at all — a
 *  now-rare case, since the ancestor `data-urn` walk covers the common feed post; the
 *  clipboard ⋯-copy path is the last resort behind this. */
export function postPermalink(container: HTMLElement): string {
  const urn = postUrn(container);
  if (urn) return normalizePostUrl(permalinkFromUrn(urn));
  // Search the enclosing update container too — the timestamp/permalink anchor often
  // sits outside the "Feed post" a11y block we anchor on.
  const scope = container.closest<HTMLElement>(selStr("postContainer")) ?? container;
  const a = scope.querySelector<HTMLAnchorElement>(
    'a[href*="/feed/update/"], a[href*="/posts/"]',
  );
  return a ? canonicalPostPermalink(a.getAttribute("href") ?? "") : "";
}

/** A post's body text: the dedicated text node when the selector matches, else the
 *  block's own rendered text with the "Feed post" a11y prefix stripped — so a rotated
 *  text class still yields something to draft from. Whitespace-collapsed, capped. */
export function postBodyText(container: HTMLElement): string {
  const MAX = 3000;
  let text = (container.querySelector(selStr("postText"))?.textContent ?? "")
    .replace(/\s+/g, " ")
    .trim();
  if (!text) {
    text = renderedText(container)
      .replace(/^\s*Feed post\s*/, "")
      .replace(/\s+/g, " ")
      .trim();
  }
  return text.length > MAX ? text.slice(0, MAX) : text;
}

/** A post author's display name: the dedicated actor-name node when it matches, else
 *  the block's first profile (`/in/`) link. Doubled-name + lockup junk stripped. */
export function postAuthorName(container: HTMLElement): string {
  const raw =
    container.querySelector(selStr("postActorName"))?.textContent ||
    container.querySelector('a[href*="/in/"]')?.textContent ||
    "";
  return dedupeDoubled(cleanText(raw));
}

/** Whether a post block is a promoted/ad card — never worth commenting on, and it has
 *  no shareable permalink anyway. */
export function isPromotedBlock(container: HTMLElement): boolean {
  return /\bPromoted\b/.test(renderedText(container));
}

/** A stable-enough per-post key for the scrape's "seen" set: the block's leading
 *  rendered text (feed blocks recycle on scroll and carry no stable id, so we key on
 *  content, like the reference scraper does). Empty when the block has no text. */
export function postTextKey(container: HTMLElement): string {
  // A longer prefix than the original 160 chars: two DIFFERENT posts that share a
  // long lead-in (reshares of the same article, boilerplate intros) collided at 160
  // and the second was silently dropped from the scrape. 400 chars keeps the key
  // cheap while making that collision far less likely.
  return renderedText(container)
    .replace(/^\s*Feed post\s*/, "")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 400);
}

/**
 * The next post block not yet in `seen`, marked seen and scrolled into view — the
 * interactive scrape processes ONE post at a time (find → capture its link → next),
 * mirroring the reference scraper, because a post's permalink is only obtainable by
 * acting on that specific post (its ⋯ menu). Returns `null` when every currently
 * rendered block has been seen (the caller then scrolls to load more). Blocks with
 * no text are skipped without being marked (they're not posts).
 */
export function nextUnseenPost(seen: Set<string>): HTMLElement | null {
  for (const block of findPostContainers()) {
    const key = postTextKey(block);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    try {
      block.scrollIntoView({ block: "center" });
    } catch {
      // A detached/odd node — ignore; still return it to attempt capture.
    }
    return block;
  }
  return null;
}

/** Open a post's ⋯ control menu (which holds "Copy link to post"). Prefers a button
 *  whose aria-label reads as a control/options menu, falling back to the healable
 *  `postMenuButton` selector. Returns whether a button was found and clicked. */
export function openPostControlMenu(container: HTMLElement): boolean {
  try {
    const byAria = Array.from(container.querySelectorAll<HTMLButtonElement>("button")).find((b) => {
      const a = (b.getAttribute("aria-label") ?? "").toLowerCase();
      return /control menu|more actions|open options|more options/.test(a);
    });
    const btn = byAria ?? container.querySelector<HTMLButtonElement>(selStr("postMenuButton"));
    if (!btn) return false;
    btn.click();
    return true;
  } catch {
    return false;
  }
}

/** Click the "Copy link to post" item in the (now-open) control menu. LinkedIn
 *  renders the menu as a document-level popup, so we CAN'T scope to the post's
 *  container — but we MUST NOT match a stale/other menu left open from a prior post
 *  (which would copy the wrong post's link and pair it with this post's text). So we
 *  search only VISIBLE, currently-open dropdown popups (never the whole document),
 *  and among them the last one in DOM order — the most recently opened. Text match
 *  is locale-dependent, like the reference. Returns whether it was clicked. */
export function clickCopyLinkItem(): boolean {
  try {
    // Currently-open, visible dropdown popups only. `offsetParent === null` filters
    // out closed/hidden menus still in the DOM. Last = most recently opened.
    const openMenus = Array.from(
      document.querySelectorAll<HTMLElement>(
        '.artdeco-dropdown__content--is-open, [role="menu"], .artdeco-dropdown__content',
      ),
    ).filter((m) => m.offsetParent !== null);
    const roots: ParentNode[] = openMenus.length > 0 ? openMenus.reverse() : [document];
    for (const root of roots) {
      const items = Array.from(
        root.querySelectorAll<HTMLElement>('[role="menuitem"], div[role="button"], button, a'),
      );
      const el = items.find((e) =>
        /copy link to post|copy link/i.test((e.innerText || e.textContent || "").trim()),
      );
      if (el) {
        el.click();
        return true;
      }
    }
    return false;
  } catch {
    return false;
  }
}

/** Dismiss any open menu/popup so the next post's ⋯ click isn't blocked or, worse,
 *  its stale "Copy link" item matched. LinkedIn frequently ignores a synthetic
 *  Escape dispatched only at the document, so we also fire it at the active element
 *  and click the dropdown's own dismiss trigger / an empty area, and finally blur —
 *  layered because no single signal reliably closes an artdeco dropdown. */
export function closeOpenMenu(): void {
  try {
    const esc = (): KeyboardEvent =>
      new KeyboardEvent("keydown", { key: "Escape", code: "Escape", bubbles: true, cancelable: true });
    const active = document.activeElement as HTMLElement | null;
    active?.dispatchEvent(esc());
    document.dispatchEvent(esc());
    document.body?.dispatchEvent(esc());
    // An outside pointerdown is what actually dismisses an artdeco dropdown when
    // Escape is swallowed; the body is a safe, no-navigation target.
    document.body?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    active?.blur?.();
  } catch {
    // Never propagate into the host page.
  }
}

/** The comment composer's editable box within `scope`, or `null` when it isn't
 *  present/open yet. `scope` should be the target post's container (see
 *  {@link postContainerForUrl}) so a permalink page's recommended-post composers
 *  can't be mistaken for this post's; defaults to the whole document. */
export function findCommentEditor(scope: ParentNode = document): HTMLElement | null {
  return scope.querySelector<HTMLElement>(selStr("commentEditable"));
}

/** Reveal a post's comment composer by clicking its comment button (LinkedIn
 *  lazily mounts the editor only after the button is pressed). Scoped to `scope`
 *  (the target post's container) so it never opens a neighbouring post's composer.
 *  Fail-quiet — a missing button just means the editor may already be present, or
 *  isn't reachable, and the caller polls for {@link findCommentEditor} either way. */
export function openCommentComposer(scope: ParentNode = document): void {
  try {
    if (findCommentEditor(scope)) return;
    const btns = Array.from(scope.querySelectorAll<HTMLButtonElement>(selStr("commentButton")));
    // Both the "Comment" ACTION button and the "N comments" count toggle can match
    // the selector; prefer the one whose aria-label reads as the action, falling
    // back to the first match so a label change never leaves us unable to open it.
    const isAction = (b: HTMLButtonElement): boolean => {
      const a = (b.getAttribute("aria-label") ?? "").trim().toLowerCase();
      return a === "comment" || a.startsWith("comment on");
    };
    (btns.find(isAction) ?? btns[0])?.click();
  } catch {
    // Never propagate into the host page.
  }
}

/** Write `text` into the (already-open) comment composer's editable box within
 *  `scope`. Returns whether it was written (false when no comment editor is
 *  present). Same active-document / focus rules as {@link writeComposer}; never
 *  submits. */
export function writeCommentBox(text: string, scope: ParentNode = document): boolean {
  const editable = findCommentEditor(scope);
  if (!editable) return false;
  return writeIntoEditable(editable, text);
}

/** Whether the comment composer within `scope` already holds user-typed text — so
 *  a fill never clobbers a comment in progress. */
export function commentBoxHasText(scope: ParentNode = document): boolean {
  return hasVisibleText(findCommentEditor(scope));
}

/** Normalize comment text for a resilient, truncation-tolerant match: collapse
 *  whitespace, drop the trailing "…more" / "…see more" fold LinkedIn appends when it
 *  clips a long comment, and lowercase. */
function normalizeCommentText(s: string): string {
  return s
    .replace(/\s+/g, " ")
    .replace(/(?:…|\.\.\.)?\s*see more\s*$/i, "")
    .replace(/(?:…|\.\.\.)\s*more?\s*$/i, "")
    .trim()
    .toLowerCase();
}

/** Length of the leading slice compared when matching a comment. Long enough that
 *  an unrelated comment won't collide, short enough to survive LinkedIn's "…more"
 *  truncation of a long comment. */
const COMMENT_MATCH_PREFIX = 80;
/** Below this, a comment is too short to match without risking a false positive
 *  against some other brief comment — skip the check rather than guess. */
const COMMENT_MATCH_MIN = 12;

/**
 * Whether a comment matching `text` is ALREADY posted in `scope`'s comment thread —
 * the mechanical idempotency guard that stops a duplicate PUBLIC comment. This is
 * the reliable source of truth for "did it post": LinkedIn's own rendered thread,
 * not a timing heuristic. Used both before posting (a prior attempt may have posted
 * but had its confirmation time out and been recorded "failed") and to confirm a
 * submit landed. Matches on a distinctive leading slice (so a truncated long comment
 * still matches) and requires a long-enough prefix that an unrelated comment can't
 * collide. `scope` must be the target post's OWN container so a neighbouring post's
 * comments never count. Fail-quiet — a DOM/selector miss returns false (never a
 * false "already posted" that would suppress a real post).
 */
export function commentAlreadyPresent(scope: ParentNode, text: string): boolean {
  try {
    const needle = normalizeCommentText(text);
    if (needle.length < COMMENT_MATCH_MIN) return false;
    const key = needle.slice(0, COMMENT_MATCH_PREFIX);
    for (const el of Array.from(scope.querySelectorAll<HTMLElement>(selStr("commentItemBody")))) {
      const hay = normalizeCommentText(renderedText(el));
      if (hay.length >= COMMENT_MATCH_MIN && (hay.startsWith(key) || hay.includes(key))) {
        return true;
      }
    }
    return false;
  } catch {
    return false;
  }
}

/** Submit the (already-filled) comment in `scope`'s composer by clicking its
 *  submit button — the AUTO-POST step. Returns whether a submit control was found,
 *  enabled, and clicked; `false` when the button is missing or still disabled
 *  (LinkedIn disables it until the box holds text, so the caller retries after the
 *  write has registered). Scoped to the target post's container so a permalink
 *  page's recommended-post composers are never submitted. Fail-quiet. */
export function submitComment(scope: ParentNode = document): boolean {
  try {
    const btn = (scope instanceof Element ? scope : document).querySelector<HTMLButtonElement>(
      selStr("commentSubmit"),
    );
    if (!btn || btn.disabled) return false;
    btn.click();
    return true;
  } catch {
    return false;
  }
}

// ── Message scraping ─────────────────────────────────────────────────────────
// The other fragile surface (see the file header). We read the messages already
// rendered in the thread and classify each as sent-by-YOU or a reply. Everything
// here is best-effort and must never throw into the page.

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
  const profileLink = selStr("profileLink");
  const link =
    item.querySelector<HTMLAnchorElement>(profileLink) ??
    item.closest(selStr("messageGroup"))?.querySelector<HTMLAnchorElement>(profileLink) ??
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
  const otherModifier = selStr("messageOtherModifier");
  const bodySel = selStr("messageBody");
  for (const item of Array.from(scope.querySelectorAll<HTMLElement>(selStr("messageItem")))) {
    const sender = senderUrl(item);
    const isIncoming =
      item.classList.contains(otherModifier) || (sender != null && sender === partnerUrl);
    const direction: MessageDirection = isIncoming ? "incoming" : "outgoing";

    const body = (item.querySelector(bodySel)?.textContent ?? "")
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
