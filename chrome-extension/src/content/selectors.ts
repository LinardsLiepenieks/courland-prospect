// THE single place every LinkedIn DOM selector lives — the self-heal registry.
//
// LinkedIn ships obfuscated, frequently-rotated class names, so its selectors go
// stale every few weeks. Instead of hardcoding them across linkedin.ts, they live
// here as named keys. Two layers:
//   1. DEFAULTS — compiled into the extension (the current known-good values).
//   2. overrides — fetched from the Courland app at startup (`GET /selectors`) and
//      merged over the defaults. When a selector breaks, `heal.ts` sends the live
//      page to the app, Claude Code derives a new value, the app persists it, and
//      the extension applies it live (see `applyOverrides`) — no rebuild needed.
//
// Read selectors through `selStr(key)` / `selList(key)` at call time so an override
// applied after startup takes effect immediately. The layered *heuristics* (fallback
// order, text/aria matching, DOM walks) stay in linkedin.ts — only the selector
// strings themselves are centralized and healed.

/** A selector value: a CSS string (may be a comma-group), or an ordered list of
 *  fallback selectors tried in turn. */
export type SelectorValue = string | string[];

/** Compiled-in defaults. Keep values as plain CSS the browser accepts; prefer the
 *  same stable-hook strategy the heal prompt asks Claude to follow. */
export const DEFAULTS = {
  /** The message-compose form root (full pane + each overlay bubble). */
  composeRoot: '[class~="msg-form"]',
  /** Known send-button classes (a comma-group: match any). */
  sendButtonClasses: ".msg-form__send-button, .msg-form__send-btn, .msg-form__send-toggle",
  /** The compose form's submit button. */
  sendSubmit: 'button[type="submit"]',
  /** The compose form's footer/toolbar row (where Send lives). */
  composeFooter: '[class~="msg-form__footer"]',
  /** The compose text editor. */
  composeEditable: '[contenteditable="true"], [role="textbox"]',
  /** Explicit conversation containers, tightest attribution scope for a thread. */
  convoExplicit:
    '.msg-overlay-conversation-bubble, [class*="msg-overlay-conversation"], .msg-convo-wrapper, .msg-thread',
  /** The rendered message stream — marks that a thread is actually open. */
  messageList: '[class*="msg-s-message-list"]',
  /** Profile links in the THREAD HEADER, tried in order to identify the person. */
  identityHeaders: [
    "a.msg-thread__link-to-profile",
    "a.msg-connection-card__link",
    '[class*="msg-title-bar"] a[href*="/in/"]',
    '[class*="msg-entity-lockup"] a[href*="/in/"]',
    '[class*="overlay-conversation"] a[href*="/in/"]',
  ],
  /** Any LinkedIn profile link. */
  profileLink: 'a[href*="/in/"]',
  /** Containers whose links must NEVER be read as the open thread's identity
   *  (the sidebar list + the message stream). A comma-group. */
  nonIdentity:
    '[class*="msg-conversations-container"], [class*="conversation-listitem"], [class*="msg-overlay-list-bubble"], [class*="msg-s-message-list"], [class*="msg-s-event-listitem"]',
  /** The header region enclosing a resolved profile link (for the display name). */
  nameHeaderContainer:
    '[class*="msg-title-bar"], [class*="msg-entity-lockup"], [class*="msg-thread"], [class*="overlay-conversation"]',
  /** A dedicated title node within the header (preferred over the lockup blob). */
  nameTitleNode: 'h2, [class*="entity-title"], [class*="title-bar__title"], [class*="thread__title"]',
  /** One message row in the thread. */
  messageItem: ".msg-s-event-listitem",
  /** Class token LinkedIn puts on the COUNTERPARTY's message rows. */
  messageOtherModifier: "msg-s-event-listitem--other",
  /** A message row's body text. */
  messageBody: ".msg-s-event-listitem__body",
  /** The message-group / list-event ancestor holding a row's sender link. */
  messageGroup: '.msg-s-message-group, li[class*="msg-s-message-list__event"]',
  /** The messaging inbox top bar (primary: data hook). */
  inboxTopBarPrimary: "[data-test-msg-cross-pillar-inbox-top-bar-wrapper]",
  /** The inbox top bar (fallback: class substring). */
  inboxTopBarFallback: '[class*="inbox-top-bar-wrapper__container"]',
  /** The inbox filter row (Inbox / Jobs / Unread / …). */
  inboxFilterRow: '[class*="msg-conversations-container__title-row"]',
  /** A conversation row's clickable element in the thread list. */
  threadRow: '[class*="msg-conversation-listitem__link"]',
  /** Class token marking the conversation row currently open in the reading pane. */
  activeRowToken: "convo-item-link--active",

  // ── Posts & comments (the LinkedIn commenter) ──────────────────────────────
  /** A single post/update container in the feed or on a profile's recent-activity
   *  page. Anchored on the update's activity URN attribute (stable) with a
   *  class-based fallback, so the post's permalink can be derived from the URN. */
  postContainer:
    '[data-urn^="urn:li:activity"], [data-id^="urn:li:activity"], div.feed-shared-update-v2, div.fie-impression-container',
  /** A post's main text body. */
  postText:
    '.update-components-text, .feed-shared-update-v2__description, [class*="update-components-text"]',
  /** A post author's display name. */
  postActorName:
    '.update-components-actor__name, .update-components-actor__title span[dir="ltr"], .update-components-actor__title',
  /** A post's ⋯ "open control menu" button (opens the menu holding "Copy link to
   *  post"). Matched by its aria-label; the scrape falls back to a text/aria scan. */
  postMenuButton:
    'button[aria-label*="control menu" i], button[aria-label*="more actions" i], button[aria-label*="open options" i]',
  /** The button that opens/focuses a post's comment composer. */
  commentButton:
    'button[aria-label*="comment" i], button.comment-button, button[aria-label*="Comment" i]',
  /** The comment composer's editable text box (LinkedIn's ProseMirror editor). */
  commentEditable:
    '.comments-comment-box div.ProseMirror[contenteditable="true"], .comments-comment-texteditor [contenteditable="true"], div.ql-editor[contenteditable="true"], form.comments-comment-box__form [contenteditable="true"]',
  /** The comment composer's submit ("Comment"/"Post") button — used to detect that
   *  the user actually posted a drafted comment (vs. just having it placed). */
  commentSubmit:
    '.comments-comment-box__submit-button, button.comments-comment-box__submit-button--cr, form.comments-comment-box__form button[type="submit"], .comments-comment-texteditor button[type="submit"]',
  /** The text body of a comment ALREADY posted in a post's comment thread — read to
   *  detect that a comment we're about to place (or just submitted) is already there,
   *  so we never post the same comment twice. Comment-scoped (never the post's own
   *  body) so a post's text can't be mistaken for an existing comment. */
  commentItemBody:
    '.comments-comment-item__main-content, .comments-comment-entity__content, article.comments-comment-entity .update-components-text, .comments-comment-item .update-components-text, [class*="comments-comment-item"] [class*="comments-comment-item__main-content"], [class*="comment-entity"] [class*="update-components-text"]',
} satisfies Record<string, SelectorValue>;

export type SelectorKey = keyof typeof DEFAULTS;

/** One-line "what this finds", sent to Claude Code when healing so it knows which
 *  element each broken key should match. Keep in sync with DEFAULTS' keys. */
export const DESCRIPTIONS: Record<SelectorKey, string> = {
  composeRoot: "the message compose form root (the box where you type a message)",
  sendButtonClasses: "the Send button of the message composer",
  sendSubmit: "the submit button of the message composer",
  composeFooter: "the composer's footer/toolbar row that contains the Send button",
  composeEditable: "the editable text field of the message composer",
  convoExplicit: "the container element wrapping a single open conversation/thread",
  messageList: "the scrollable list of messages in an open conversation",
  identityHeaders:
    "profile links (to /in/…) in the conversation header identifying who the thread is with",
  profileLink: "a link to a LinkedIn member profile (href contains /in/)",
  nonIdentity:
    "containers to EXCLUDE from identity: the sidebar conversation list and the message stream",
  nameHeaderContainer: "the conversation header region enclosing the person's name/title",
  nameTitleNode: "the element holding the conversation partner's display name/title",
  messageItem: "a single message row in the conversation",
  messageOtherModifier:
    "the CSS class (bare token, no dot) LinkedIn adds to the OTHER person's message rows",
  messageBody: "the text body element inside a message row",
  messageGroup: "the group/list-item ancestor of a message row that holds the sender's link",
  inboxTopBarPrimary: "the messaging inbox top bar (title/search/compose row)",
  inboxTopBarFallback: "the messaging inbox top bar (fallback)",
  inboxFilterRow: "the messaging inbox filter row (Inbox / Jobs / Unread tabs)",
  threadRow: "a clickable conversation row in the messaging thread list",
  activeRowToken:
    "the CSS class (bare token, no dot) marking the thread-list row currently open",
  postContainer:
    "a single post/update container in the feed or on a profile's recent-activity page — PREFER the element carrying the post's activity URN (an attribute like data-urn or data-id whose value is 'urn:li:activity:<digits>', or 'urn:li:ugcPost:'/'urn:li:share:'), so the post's permalink can be derived from it",
  postText: "the main text body of a post/update",
  postActorName: "the display name of the person or page that authored a post",
  postMenuButton:
    "a post's ⋯ 'open control menu' button (the menu that contains 'Copy link to post')",
  commentButton: "the button that opens or focuses the comment composer under a post",
  commentEditable: "the editable text box of a post's comment composer (where you type a comment)",
  commentSubmit: "the submit button that posts a comment in the comment composer (labelled Comment or Post)",
  commentItemBody:
    "the text body of a comment that has ALREADY been posted under a post (a comment in the post's comment thread, NOT the post's own body text and NOT the composer where you type)",
};

// Live values = defaults with any fetched/healed overrides merged on top.
const current: Record<SelectorKey, SelectorValue> = { ...DEFAULTS };

/** Keys consumed as bare class TOKENS (via `classList.contains` / `className.includes`),
 *  not as CSS selectors — so a healed value is validated as a plain token, never parsed
 *  as CSS (and a dotted/selector form is rejected, which would silently mis-match). */
const TOKEN_KEYS = new Set<SelectorKey>(["messageOtherModifier", "activeRowToken"]);
const TOKEN_RE = /^[\w-]+$/;

/** Whether `s` is syntactically valid CSS. The browser is the oracle: a throwaway
 *  fragment shares the exact selector parser the real `querySelector`/`matches`/
 *  `closest` call sites use, so this accepts exactly what they accept. */
function isValidCss(s: string): boolean {
  try {
    document.createDocumentFragment().querySelector(s);
    return true;
  } catch {
    return false;
  }
}

/** Validate one candidate string for `key`: a non-empty bare token for token keys,
 *  else a syntactically valid CSS selector. This is what stops a bad healed value
 *  (invalid CSS, or a dotted value for a token key) from being applied — which would
 *  otherwise throw in `querySelector` and, since that throw pre-empts the heal
 *  trigger, brick the key with no recovery. */
function isUsableString(key: SelectorKey, s: unknown): s is string {
  if (typeof s !== "string" || s.trim().length === 0) return false;
  return TOKEN_KEYS.has(key) ? TOKEN_RE.test(s) : isValidCss(s);
}

/** True if `value` is usable for `key`: a valid single value, or a non-empty array of
 *  valid values. Guards network/Claude-supplied overrides. */
function isUsableValue(key: SelectorKey, value: unknown): value is SelectorValue {
  if (Array.isArray(value)) {
    return value.length > 0 && value.every((s) => isUsableString(key, s));
  }
  return isUsableString(key, value);
}

/** A value that arrived as a JSON-encoded array string, coerced back into an
 *  array. A model asked to "return the same shape" for a list-valued key sometimes
 *  echoes the serialized string form it was shown (see {@link currentSerialized})
 *  rather than a real array; unwrap that so it validates as a list instead of being
 *  rejected as invalid CSS. A genuine CSS attribute selector like `[class~="x"]`
 *  starts with `[` too but is not valid JSON, so `JSON.parse` throws and it's left
 *  untouched. */
function coerceMaybeJsonArray(value: unknown): unknown {
  if (typeof value !== "string" || !value.trim().startsWith("[")) return value;
  try {
    const parsed = JSON.parse(value.trim());
    if (Array.isArray(parsed) && parsed.length > 0 && parsed.every((s) => typeof s === "string")) {
      return parsed;
    }
  } catch {
    // Not JSON (e.g. an attribute selector) — fall through and treat as a string.
  }
  return value;
}

/** Merge overrides (from `GET /selectors` or a heal response) over the defaults.
 *  Ignores unknown keys and values that are empty, the wrong shape, or not valid
 *  CSS / a bare token — so a bad payload can never blank out or brick a working
 *  selector (the compiled default just stays in place). Returns the keys actually
 *  applied — so a heal can tell which of the keys it asked about were really fixed
 *  (and mark only those as done), rather than treating "≥1 applied" as "all fixed". */
export function applyOverrides(overrides: unknown): SelectorKey[] {
  if (!overrides || typeof overrides !== "object") return [];
  const applied: SelectorKey[] = [];
  for (const [key, raw] of Object.entries(overrides)) {
    const value = coerceMaybeJsonArray(raw);
    if (key in DEFAULTS && isUsableValue(key as SelectorKey, value)) {
      current[key as SelectorKey] = value;
      applied.push(key as SelectorKey);
    }
  }
  return applied;
}

/** The live value as a single CSS string. For list-valued keys, joins the fallbacks
 *  into one comma-group (fine for a plain `querySelector`, where "match any" is the
 *  intent); use {@link selList} when the fallbacks must be tried in order. */
export function selStr(key: SelectorKey): string {
  const v = current[key];
  return Array.isArray(v) ? v.join(", ") : v;
}

/** The live value as an ordered list of selectors (a single string becomes a
 *  one-element list). */
export function selList(key: SelectorKey): string[] {
  const v = current[key];
  return Array.isArray(v) ? v : [v];
}

/** The current value serialized for the heal request's `current` field — a plain
 *  string, or JSON for a list (so Claude returns the same shape). */
export function currentSerialized(key: SelectorKey): string {
  const v = current[key];
  return Array.isArray(v) ? JSON.stringify(v) : v;
}
