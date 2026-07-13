//! The gate. The desktop UI stays locked until the capture extension checks in.
//! There is no dedicated Chrome and no CDP: the extension pings the loopback
//! server (a heartbeat), and the gate derives its status from two cheap,
//! non-CDP signals — is the heartbeat fresh, and (when it isn't) is a Chrome
//! process running at all. A background monitor keeps the status live; the
//! frontend polls it and gets push events.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::chrome::{self, ProfileInfo};
use super::{Heartbeat, IngestConfig};

/// Emitted on every status transition so the UI updates without waiting for its
/// next poll. Poll (`gate_status`) is the reliable floor; the event is the fast path.
const EVENT: &str = "gate://status";

/// How often the monitor re-derives the status. Snappy so `Ready` appears
/// promptly after the extension checks in. In the steady `Ready` state each tick
/// short-circuits on the fresh heartbeat before any process check, so this only
/// costs extra `pgrep`/`tasklist` spawns while the gate is already waiting.
const MONITOR_INTERVAL: Duration = Duration::from_secs(1);

/// Consider the extension alive if it checked in within this window. Comfortably
/// larger than the extension's ~30s heartbeat so a single missed/clamped tick
/// (Chrome can clamp alarms to 60s) doesn't flap the gate to "not ready".
const HEARTBEAT_WINDOW: Duration = Duration::from_secs(150);

/// While Chrome is running but nothing has checked in yet, stay `Initializing`
/// this long before concluding the extension is missing. Kept short because a
/// present extension checks in within seconds (the content script pings on
/// LinkedIn load); a longer wait here is just a slow "load the extension"
/// verdict. The tradeoff: if the app starts while Chrome + the extension are
/// already open but idle (service worker asleep, no LinkedIn activity), the gate
/// may briefly show "extension missing" until the next heartbeat corrects it.
const STARTUP_GRACE: Duration = Duration::from_secs(10);

/// Shown when Chrome can't be located — as a gate error and as the failure of
/// the "open Chrome" commands.
const CHROME_NOT_FOUND: &str =
    "Google Chrome wasn't found. Install it, or set COURLAND_CHROME_PATH to its executable.";

/// The gate's possible states. Serialized as `{ state, detail? }` for the UI.
#[derive(Clone, PartialEq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "camelCase")]
pub enum GateStatus {
    /// Startup, before the first heartbeat (within the grace window).
    Initializing,
    /// No heartbeat and no Chrome process — offer to open Chrome.
    ChromeClosed,
    /// Chrome is running but the extension isn't checking in — show install /
    /// enable onboarding.
    ExtensionMissing,
    /// The extension checked in recently — the app is usable.
    Ready,
    /// Unrecoverable-without-action problem (e.g. Chrome not installed).
    Error(String),
}

/// Managed state holding the current gate status.
pub struct GateState {
    pub status: Mutex<GateStatus>,
    /// Set once a terminal, restart-required failure occurs (e.g. the capture
    /// server couldn't bind). Latches the status to `Error`: the monitor stops
    /// re-deriving so it can't clobber the real fault with a "Chrome closed"
    /// misdiagnosis.
    fatal: AtomicBool,
}

impl GateState {
    pub fn new() -> Self {
        GateState {
            status: Mutex::new(GateStatus::Initializing),
            fatal: AtomicBool::new(false),
        }
    }

    /// Whether a terminal failure has latched the gate.
    fn is_fatal(&self) -> bool {
        self.fatal.load(Ordering::Relaxed)
    }
}

impl Default for GateState {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a `--profile-directory` value from the frontend. Real values are
/// simple names like `Default` or `Profile 1`; reject separators / `..` so the
/// value can't be coerced into escaping the profile dir when Chrome resolves it.
fn clean_profile_dir(dir: &str) -> Result<&str, String> {
    let trimmed = dir.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid profile.".into());
    }
    Ok(trimmed)
}

/// Derive the current gate status from the heartbeat + a Chrome process check.
fn evaluate(app: &AppHandle, started: Instant) -> GateStatus {
    let heartbeat = app.state::<Heartbeat>();
    if heartbeat.is_fresh(HEARTBEAT_WINDOW) {
        return GateStatus::Ready;
    }
    // Nothing checking in. Decide "is Chrome even open?" *before* the startup
    // grace: if no Chrome is running there's nothing to wait for, so offer to
    // open it right away rather than sitting on "Connecting…". (Only if we can
    // find the binary — otherwise it's a config error.)
    if !chrome::is_running() {
        return if chrome::find_chrome().is_some() {
            GateStatus::ChromeClosed
        } else {
            GateStatus::Error(CHROME_NOT_FOUND.into())
        };
    }
    // Chrome is open but not checking in. On a cold start give the extension a
    // moment to reach the freshly-started server before declaring it missing —
    // otherwise it's genuinely not loaded/enabled (or in another profile).
    if !heartbeat.ever_seen() && started.elapsed() < STARTUP_GRACE {
        GateStatus::Initializing
    } else {
        GateStatus::ExtensionMissing
    }
}

/// Store + broadcast a status, but only when it actually changed (avoids
/// event/poll churn every monitor tick).
fn publish_if_changed(app: &AppHandle, status: GateStatus) {
    if let Some(gate) = app.try_state::<GateState>() {
        if let Ok(mut current) = gate.status.lock() {
            if *current == status {
                return;
            }
            *current = status.clone();
        }
    }
    let _ = app.emit(EVENT, status);
}

/// Latch the gate to a terminal `Error` and broadcast it. Called when a
/// restart-required failure (e.g. the capture server failing to bind) means the
/// heartbeat-derived status would be a misleading diagnosis. Best-effort: if the
/// gate state isn't reachable, the prior status stands.
pub fn set_error(app: &AppHandle, message: &str) {
    let status = GateStatus::Error(message.to_string());
    if let Some(gate) = app.try_state::<GateState>() {
        gate.fatal.store(true, Ordering::Relaxed);
        if let Ok(mut current) = gate.status.lock() {
            *current = status.clone();
        }
    }
    let _ = app.emit(EVENT, status);
}

/// Monitor loop: re-derive the gate status every few seconds and publish
/// changes. Recovers on its own — once the extension checks in, the next tick
/// flips to `Ready`; when Chrome closes, it flips back. Stops once a fatal error
/// latches the gate (see `set_error`) so it can't overwrite the real fault.
pub async fn run(app: AppHandle) {
    let started = Instant::now();
    loop {
        if app.try_state::<GateState>().is_some_and(|g| g.is_fatal()) {
            return;
        }
        publish_if_changed(&app, evaluate(&app, started));
        tokio::time::sleep(MONITOR_INTERVAL).await;
    }
}

/// Current gate status — polled by the frontend as the reliable floor.
#[tauri::command]
pub fn gate_status(state: State<GateState>) -> GateStatus {
    state
        .status
        .lock()
        .map(|s| s.clone())
        .unwrap_or(GateStatus::Initializing)
}

/// Open the user's Chrome into a specific profile (`--profile-directory`). Backs
/// the launcher's Open button; the gate then flips to `Ready` on its own once
/// the extension checks in.
#[tauri::command]
pub fn open_chrome_profile(dir: String) -> Result<(), String> {
    let dir = clean_profile_dir(&dir)?;
    let chrome = chrome::find_chrome().ok_or(CHROME_NOT_FOUND)?;
    chrome::open_chrome(&chrome, dir).map_err(|e| e.to_string())
}

/// The user's real Chrome profiles, for the launcher dropdown.
#[tauri::command]
pub fn list_chrome_profiles() -> Vec<ProfileInfo> {
    chrome::real_profiles()
}

/// The writable extension folder to load-unpack from. The onboarding screen
/// shows this path and offers to reveal it in the file manager.
#[tauri::command]
pub fn extension_dir(config: State<IngestConfig>) -> String {
    config.extension_dir.display().to_string()
}
