//! Chrome helpers for the (CDP-free) ingest model. The app no longer runs a
//! dedicated debug Chrome — it works with the user's own Chrome. This module
//! just: finds the Chrome binary, opens it (optionally into a chosen profile),
//! reports whether a Chrome process is running, and enumerates the user's real
//! Chrome profiles from its `Local State`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use serde_json::Value;

/// The tab Chrome opens on launch — LinkedIn messaging, where the capture button
/// lives.
const LANDING_URL: &str = "https://www.linkedin.com/messaging/";

/// Chrome's default profile-directory name. The fallback when no `Local State`
/// exists yet (or it lists nothing).
const DEFAULT_PROFILE_DIRECTORY: &str = "Default";

/// One Chrome profile: its on-disk directory name (`--profile-directory` value)
/// and Chrome's display name for it. Serialized to the frontend dropdown.
#[derive(Debug, PartialEq, Serialize)]
pub struct ProfileInfo {
    pub dir: String,
    pub name: String,
}

// ── Finding + opening Chrome ────────────────────────────────────────────────

/// Locate the Google Chrome executable, honoring a `COURLAND_CHROME_PATH`
/// override first. Returns `None` if Chrome can't be found.
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("COURLAND_CHROME_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    for candidate in candidates() {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(p) = windows_registry_path() {
        return Some(p);
    }
    None
}

/// Open the user's Chrome at LinkedIn in a specific profile
/// (`--profile-directory`). No `--user-data-dir` and no debug port — this is the
/// user's normal Chrome, so it opens a window in their existing instance if one
/// is already running, or starts a fresh one. Detached (stdio silenced).
pub fn open_chrome(chrome: &Path, profile: &str) -> std::io::Result<()> {
    let child = Command::new(chrome)
        .arg(format!("--profile-directory={profile}"))
        .arg(LANDING_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // When Chrome is already running, this launcher hands the URL off and exits
    // within ~a second; a dropped Child is never reaped, so it'd linger as a
    // zombie until the app quits. Reap it on a detached thread. `wait()` only
    // collects the exit status — it never signals Chrome — so in the cold-start
    // case (where this process *is* the browser) the thread just parks until
    // Chrome exits.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// Best-effort: is a Chrome process currently running? Used only when no
/// heartbeat is present, to tell "Chrome closed" (propose opening it) apart from
/// "Chrome open but the extension isn't checking in". Any error → `false` (treat
/// as closed, the more actionable state).
pub fn is_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        pgrep("Google Chrome")
    }
    #[cfg(target_os = "linux")]
    {
        pgrep("chrome") || pgrep("google-chrome") || pgrep("chromium")
    }
    #[cfg(target_os = "windows")]
    {
        tasklist_running()
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn pgrep(name: &str) -> bool {
    Command::new("pgrep")
        .arg("-x")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn tasklist_running() -> bool {
    Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq chrome.exe", "/NH"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_lowercase()
                .contains("chrome.exe")
        })
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    )];
    if let Ok(home) = std::env::var("HOME") {
        v.push(PathBuf::from(format!(
            "{home}/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        )));
    }
    v
}

#[cfg(target_os = "windows")]
fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            v.push(PathBuf::from(format!(
                "{base}\\Google\\Chrome\\Application\\chrome.exe"
            )));
        }
    }
    v
}

#[cfg(target_os = "linux")]
fn candidates() -> Vec<PathBuf> {
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"]
        .iter()
        .map(|n| PathBuf::from("/usr/bin").join(n))
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_registry_path() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe")
        .ok()?;
    let path: String = key.get_value("").ok()?;
    let pb = PathBuf::from(path);
    pb.exists().then_some(pb)
}

// ── Enumerating the user's real Chrome profiles ─────────────────────────────

/// The user's real Chrome user-data-dir (where `Local State` + the profile
/// folders live), per platform. `None` if the home/appdata var is unset.
#[cfg(target_os = "macos")]
fn real_user_data_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("Library/Application Support/Google/Chrome"))
}

