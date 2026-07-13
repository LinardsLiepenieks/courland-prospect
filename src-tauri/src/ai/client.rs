//! Centralized client for the user's local Claude Code CLI.
//!
//! We shell out to the `claude` binary in headless print mode (`claude -p`)
//! instead of hitting an HTTP API: this reuses the user's own Claude Code
//! install and auth, with no API key to manage. The user text is passed as a
//! process argument (no shell), so it can't be interpreted as a command.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use super::prompt::Prompt;

/// Ceiling on how many `claude` processes run at once across the whole app. A
/// batch draft cycle fires one request per scraped conversation (up to 50) in
/// quick succession, so without a cap dozens of concurrent CLI processes would
/// thrash the machine and trip API rate limits. Seven balances throughput against
/// load — generation is the batch's bottleneck, so this paces the whole pipeline;
/// the rest queue on the permit.
const MAX_CONCURRENT: usize = 7;

fn limiter() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

/// One friendly, generic message for every failure mode. Error handling here is
/// intentionally minimal: whether `claude` is missing, exits non-zero, or
/// returns nothing, the UI shows the same actionable line.
const GENERIC_ERROR: &str = "Couldn't reach Claude Code. Make sure it's installed and try again.";

/// Hard ceiling on a single CLI invocation. A generation that stalls past this
/// (hung network, wedged process) is killed so the UI button can't spin forever
/// and repeated hangs can't starve the blocking thread pool.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Whether the local Claude Code CLI can be reached. Runs `claude --version`
/// (fast, side-effect-free) and reports whether it succeeded. The UI calls this
/// to explain and disable the polish/draft features up front when Claude Code
/// isn't installed, instead of only surfacing the dependency after a click.
///
/// Blocking — call it off the UI thread (via `spawn_blocking`), like [`run`].
pub fn is_available() -> bool {
    Command::new(find_claude())
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a prompt through Claude Code and return its trimmed text output.
///
/// Blocking — spawns `claude -p <prompt>` and waits (with a [`TIMEOUT`]). Call it
/// off the UI thread (e.g. via `spawn_blocking`) so a slow generation doesn't
/// freeze the app.
pub fn run(prompt: &Prompt) -> Result<String, String> {
    let claude = find_claude();
    let mut child = Command::new(&claude)
        .arg("-p")
        .arg(prompt.render())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            // Distinct diagnostic (finding: field failures are otherwise
            // undebuggable) while the UI still shows one friendly line.
            eprintln!("ai: failed to spawn {}: {e}", claude.display());
            GENERIC_ERROR.to_string()
        })?;

    // Drain stdout on a separate thread so a large output can't deadlock against
    // a full pipe buffer while we're time-waiting on the process.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    // Poll for exit until the deadline; kill on timeout. `run` already executes
    // on a blocking thread, so the short sleep here is fine.
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("ai: `claude` timed out after {}s", TIMEOUT.as_secs());
                    return Err(GENERIC_ERROR.to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("ai: wait on `claude` failed: {e}");
                return Err(GENERIC_ERROR.to_string());
            }
        }
    };

    if !status.success() {
        eprintln!("ai: `claude` exited with {status}");
        return Err(GENERIC_ERROR.to_string());
    }

    let bytes = rx.recv().unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    if text.is_empty() {
        return Err(GENERIC_ERROR.to_string());
    }
    Ok(text)
}

/// Run a prompt through Claude Code with the global concurrency cap applied,
/// off the async runtime's blocking pool. The permit is held for the whole
/// generation (acquired before the process spawns, released when it finishes or
/// times out), so at most [`MAX_CONCURRENT`] `claude` processes ever run at once
/// no matter how many draft requests arrive together. Await this from a command
/// or HTTP handler instead of calling [`run`] on the async thread directly.
pub async fn run_capped(prompt: Prompt) -> Result<String, String> {
    let _permit = limiter()
        .acquire()
        .await
        .map_err(|_| GENERIC_ERROR.to_string())?;
    tokio::task::spawn_blocking(move || run(&prompt))
        .await
        .map_err(|e| {
            eprintln!("ai: draft task panicked: {e}");
            GENERIC_ERROR.to_string()
        })?
}

/// Shared plumbing for every feature's "polish" command: trim the input, reject
/// it when empty with a feature-specific message, then run the chosen prompt
/// builder through Claude Code (concurrency-capped, off the async runtime). Keeps
/// the trim/empty/spawn/error-map boilerplate in one place rather than copied
/// into each feature's command layer.
pub async fn polish(
    text: String,
    build: fn(&str) -> Prompt,
    empty_msg: &str,
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(empty_msg.to_string());
    }
    run_capped(build(trimmed)).await
}

/// Locate the `claude` executable, honoring a `COURLAND_CLAUDE_PATH` override
/// first (mirrors `COURLAND_CHROME_PATH`). Falls back to candidate install
/// locations, then to a bare `claude` resolved via `PATH`.
///
/// This matters most in the *packaged* app: a GUI launched from Finder/Dock
/// inherits a minimal launchd `PATH` (`/usr/bin:/bin:…`), not the user's shell
/// `PATH` where `claude` actually lives — so a bare `Command::new("claude")`
/// works in `tauri dev` but fails as NotFound in the shipped build.
fn find_claude() -> PathBuf {
    if let Ok(p) = std::env::var("COURLAND_CLAUDE_PATH") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    for candidate in candidates() {
        if candidate.exists() {
            return candidate;
        }
    }
    // Last resort: let the OS resolve it on PATH (works under `tauri dev`).
    PathBuf::from("claude")
}

/// Common install locations for the `claude` CLI, most-specific first. The
/// official installer drops it in `~/.claude/local`; the npm global and Homebrew
/// paths cover the other typical setups.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        v.push(home.join(".claude/local/claude"));
        v.push(home.join(".npm-global/bin/claude"));
        v.push(home.join(".local/bin/claude"));
    }
    v.push(PathBuf::from("/opt/homebrew/bin/claude"));
    v.push(PathBuf::from("/usr/local/bin/claude"));
    v
}

#[cfg(target_os = "windows")]
fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        v.push(PathBuf::from(format!("{home}\\.claude\\local\\claude.exe")));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        // npm global installs land here as a `.cmd` shim.
        v.push(PathBuf::from(format!("{appdata}\\npm\\claude.cmd")));
    }
    v
}
