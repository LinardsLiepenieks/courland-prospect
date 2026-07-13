// A URL-keyed cache of pre-generated drafts, shared across the extension's tabs
// via chrome.storage.local. The main inbox tab (which cycles the conversations
// and runs generation) WRITES a draft here the moment it's ready; the review tab
// the service worker then opens on that conversation READS it and pastes it into
// the composer. chrome.storage.local is the only store both tabs can see, so it
// is the hand-off channel between "generated here" and "pasted there".
//
// Entries are per-URL keys (not one shared object) so the several drafts a cycle
// resolves in parallel each write independently — no read-modify-write race on a
// single blob. Each carries a timestamp; a stale entry is ignored on read and the
// whole set is cleared at the start of every new batch, so a re-run never pastes a
// draft left over from a previous cycle.

/** Key prefix for every cached draft. The normalized thread URL is appended. */
const PREFIX = "cpdraft:";

/** How long a cached draft stays valid. A backstop against orphans (a review tab
 *  that died before consuming its entry); the batch-start clear is the primary
 *  reset. Comfortably longer than any batch takes to drain. */
const TTL_MS = 60 * 60 * 1000; // 1 hour

interface DraftEntry {
  draft: string;
  /** When this draft was stored (epoch ms), for the TTL check. */
  ts: number;
}

function keyFor(url: string): string {
  return PREFIX + url;
}

/** Store a freshly-generated draft for `url`. Overwrites any prior entry for the
 *  same conversation (a fresh cycle's draft supersedes a stale one). */
export async function putDraft(url: string, draft: string): Promise<void> {
  await chrome.storage.local.set({ [keyFor(url)]: { draft, ts: Date.now() } }).catch(() => {});
}

/** Read the cached draft for `url`, or `null` when there is none or it has aged
 *  past the TTL. A non-destructive read (the caller removes it after pasting via
 *  `removeDraft`); a timed-out entry is the one exception — it's swept as a side
 *  effect so the store doesn't accumulate orphans. */
export async function peekDraft(url: string): Promise<string | null> {
  const key = keyFor(url);
  const stored = await chrome.storage.local
    .get(key)
    .catch(() => ({}) as Record<string, unknown>);
  const entry = stored[key] as DraftEntry | undefined;
  if (!entry) return null;
  if (Date.now() - entry.ts > TTL_MS) {
    await removeDraft(url);
    return null;
  }
  return entry.draft;
}

/** Drop the cached draft for `url` — called once a review tab has consumed it, so
 *  reopening the conversation doesn't re-paste over a reply in progress. */
export async function removeDraft(url: string): Promise<void> {
  await chrome.storage.local.remove(keyFor(url)).catch(() => {});
}

/** Clear every cached draft. Run at the start of a new batch so a fresh "Draft
 *  for" run never surfaces drafts left over from the last one. */
export async function clearDrafts(): Promise<void> {
  const all = await chrome.storage.local.get(null).catch(() => ({}) as Record<string, unknown>);
  const keys = Object.keys(all).filter((k) => k.startsWith(PREFIX));
  if (keys.length > 0) await chrome.storage.local.remove(keys).catch(() => {});
}
