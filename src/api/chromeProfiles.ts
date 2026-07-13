import { invoke } from "@tauri-apps/api/core";

/** A profile in the user's real Chrome. `dir` is the `--profile-directory` value
 *  (the launch key); `name` is Chrome's display name. Read live from Chrome's own
 *  profile list — we can't know which one the extension runs in, so there's no
 *  "active" flag; this is purely a launcher. */
export interface ChromeProfile {
  dir: string;
  name: string;
}

// Typed wrappers over the Rust commands. The app doesn't manage Chrome — it just
// lists the user's profiles and opens one (or the default).

export function listChromeProfiles(): Promise<ChromeProfile[]> {
  return invoke("list_chrome_profiles");
}

/** Open the user's Chrome into a specific profile at LinkedIn. */
export function openChromeProfile(dir: string): Promise<void> {
  return invoke("open_chrome_profile", { dir });
}
