// Shapes shared between the content script and the service worker. These mirror
// the Rust side (features/customers, features/prospects) — keep them in sync.

/** A customer profile (ICP), as returned by the app's `GET /customers`.
 *
 *  Only the fields the extension actually shows. The app returns the profile's
 *  steering text too (who they are / their pain / the goal), but that's for the
 *  draft prompt to read server-side — the extension never needs it, so it isn't
 *  mirrored here. */
export interface Customer {
  id: number;
  name: string;
}

/** The payload the content script captures and POSTs to `/prospects`. */
export interface NewProspect {
  name: string;
  linkedin_url: string;
  headline?: string;
  customer_id?: number | null;
  note?: string;
}

/** `POST /prospects` response: the saved row + whether it already existed. */
export interface AddProspectResult {
  existed: boolean;
  prospect: {
    id: number;
    name: string;
    linkedin_url: string;
    headline: string;
    customer_id: number | null;
    note: string;
    created_at: string;
  };
}

/** `GET /prospect` response: whether the open thread's person is already a
 *  prospect, and which customer profile they match. `customer_id` is null when
 *  they're unassigned — captured without a profile, or their profile was deleted
 *  since (which leaves the prospect in the pipeline, just unassigned). That's a
 *  valid state, not an error: their drafts simply get no goal to steer toward.
 *  The extension resolves the *name* from the customer list it already fetched
 *  (no name is sent here). */
export interface ProspectLookup {
  exists: boolean;
  customer_id: number | null;
}

/** Who sent a captured message. `outgoing` = you messaged the prospect (drives
 *  the messages-sent count); `incoming` = the prospect replied. The newest
 *  message's direction drives their "awaiting reply" state. */
export type MessageDirection = "outgoing" | "incoming";

/** One message scraped from a LinkedIn thread. `li_key` is a stable per-message
 *  identity (a DOM id/urn, or a content hash) — the dedup key. */
export interface CapturedMessage {
  li_key: string;
  body: string;
  sent_at: string | null;
  direction: MessageDirection;
}

/** A person's captured messages, as the content script hands them to the SW. */
export interface QueueMessagesPayload {
  linkedin_url: string;
  messages: CapturedMessage[];
}

/** Ask the SW to open one conversation as a pre-filled "review" tab. The main
 *  inbox tab has already generated this thread's draft and cached it (keyed by
 *  `url`); the SW navigates a background tab straight to `url`, where the content
 *  script reads the cached draft and pastes it. `url` is the normalized thread URL
 *  (`https://www.linkedin.com/messaging/thread/<id>/`) captured during the cycle. */
export interface OpenReviewTabPayload {
  url: string;
}

/** One prior message in a thread, as the drafter scrapes it — the minimal shape
 *  the `/draft` endpoint needs (direction + body; no dedup key). */
export interface DraftMessageInput {
  direction: MessageDirection;
  body: string;
}

/** What the content script sends the SW to draft one reply. The conversation is
 *  scraped live from the open thread; `customer_id` is the profile to steer the
 *  reply toward, and may be null — an unmatched prospect still gets a draft, just
 *  one composed from the product and snippets with no goal to aim at.
 *
 *  `linkedin_url` is the person's PROFILE url (`/in/<slug>/`, canonical form).
 *  The app uses it for exactly one lookup: which cycle stage this prospect sits
 *  in, so the reply can aim at that step's goal rather than the whole
 *  relationship's. Send "" for a thread whose person couldn't be resolved — the
 *  app drops the stage block and drafts as before. */
export interface DraftReplyPayload {
  prospect_name: string;
  customer_id: number | null;
  linkedin_url: string;
  messages: DraftMessageInput[];
}

/** `POST /draft` response: the composed reply, or an ALL-CAPS reason it couldn't
 *  be built. Written verbatim into the thread's compose box either way.
 *
 *  `blocked` marks the one refusal that isn't about this thread: the app has no
 *  product description, no profile, and no snippets, so NOTHING can be drafted
 *  until the user sets it up. Every conversation in a batch would come back the
 *  same, so the runner stops on the first one instead of writing the refusal into
 *  every open composer. Absent on older app builds — treat as false. */
