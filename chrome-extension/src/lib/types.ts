// Shapes shared between the content script and the service worker. These mirror
// the Rust side (features/pitches, features/prospects) — keep them in sync.

/** A pitch, as returned by the app's `GET /pitches`. */
export interface Pitch {
  id: number;
  name: string;
  skill: string;
  created_at: string;
}

/** The payload the content script captures and POSTs to `/prospects`. */
export interface NewProspect {
  name: string;
  linkedin_url: string;
  headline?: string;
  pitch_id?: number | null;
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
    pitch_id: number | null;
    note: string;
    created_at: string;
  };
}

/** Who sent a captured message. `outgoing` = you messaged the prospect (drives
 *  the messages-sent count); `incoming` = the prospect replied (drives their
 *  durable "responded" state). */
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

/** Ask the SW to open `count` background tabs on the messaging inbox, each tagged
 *  (via a URL hash) with the chosen pitch and its target conversation index, so
 *  each tab opens that conversation itself and drafts a reply. LinkedIn's list
 *  rows have no thread URL to hand off, so the tab identifies its conversation by
 *  list position rather than by URL. `start` is the list index of the currently-
 *  selected conversation: the batch drafts for it and the `count - 1` below it, so
 *  the tabs cover absolute indices `start … start + count - 1`. `filter` is the
 *  active inbox filter pill token (e.g. `"UNREAD"`) to re-apply in each tab so its
 *  list positions match the filtered view, or `null` for the default inbox. */
export interface OpenThreadsPayload {
  pitchId: number;
  count: number;
  start: number;
  filter: string | null;
}

/** One prior message in a thread, as the drafter scrapes it — the minimal shape
 *  the `/draft` endpoint needs (direction + body; no dedup key). */
export interface DraftMessageInput {
  direction: MessageDirection;
  body: string;
}

/** What the content script sends the SW to draft one reply. The conversation is
 *  scraped live from the open thread; `pitch_id` selects the snippet library. */
export interface DraftReplyPayload {
  prospect_name: string;
  pitch_id: number;
  messages: DraftMessageInput[];
}

/** `POST /draft` response: the composed reply, or an ALL-CAPS reason it couldn't
 *  be built. Written verbatim into the thread's compose box either way. */
export interface DraftResult {
  draft: string;
}

/** What became of an attempted capture, reported back so the content script can
 *  give immediate feedback:
 *   - `stored`  — recorded against a tracked prospect (outgoing count bumped, or
 *                 an incoming reply marked them responded).
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
  | { type: "listPitches" }
  | { type: "addProspect"; payload: NewProspect }
  | { type: "queueMessages"; payload: QueueMessagesPayload }
  | { type: "openThreads"; payload: OpenThreadsPayload }
  | { type: "draftReply"; payload: DraftReplyPayload }
  | { type: "draftSlotFree" };

export type Response<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };
