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
  | { type: "listPitches" }
  | { type: "addProspect"; payload: NewProspect }
  | { type: "queueMessages"; payload: QueueMessagesPayload }
  | { type: "draftReply"; payload: DraftReplyPayload }
  | { type: "resetReviewQueue" }
  | { type: "openReviewTab"; payload: OpenReviewTabPayload }
  | { type: "reviewTabFilled" };

export type Response<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };
