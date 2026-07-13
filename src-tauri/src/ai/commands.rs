//! The AI module's only Tauri command: a capability probe the UI uses to decide
//! whether to offer (or explain the absence of) the polish/draft features.

/// Whether the local Claude Code CLI is reachable. Off-thread so the
/// `claude --version` probe never blocks the UI; a failure to even spawn the
/// task reports "unavailable" rather than erroring the call.
#[tauri::command]
pub async fn ai_available() -> bool {
    tokio::task::spawn_blocking(crate::ai::client::is_available)
        .await
        .unwrap_or(false)
}
