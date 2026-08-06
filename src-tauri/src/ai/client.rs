//! Centralized client for the user's local Claude Code CLI.
//!
//! We shell out to the `claude` binary in headless print mode (`claude -p`)
//! instead of hitting an HTTP API: this reuses the user's own Claude Code
//! install and auth, with no API key to manage. The prompt is fed on the child's
//! stdin (no shell), so it can't be interpreted as a command — and, unlike an argv
//! argument, it isn't bounded by the OS command-line length limit (Windows'
//! `CreateProcessW` caps that at 32,767 chars, which a page-HTML heal prompt blows
//! straight past).
//!
//! # Every prompt here is text-in, text-out — and is spawned with no tools
//!
//! Reusing the user's install means inheriting the user's *permissions*. A plain
//! `claude -p` picks up their `~/.claude/settings.json` allowlist — which on a
//! working developer machine routinely pre-approves `Bash`, `Write`, `Edit`,
//! `WebFetch`, plus every configured MCP server. That is the correct setup for a
//! coding session and the wrong one for this app.
//!
//! It matters because most of what gets rendered into these prompts is text the
//! user did not write: LinkedIn messages from strangers, post bodies, scraped page
//! HTML. The prompts fence that content and tell the model to treat it as data,
//! and every one of them asks only for prose or a small JSON verdict back. So the
//! tools are pure downside, and [`TOOL_DENY`] removes them.
//!
//! This became load-bearing when the advance analyzer landed
//! (`features::prospects::advance`): it is the first path that feeds *inbound*
//! stranger text to the CLI with no human in the loop at all — no click, no
//! review, fired straight off a captured message. Before it, every path was
//! user-initiated or composed only from the user's own outgoing text.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{Semaphore, TryAcquireError};

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

/// The tools no prompt from this app may use, passed as `--disallowedTools`.
///
/// Deny wins over allow in Claude Code's permission resolution, so this overrides
/// whatever the user's own `settings.json` pre-approves — which is the point:
/// their allowlist is scoped to their coding work, not to a background pass over
/// a stranger's LinkedIn message.
///
/// The list is deliberately blunt rather than clever. It covers arbitrary
/// execution (`Bash`), every filesystem write (`Write`, `Edit`, `NotebookEdit`),
/// filesystem *reads* (`Read`, `Glob`, `Grep` — nothing here has any business
/// looking at the disk, and reads are the exfiltration half of a leak), network
/// egress (`WebFetch`, `WebSearch`), and sub-agent spawning (`Task`, which would
/// otherwise be a hole straight through the rest of this list).
///
/// A tool added to Claude Code in future won't appear here, which is why
/// [`STRICT_MCP`] does the structural half of the job: it cuts off every
/// configured MCP server wholesale rather than naming them one by one.
const TOOL_DENY: &str = "Bash Read Write Edit NotebookEdit Glob Grep WebFetch WebSearch Task";

/// Ignore every MCP server the user has configured. Unlike [`TOOL_DENY`] this
/// needs no enumeration and can't fall behind: a connector added tomorrow is
/// excluded by default rather than by name.
const STRICT_MCP: &str = "--strict-mcp-config";

/// A generation that hit [`TIMEOUT`] is a different problem from a missing binary,
/// and telling the user to check their install sends them the wrong way. The usual
/// cause is an oversized prompt (a very large snippet library), so name that.
const TIMEOUT_ERROR: &str =
    "Claude Code took too long to respond. This usually means there's too much material to send — try trimming your snippet library.";

/// Hard ceiling on a single CLI invocation. A generation that stalls past this
/// (hung network, wedged process) is killed so the UI button can't spin forever
/// and repeated hangs can't starve the blocking thread pool.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How many characters of a failing invocation's stderr to log. Enough for the CLI's
/// one-line "unknown option" complaint, short enough that a runaway can't flood the log.
const MAX_STDERR_LOG: usize = 400;

/// How long to wait for a drain thread to hand over its output AFTER the child has exited.
///
/// [`TIMEOUT`] bounds waiting for the *process*; this bounds waiting for the *pipes*, which
/// is not the same thing and used to be unbounded. A drain thread reports only once
/// `read_to_end` returns, and that needs every write end of its pipe closed — so a `claude`
/// invocation that leaves behind a detached descendant holding stdout or stderr (Node CLIs
/// do: updaters, telemetry helpers) keeps the pipe open after the parent is gone, the
/// blocking `recv()` never returns, and `run` never returns either.
///
/// That is worse than one hung call. The concurrency permit is held until [`run_capped`]
/// returns, so a call parked here surrenders a permit for the life of the app; enough of
/// them and every AI feature is dead — spinners that never resolve, background passes that
/// silently skip, and no error reported anywhere, because nothing ever completes to report
/// one. Short, because by the time this is consulted the process has already exited.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

/// Ceiling on the `claude --version` availability probe. Much shorter than [`TIMEOUT`]:
/// printing a version string is instant, so anything slower is wedged rather than busy,
/// and the UI is waiting on this answer to decide whether to enable the AI features.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether the local Claude Code CLI can be reached. Runs `claude --version`
/// (fast, side-effect-free) and reports whether it succeeded. The UI calls this
/// to explain and disable the polish/draft features up front when Claude Code
/// isn't installed, instead of only surfacing the dependency after a click.
///
/// Blocking — call it off the UI thread (via `spawn_blocking`), like [`run`].
pub fn is_available() -> bool {
    let Ok(mut child) = Command::new(find_claude())
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Bounded, unlike a bare `status()`. This runs on a blocking thread that a Tauri
    // command awaits, so a wedged `claude --version` would otherwise hang that command
    // forever — `run` has a ceiling and the probe had none.
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("ai: `claude --version` timed out");
                    return false;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return false,
        }
    }
}