export interface DraftResult {
  draft: string;
  blocked?: boolean;
}

// ── The LinkedIn commenter ───────────────────────────────────────────────────

/** A watched profile, as returned by the app's `GET /watched-profiles` — a
 *  LinkedIn profile a comment run visits (before the feed) to check for new
 *  posts. Mirrors the Rust `WatchedProfile`. */
export interface WatchedProfile {
  id: number;
  linkedin_url: string;
  name: string;
  created_at: string;
}

/** One post scraped from the feed or a profile's recent-activity — the material a
 *  comment run drafts from. `permalink` is the post's canonical URL (the per-post
 *  dedup key and where a review tab opens to place the comment). */
export interface ScrapedPost {
  permalink: string;
  author_name: string;
  text: string;
}

/** A `{key: value}` map of LinkedIn-selector overrides (value = a CSS string or an
 *  ordered fallback list). `GET /selectors` returns these; a heal response carries
 *  the merged set. The content script merges them over its compiled defaults. */
export type SelectorOverrides = Record<string, string | string[]>;

/** One broken selector the content script reports to `POST /heal-selectors`:
 *  the registry key, what it should find, and its current (broken) value. */
export interface BrokenSelectorInput {
  key: string;
  description: string;
  current: string;
}

/** What the content script sends the SW to repair stale selectors: the live page
 *  HTML, the current URL (context), and the broken selectors. */
export interface HealSelectorsPayload {
  html: string;
  url: string;
  broken: BrokenSelectorInput[];
}

/** What became of an attempted capture, reported back so the content script can
 *  give immediate feedback:
 *   - `stored`  — recorded against a tracked prospect (outgoing count bumped, or
 *                 an incoming reply set them awaiting a reply).
 *   - `skipped` — accepted but not a tracked prospect (normal; no toast).
 *   - `offline` — the app couldn't be reached; the message stays queued in the
 *                 durable outbox and syncs on the next drain.
 *  `stored`/`skipped` are the backend's batch counts for this attempt. */
export interface CaptureOutcome {
  outcome: "stored" | "skipped" | "offline";
  stored: number;
  skipped: number;
}

/** A queued row in the SW's write-through outbox / the `POST /messages` body.
 *  Self-contained (carries its own `linkedin_url`) so the backend resolves the
 *  prospect at delivery time — replay-safe, since the backend dedups on
 *  `(prospect, li_key)`. */
export interface OutboxItem extends CapturedMessage {
  linkedin_url: string;
}

// Messages between the content script (sender) and service worker (handler).
// All cross-origin fetches happen in the SW — MV3 forbids them in content scripts.

export type Request =
  | { type: "checkin" }
  | { type: "listCustomers" }
  | { type: "lookupProspect"; payload: { linkedin_url: string } }
  | { type: "addProspect"; payload: NewProspect }
  | { type: "queueMessages"; payload: QueueMessagesPayload }
  | { type: "draftReply"; payload: DraftReplyPayload }
  | { type: "resetReviewQueue"; payload: { hash: string } }
  | { type: "openReviewTab"; payload: OpenReviewTabPayload }
  | { type: "reviewTabFilled" }
  | { type: "getSelectors" }
  | { type: "healSelectors"; payload: HealSelectorsPayload }
  // A LinkedIn tab nudging the SW to check the app for pending comment work (a
  // requested scrape or queued posts) — the app can't push to the extension, so
  // this makes "Scrape" / "Post all" start promptly. Fire-and-forget.
  | { type: "pollCommentWork" }
  // Worker-tab → SW (internal): a scrape tab reporting the posts it scraped, and a
  // post tab reporting whether it auto-submitted its comment. The SW correlates
  // each to the pending job by the sender tab's id, then closes the tab.
  | { type: "postsScraped"; payload: { posts: ScrapedPost[] } }
  | { type: "commentPosted"; payload: { status: "posted" | "failed"; error: string } };

export type Response<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };
