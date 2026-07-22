//! Tauri commands for the watchlist — the frontend's entry point for managing the
//! list of LinkedIn profiles a comment run checks for new posts. Thin by design:
//! lock the shared connection, validate input, delegate to `repository`, and map
//! errors to strings the UI can display.

use tauri::State;

use super::model::WatchedProfile;
use super::repository;
use crate::database::AppState;
use crate::util::{bounded, MAX_NAME_LEN, MAX_TEXT_LEN};

#[tauri::command]
pub fn list_watched_profiles(state: State<AppState>) -> Result<Vec<WatchedProfile>, String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::list(&conn).map_err(|e| e.to_string())
}

/// Add a LinkedIn profile to the watchlist (or refresh its label if already
/// watched). The URL must look like a LinkedIn member profile — the run navigates
/// to its recent activity, so a non-profile link would just fail there.
#[tauri::command]
pub fn add_watched_profile(
    state: State<AppState>,
    linkedin_url: String,
    name: String,
) -> Result<WatchedProfile, String> {
    let linkedin_url = bounded(&linkedin_url, MAX_TEXT_LEN, "Profile URL")?;
    let name = bounded(&name, MAX_NAME_LEN, "Name")?;
    if linkedin_url.is_empty() {
        return Err("A LinkedIn profile URL is required.".into());
    }
    if !is_linkedin_profile_url(linkedin_url) {
        return Err("That doesn't look like a LinkedIn profile URL (expected linkedin.com/in/…).".into());
    }
    // Canonicalize to `https://www.linkedin.com/in/<slug>/` before storing, so the
    // list dedups on the person (the UNIQUE constraint) rather than on the exact
    // pasted string — otherwise `/in/ada`, `/in/ada/`, and a `?…`-suffixed copy of
    // the same profile would each add a distinct row and the run would visit them
    // repeatedly. Mirrors the extension's own `normalizeProfileUrl` (slug case
    // preserved, matching how prospects are keyed).
    let canonical = canonical_profile_url(linkedin_url);
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    repository::add(&conn, &canonical, name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_watched_profile(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.conn.lock().map_err(|e| e.to_string())?;
    match repository::delete(&conn, id).map_err(|e| e.to_string())? {
        0 => Err("Watched profile not found.".into()),
        _ => Ok(()),
    }
}

/// A light sanity check that a string points at a LinkedIn member profile —
/// case-insensitive `linkedin.com/in/` substring. Deliberately forgiving on scheme
/// and trailing path: the extension canonicalizes the URL when it builds the
/// recent-activity page to visit; this only rejects an obviously-wrong paste.
fn is_linkedin_profile_url(url: &str) -> bool {
    url.to_ascii_lowercase().contains("linkedin.com/in/")
}

/// Canonicalize a LinkedIn profile URL to `https://www.linkedin.com/in/<slug>/`,
/// the same shape the extension's `normalizeProfileUrl` produces (slug case
/// preserved). Extracts the slug as the path segment after `/in/` (case-insensitive
/// locate), dropping any trailing path, query, or fragment. Falls back to the input
/// unchanged when no slug can be found (the caller has already validated the URL
/// contains `linkedin.com/in/`, so this normally succeeds).
fn canonical_profile_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    if let Some(pos) = lower.find("/in/") {
        // Same byte offset in the original (ASCII marker), so slug case is kept.
        let after = &url[pos + "/in/".len()..];
        let slug = after
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .trim();
        if !slug.is_empty() {
            return format!("https://www.linkedin.com/in/{slug}/");
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::{canonical_profile_url, is_linkedin_profile_url};

    #[test]
    fn accepts_profile_urls_and_rejects_others() {
        assert!(is_linkedin_profile_url("https://www.linkedin.com/in/ada/"));
        assert!(is_linkedin_profile_url("linkedin.com/in/ada"));
        assert!(is_linkedin_profile_url("https://LinkedIn.com/IN/Ada")); // case-insensitive
        assert!(!is_linkedin_profile_url("https://www.linkedin.com/company/acme/"));
        assert!(!is_linkedin_profile_url("https://example.com/in/ada"));
        assert!(!is_linkedin_profile_url("just some text"));
    }

    #[test]
    fn canonicalizes_variants_of_the_same_profile_to_one_form() {
        let canonical = "https://www.linkedin.com/in/ada/";
        for variant in [
            "https://www.linkedin.com/in/ada/",
            "https://www.linkedin.com/in/ada",
            "http://linkedin.com/in/ada",
            "https://www.linkedin.com/in/ada/?miniProfileUrn=x",
            "https://www.linkedin.com/in/ada/recent-activity/all/",
        ] {
            assert_eq!(canonical_profile_url(variant), canonical, "variant: {variant}");
        }
        // Slug case is preserved (matches the extension's normalizeProfileUrl).
        assert_eq!(
            canonical_profile_url("https://www.linkedin.com/in/Ada-Lovelace/"),
            "https://www.linkedin.com/in/Ada-Lovelace/"
        );
        // No slug → returned unchanged (defensive; the caller pre-validates).
        assert_eq!(canonical_profile_url("no slug here"), "no slug here");
    }
}
