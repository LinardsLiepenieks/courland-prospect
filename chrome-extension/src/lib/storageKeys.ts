/** `chrome.storage.local` keys shared across content surfaces. Centralized so
 *  the capture widget and the "Draft for" control can't drift apart on the key
 *  and silently stop sharing the user's last-picked pitch. */
export const LAST_PITCH_KEY = "lastPitchId";

/** URL-hash marker the review-tab queue appends to a thread URL, and the content
 *  script keys off to know a tab was opened to be pre-filled with a cached draft
 *  (vs. a thread the user opened by hand). Shared so the SW that builds the URL
 *  and the content script that detects it can't drift apart. */
export const FILL_HASH = "cpfill";

/** URL-hash marker for a POST tab the service worker opens on a post's permalink
 *  to AUTO-POST an approved comment: the content script reads the cached comment,
 *  writes it into the post's comment box, and submits (vs. `FILL_HASH`, which
 *  pre-fills a message reply and never sends). Carries the canonical permalink
 *  (`#cppost=<encoded permalink>`) — the key the comment is cached under. */
export const POST_COMMENT_HASH = "cppost";

/** URL-hash marker for a background worker tab the service worker opens on the
 *  feed home or a watched profile's recent-activity page: the content script
 *  scrapes that page's posts and reports them back, then the SW closes the tab. */
export const SCRAPE_HASH = "cpscrape";

/** Draft-store namespace for the message-reply batch feature (draft.ts). Keeps its
 *  cached drafts in a separate keyspace from the comment feature so one feature's
 *  batch-start clear can't wipe the other's still-pending drafts. */
export const DRAFT_NS_REPLY = "reply";

/** Draft-store namespace for the comment the service worker hands to a post tab to
 *  auto-submit (keyed by the post's permalink). Separate from the reply namespace
 *  so the two never share a keyspace. */
export const DRAFT_NS_POST = "post";