/// Run a prompt through Claude Code and return its trimmed text output.
///
/// Blocking — spawns `claude -p`, feeds the prompt on stdin, and waits (with a
/// [`TIMEOUT`]). Call it off the UI thread (e.g. via `spawn_blocking`) so a slow
/// generation doesn't freeze the app.
pub fn run(prompt: &Prompt) -> Result<String, String> {
    let claude = find_claude();
    let rendered = prompt.render();
    let mut child = Command::new(&claude)
        .arg("-p")
        // Text in, text out, no tools — see the module docs. Both flags are
        // passed on every invocation rather than per-prompt: there is no prompt
        // in this app that needs a tool, so an opt-in would only be a chance to
        // forget one.
        .arg("--disallowedTools")
        .arg(TOOL_DENY)
        .arg(STRICT_MCP)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Piped, not null. The user-facing message stays generic, but the CLI writes the
        // actual cause here and ONLY here — and the cause that matters is a rejected
        // flag: this app passes `--disallowedTools` and `--strict-mcp-config` on every
        // invocation, `is_available()` only probes `--version` so it never exercises
        // them, and an unrecognised option exits non-zero with an empty stdout. Discarding
        // stderr made that indistinguishable from a missing binary, so every AI feature
        // failed with "make sure it's installed" about a binary that was installed and
        // working, and the one line naming the real problem was thrown away.
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            // Distinct diagnostic (finding: field failures are otherwise
            // undebuggable) while the UI still shows one friendly line.
            eprintln!("ai: failed to spawn {}: {e}", claude.display());
            GENERIC_ERROR.to_string()
        })?;

    // Feed the prompt on stdin from its own thread: a large prompt can fill the
    // pipe buffer, and writing it inline would block us before we start draining
    // stdout (a deadlock). Dropping the handle closes stdin → EOF, so `claude`
    // stops waiting for more input. A write failure (e.g. the process already
    // exited) is ignored here; the exit-status / empty-output checks below surface
    // it as the one friendly error.
    let mut stdin = child.stdin.take().expect("stdin is piped");
    std::thread::spawn(move || {
        let _ = stdin.write_all(rendered.as_bytes());
    });

    // Drain stdout on a separate thread so a large output can't deadlock against
    // a full pipe buffer while we're time-waiting on the process.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    // Drain stderr too, for the same deadlock reason — a chatty failure could otherwise
    // fill its pipe and wedge the process we're waiting on.
    // Only the first chunk is ever logged, so only the first chunk is read: `take` caps the
    // buffer instead of holding a runaway stderr in memory to throw most of it away. It also
    // means this thread finishes promptly on a chatty failure rather than reading to EOF.
    // Generous multiple of the log budget so a multi-byte tail still has room.
    let stderr = child.stderr.take().expect("stderr is piped");
    let (err_tx, err_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.take((MAX_STDERR_LOG * 8) as u64).read_to_end(&mut buf);
        let _ = err_tx.send(buf);
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
                    return Err(TIMEOUT_ERROR.to_string());
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
        // Log a bounded tail of stderr alongside the status. This is the line that says
        // whether the CLI is missing, rejected one of our flags, or failed for its own
        // reasons — the difference between "reinstall Claude Code" and "this build
        // doesn't accept --strict-mcp-config". Bounded so a runaway stderr can't flood
        // the log; the user still sees only the one friendly message.
        let detail = err_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();
        let detail = String::from_utf8_lossy(&detail);
        let detail = detail.trim();
        if detail.is_empty() {
            eprintln!("ai: `claude` exited with {status} (no stderr)");
        } else {
            let tail: String = detail.chars().take(MAX_STDERR_LOG).collect();
            eprintln!("ai: `claude` exited with {status}: {tail}");
        }
        return Err(GENERIC_ERROR.to_string());
    }

    // Bounded, not blocking — see `DRAIN_GRACE`. The child has exited by now, so the output
    // is either already buffered or held open by something that outlived it; waiting forever
    // on the latter parks this call's permit permanently. A drain that misses the grace
    // period is reported as the one friendly error rather than as an empty success.
    let bytes = match rx.recv_timeout(DRAIN_GRACE) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!(
                "ai: `claude` exited but its output never closed within {}s — abandoning the read",
                DRAIN_GRACE.as_secs()
            );
            return Err(GENERIC_ERROR.to_string());
        }
    };
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

/// Run a prompt through Claude Code for BACKGROUND, best-effort work — like
/// proposing snippets from a captured message — without ever making an interactive
/// caller wait. Unlike [`run_capped`], which queues on [`Semaphore::acquire`], this
/// uses `try_acquire`: if no CLI permit is immediately free it returns `Ok(None)`
/// and does nothing. Because a background task never joins the permit queue, a
/// user-facing draft/polish `acquire` can never sit behind a backlog of background
/// generations (tokio's semaphore is FIFO-fair, so a queued background waiter would
/// otherwise take the next permit ahead of a later foreground one). The dropped
/// pass is harmless — the caller re-proposes on the user's next action.
pub async fn run_capped_background(prompt: Prompt) -> Result<Option<String>, String> {
    let _permit = match limiter().try_acquire() {
        Ok(p) => p,
        // No spare capacity right now — yield to foreground work and skip this pass.
        Err(TryAcquireError::NoPermits) => return Ok(None),
        Err(TryAcquireError::Closed) => return Err(GENERIC_ERROR.to_string()),
    };
    tokio::task::spawn_blocking(move || run(&prompt))
        .await
        .map_err(|e| {
            eprintln!("ai: background task panicked: {e}");
            GENERIC_ERROR.to_string()
        })?
        .map(Some)
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
