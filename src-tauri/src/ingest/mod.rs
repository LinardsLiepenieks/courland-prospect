//! Ingest infrastructure — how the Chrome extension feeds prospects into the CRM.
//!
//! This is cross-cutting infra (like `crate::database`), not a vertical feature
//! slice. The app works with the user's *own* Chrome (no dedicated browser, no
//! CDP): a loopback HTTP server receives captures, and the "gate" keeps the app
//! locked until the extension checks in (a heartbeat).
//!
//!  - `config`   — token + port provisioning; ships the extension into a writable dir
//!  - `chrome`   — find/open the user's Chrome; process check; profile enumeration
//!  - `server`   — axum server on Tauri's runtime (GET /health, /pitches;
//!                 POST /prospects, /messages, /draft)
//!  - `security` — pure Host/token/Origin checks (unit-tested)
//!  - `gate`     — GateStatus (heartbeat + process) + commands + events

pub mod chrome;
pub mod config;
pub mod gate;
pub mod server;

mod security;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Preferred loopback port for the ingest HTTP server. Falls back to an OS-picked
/// free port if taken; the chosen port is written into the extension's config.
pub const DEFAULT_APP_PORT: u16 = 47823;

/// The extension's stable ID, derived from the pinned public `key` in its
/// manifest. Constant because the CORS allowlist needs to name
/// `chrome-extension://<id>` exactly, ahead of time.
pub const EXTENSION_ID: &str = "knipphmpmemfkimdiknnjjbelecnkenf";

/// Runtime configuration shared by the server. Cheap to clone; managed by Tauri.
#[derive(Clone)]
pub struct IngestConfig {
    /// Shared secret the extension must present on every request.
    pub token: String,
    /// The port the loopback server actually bound.
    pub app_port: u16,
    /// The extension's stable ID.
    pub extension_id: String,
    /// Writable copy of the unpacked extension (where `config.json` is written).
    pub extension_dir: PathBuf,
}

/// Last time the extension checked in. The server refreshes it on every authed
/// request (the extension's periodic `GET /health` is the pulse); the gate reads
/// it to decide readiness. Managed by Tauri so both can reach it.
pub struct Heartbeat(Mutex<Option<Instant>>);

impl Heartbeat {
    pub fn new() -> Self {
        Heartbeat(Mutex::new(None))
    }

    /// Record a check-in now.
    pub fn beat(&self) {
        if let Ok(mut last) = self.0.lock() {
            *last = Some(Instant::now());
        }
    }

    /// Has the extension checked in within `window`?
    pub fn is_fresh(&self, window: Duration) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|last| *last)
            .map(|t| t.elapsed() < window)
            .unwrap_or(false)
    }

    /// Whether the extension has *ever* checked in this session. Distinguishes a
    /// cold start (still waiting for the first ping → show "Connecting…") from a
    /// genuine drop (was fresh, now stale).
    pub fn ever_seen(&self) -> bool {
        self.0.lock().ok().map(|last| last.is_some()).unwrap_or(false)
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}