#[cfg(target_os = "windows")]
fn real_user_data_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|b| PathBuf::from(b).join("Google\\Chrome\\User Data"))
}

#[cfg(target_os = "linux")]
fn real_user_data_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config/google-chrome"))
}

/// The user's real Chrome profiles, for the launcher dropdown. Falls back to the
/// single default profile if the data-dir can't be located or read.
pub fn real_profiles() -> Vec<ProfileInfo> {
    match real_user_data_dir() {
        Some(dir) => list_profiles(&dir),
        None => default_profiles(),
    }
}

/// Enumerate the Chrome profiles inside `user_data_dir` by parsing its
/// `Local State` (`profile.info_cache`), ordered by `profiles_order` when
/// present. Defensive: a missing/unreadable/malformed file yields the single
/// default profile, so the launcher always has at least one entry.
pub fn list_profiles(user_data_dir: &Path) -> Vec<ProfileInfo> {
    std::fs::read_to_string(user_data_dir.join("Local State"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|json| parse_profiles(&json))
        .filter(|profiles| !profiles.is_empty())
        .unwrap_or_else(default_profiles)
}

/// The single default profile — the fallback whenever `Local State` can't be
/// read or lists nothing.
fn default_profiles() -> Vec<ProfileInfo> {
    vec![ProfileInfo {
        dir: DEFAULT_PROFILE_DIRECTORY.to_string(),
        name: DEFAULT_PROFILE_DIRECTORY.to_string(),
    }]
}

/// Pure parse of a `Local State` JSON value into ordered profiles. Returns empty
/// if the `profile.info_cache` object is absent; the file-reading wrapper turns
/// that into the default profile.
fn parse_profiles(json: &Value) -> Vec<ProfileInfo> {
    let Some(cache) = json.pointer("/profile/info_cache").and_then(Value::as_object) else {
        return Vec::new();
    };

    let order: Vec<String> = json
        .pointer("/profile/profiles_order")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut dirs: Vec<String> = order.into_iter().filter(|d| cache.contains_key(d)).collect();
    let mut rest: Vec<String> = cache.keys().filter(|k| !dirs.contains(k)).cloned().collect();
    rest.sort();
    dirs.append(&mut rest);

    dirs.into_iter()
        .map(|dir| {
            let name = cache
                .get(&dir)
                .and_then(|e| e.get("name"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(dir.as_str())
                .to_string();
            ProfileInfo { dir, name }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_profiles_in_declared_order_with_display_names() {
        let ls = json!({
            "profile": {
                "info_cache": {
                    "Profile 1": { "name": "Work" },
                    "Default": { "name": "Personal" }
                },
                "profiles_order": ["Default", "Profile 1"]
            }
        });
        assert_eq!(
            parse_profiles(&ls),
            vec![
                ProfileInfo { dir: "Default".into(), name: "Personal".into() },
                ProfileInfo { dir: "Profile 1".into(), name: "Work".into() },
            ]
        );
    }

    #[test]
    fn falls_back_to_dir_name_when_display_name_missing_or_empty() {
        let ls = json!({
            "profile": { "info_cache": { "Default": { "name": "" }, "Profile 2": {} } }
        });
        assert_eq!(
            parse_profiles(&ls),
            vec![
                ProfileInfo { dir: "Default".into(), name: "Default".into() },
                ProfileInfo { dir: "Profile 2".into(), name: "Profile 2".into() },
            ]
        );
    }

    #[test]
    fn empty_or_malformed_state_yields_no_profiles_from_parser() {
        assert!(parse_profiles(&json!({})).is_empty());
        assert!(parse_profiles(&json!({ "profile": {} })).is_empty());
    }

    #[test]
    fn list_profiles_defaults_when_local_state_absent() {
        let dir = std::env::temp_dir().join("courland-test-no-local-state-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(list_profiles(&dir), default_profiles());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
