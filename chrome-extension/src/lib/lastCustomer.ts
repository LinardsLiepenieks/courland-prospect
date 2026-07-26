/** The last customer profile the user picked, shared by the two content surfaces
 *  that offer the choice (the capture widget and the "Draft for" control) so a
 *  pick on one pre-selects on the other.
 *
 *  Stored as `{ id, name }` rather than a bare id because `customers.id` is a
 *  plain SQLite `INTEGER PRIMARY KEY`, so rowids are RECYCLED: delete profile 2
 *  and the next profile created takes the same id. An id-only check can't tell
 *  the two apart and would silently pre-select a stranger's ICP — quiet wrong
 *  steering on real drafts, which is worse than no pre-selection at all. Matching
 *  the name too makes a recycled id simply miss and fall back to no pick.
 *
 *  Both helpers swallow storage rejections: a torn-down extension context must
 *  degrade to "no pre-selection", never reject unhandled into the page. */

import { LAST_CUSTOMER_KEY, LEGACY_LAST_PITCH_KEY } from "./storageKeys";
import type { Customer } from "./types";

type Remembered = { id: number; name: string };

function isRemembered(v: unknown): v is Remembered {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as Remembered).id === "number" &&
    typeof (v as Remembered).name === "string"
  );
}

/** Remember a real pick. `null` (the "No profile" option) is deliberately NOT
 *  persisted: it's a per-use choice, not a preference, and writing it would
 *  clear the id the other surface pre-selects from. */
export function rememberCustomer(customer: Customer | null): void {
  if (!customer) return;
  void chrome.storage.local
    .set({ [LAST_CUSTOMER_KEY]: { id: customer.id, name: customer.name } })
    .catch(() => {});
}

/** The remembered customer's id, but only if a profile with that id AND name is
 *  still in `customers`. Returns `null` for "no pre-selection" in every other
 *  case — nothing stored, a deleted profile, or a recycled id now pointing at
 *  someone else. Migrates the pre-rework key on first read (see below). */
export async function recallCustomerId(customers: Customer[]): Promise<number | null> {
  const stored = await chrome.storage.local
    .get([LAST_CUSTOMER_KEY, LEGACY_LAST_PITCH_KEY])
    .catch(() => ({}) as Record<string, unknown>);

  const last = stored[LAST_CUSTOMER_KEY];
  if (isRemembered(last)) {
    return customers.some((c) => c.id === last.id && c.name === last.name) ? last.id : null;
  }

  // Nothing under the current key — fall back to the pre-rework `lastPitchId`
  // once. Migration 0023 rebuilt each pitch as a customer profile keeping its id,
  // so the stored number still names the same row. It carries no name to match
  // on, so it gets the weaker id-only check; that's acceptable here because it
  // resolves at most once per install, and is then rewritten in the richer shape
  // by the first real pick.
  const legacy = stored[LEGACY_LAST_PITCH_KEY];
  void chrome.storage.local.remove(LEGACY_LAST_PITCH_KEY).catch(() => {});
  if (typeof legacy !== "number") return null;
  const match = customers.find((c) => c.id === legacy);
  if (!match) return null;
  rememberCustomer(match);
  return match.id;
}
