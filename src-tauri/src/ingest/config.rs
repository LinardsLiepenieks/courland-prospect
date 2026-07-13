//! First-run provisioning: a persisted shared token, a chosen loopback port, and
//! a *writable* copy of the unpacked extension with `config.json` written into it.
//!
//! Why copy the extension out of the bundle: a distributed macOS `.app` is
//! code-signed and effectively read-only, so we can't write `config.json` into
//! `resource_dir`. We copy the bundled extension into `app_data_dir/extension`
//! and load-unpacked from there.

use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::Path;

use tauri::{AppHandle, Manager};

use super::{IngestConfig, DEFAULT_APP_PORT, EXTENSION_ID};
use crate::util::random_alphanumeric;

/// User-facing folder (under Documents) the unpacked extension is staged into.
const EXTENSION_FOLDER_NAME: &str = "Courland Prospect Extension";

/// Prepare everything the server + launcher need. Idempotent: safe to call on
/// every startup. Refreshes the extension's code from the bundle, preserves the
/// token across runs (regenerating would 401 an already-running Chrome whose
/// service worker still holds the old one), and (re)writes `config.json`.
pub fn provision(app: &AppHandle) -> Result<IngestConfig, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // The extension folder lives in Documents (not the hidden app-data dir) so
    // it's trivial to point Chrome's "Load unpacked" at it during setup.
    let docs_dir = app.path().document_dir().map_err(|e| e.to_string())?;
    let extension_dir = docs_dir.join(EXTENSION_FOLDER_NAME);
    fs::create_dir_all(&extension_dir).map_err(|e| e.to_string())?;

    // Refresh the extension's built files from the bundle if present. In `tauri
    // dev` the resource may be absent — that's fine; the dir can be populated by
    // pointing load-unpacked at the built extension manually.
    if let Ok(res_dir) = app.path().resource_dir() {
        let src = res_dir.join("extension");
        if src.is_dir() {
            copy_dir(&src, &extension_dir).map_err(|e| {
                format!("Failed to stage extension into {}: {e}", extension_dir.display())
            })?;
        }
    }

    let (token, first_run) = load_or_create_token(&data_dir)?;
    let app_port = pick_port(DEFAULT_APP_PORT);

    // The extension's service worker reads this (privately — never web-accessible).
    let config = serde_json::json!({ "token": token, "appPort": app_port });
    let body = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    let config_path = extension_dir.join("config.json");
    fs::write(&config_path, body).map_err(|e| {
        format!("Failed to write extension config.json: {e}")
    })?;
    // The config carries the shared token; keep it owner-only so another local
    // user on a shared machine can't read it (Chrome runs as this user anyway).
    restrict_to_owner(&config_path);

    // First-time setup: pop the extension folder open in the file manager so the
    // user can immediately drag/point Chrome at it. Best-effort — a failure here
    // never blocks startup (the onboarding screen still shows the path + Reveal).
    if first_run {
        use tauri_plugin_opener::OpenerExt;
        let _ = app
            .opener()
            .open_path(extension_dir.to_string_lossy().to_string(), None::<&str>);
    }

    Ok(IngestConfig {
        token,
        app_port,
        extension_id: EXTENSION_ID.to_string(),
        extension_dir,
    })
}

/// Load the persisted token, or generate + persist a new 32-char one. The bool
/// is `true` when a new token was created — our proxy for "first run", used to
/// decide whether to auto-open the extension folder.
fn load_or_create_token(data_dir: &Path) -> Result<(String, bool), String> {
    let token_path = data_dir.join("ingest_token");
    if let Ok(existing) = fs::read_to_string(&token_path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok((trimmed.to_string(), false));
        }
    }
    let token = random_alphanumeric(32);
    fs::write(&token_path, &token).map_err(|e| format!("Failed to persist token: {e}"))?;
    // Owner-only: the token is the shared secret the whole gate rests on.
    restrict_to_owner(&token_path);
    Ok((token, true))
}

/// Best-effort restrict a file to owner read/write (`0600`) on Unix. A no-op on
/// Windows (per-user profile dirs already scope access) and a silent no-op if
/// the metadata can't be read — tightening perms must never fail provisioning.
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Prefer `preferred`; if it's taken, let the OS assign a free port. There's a
/// tiny window between this probe and the server binding, acceptable for a local
/// single-user app (a lost race surfaces as a gate error, not silent breakage).
fn pick_port(preferred: u16) -> u16 {
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(preferred)
}

/// Recursively copy `src` into `dst`, overwriting files. Does not delete extra
/// files already in `dst` (notably the runtime-written `config.json`, which is
/// never present in `src`).
fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}
