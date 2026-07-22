import { invoke } from "@tauri-apps/api/core";

/** A hand-curated LinkedIn profile to check for new posts during a
 *  comment run. A small, app-wide list, deduped on `linkedin_url`. */
export interface WatchedProfile {
  id: number;
  /** The LinkedIn profile URL (contains "linkedin.com/in/"). */
  linkedin_url: string;
  /** A human label for the profile; may be empty. */
  name: string;
  created_at: string;
}

// Typed wrappers over the Rust watchlist commands. All SQL lives in the
// backend; these are the only entry points the UI uses to touch watchlist data.

export function listWatchedProfiles(): Promise<WatchedProfile[]> {
  return invoke("list_watched_profiles");
}

/** Add a profile to the watchlist. The backend validates the URL (it must
 *  contain "linkedin.com/in/") and rejects otherwise; re-adding the same URL
 *  updates its label (dedup on URL). Returns the created/updated record. */
export function addWatchedProfile(
  linkedinUrl: string,
  name: string,
): Promise<WatchedProfile> {
  return invoke("add_watched_profile", { linkedinUrl, name });
}

export function deleteWatchedProfile(id: number): Promise<void> {
  return invoke("delete_watched_profile", { id });
}
