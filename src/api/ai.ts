import { invoke } from "@tauri-apps/api/core";

// Typed wrapper over the AI capability probe. The polish/draft features shell
// out to the user's local Claude Code CLI; this reports whether it's reachable
// so the UI can explain (and disable) those actions up front.

export function aiAvailable(): Promise<boolean> {
  return invoke("ai_available");
}
