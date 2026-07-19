//! Classifying a snippet along two AI-derived axes: its `position` on the
//! conversation arc (0 = opener → 1 = closing ask) and its `category` (a reusable
//! group label). Triggered fire-and-forget when a snippet's content is
//! added/edited or a proposal is approved (see `commands`), mirroring the
//! `proposals` pass.
//!
//! One background LLM call per snippet — never a re-rank of the whole library — so
//! the cost is bounded and existing snippets don't churn. The call goes through
//! `run_capped_background`, which yields to interactive draft/polish work: if the
//! CLI is busy the pass is skipped and re-runs on the snippet's next edit.
//!
//! Two guarantees enforced in code, not left to the model:
//!   - **Never stomp a manual choice** — the write is gated on `manual = 0` (both in
//!     the pre-write re-read and in `set_classification`'s `WHERE`).
//!   - **Never stamp stale content** — the pass re-reads the snippet under the write
//!     lock and applies only if the content it classified is still current; a newer
//!     edit's own pass supersedes it.

use tauri::{AppHandle, Emitter, Manager};

use super::repository::{self, APPROVED};
use super::SNIPPETS_CHANGED;
use crate::ai::{self, ClassifyContext, Prompt};
use crate::database::AppState;
use crate::util::MAX_NAME_LEN;

/// Fire the classify pass for one snippet, off the request path. Returns
/// immediately; any failure is logged, not propagated. Uses Tauri's async runtime
/// (not `tokio::spawn`) so it's callable from a synchronous Tauri command, which
/// has no ambient tokio runtime of its own.
pub(crate) fn spawn(app: AppHandle, snippet_id: i64) {
    tauri::async_runtime::spawn(async move {
        run(app, snippet_id).await;
    });
}

/// Gather → classify → write, all best-effort. Every fallible step logs and returns
/// rather than propagating; this is fire-and-forget.
async fn run(app: AppHandle, snippet_id: i64) {
    // Gather under one lock: the snippet, and its scope's existing categories (so
    // the model reuses a fitting one). Only approved, non-blank, non-manual snippets
    // are classified — a blank card has nothing to place, a proposal is shown
    // separately until approved, and a manual row is off-limits.
    let gathered = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let Some(snippet) = repository::find(&conn, snippet_id).map_err(|e| e.to_string())?
            else {
                return Ok::<_, String>(None);
            };
            if snippet.content.trim().is_empty()
                || snippet.manual
                || snippet.status != APPROVED
            {
                return Ok(None);
            }
            let existing = repository::existing_categories(&conn, snippet.pitch_id)
                .map_err(|e| e.to_string())?;
            Ok(Some((snippet.pitch_id, snippet.content, existing)))
        })
        .await
    };

    let (scope, content, existing) = match gathered {
        Ok(Ok(Some(v))) => v,
        Ok(Ok(None)) => return, // deleted, blank, proposed, or manual — nothing to do
        Ok(Err(e)) => {
            eprintln!("snippets: classify gather failed: {e}");
            return;
        }
        Err(e) => {
            eprintln!("snippets: classify gather task panicked: {e}");
            return;
        }
    };

    let ctx = ClassifyContext { content: &content, existing_categories: &existing };
    let raw = match ai::client::run_capped_background(Prompt::classify_snippet(&ctx)).await {
        Ok(Some(t)) => t,
        // No spare CLI capacity — interactive work has the permits. Skip; the next
        // edit re-classifies.
        Ok(None) => return,
        Err(e) => {
            eprintln!("snippets: classify generation failed: {e}");
            return;
        }
    };

    let Some((position, category)) = parse_classification(&raw) else {
        return; // unparseable model output — drop it
    };
    // Snap to an existing category's spelling so "security" doesn't fork "Security".
    let category = snap_to_existing(&category, &existing);

    // Write under one lock, but only if the row is still classifiable AND still holds
    // the exact content we classified — otherwise a newer edit is in flight and its
    // own pass will place the current text.
    let app2 = app.clone();
    let wrote = tokio::task::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let Some(cur) = repository::find(&conn, snippet_id).map_err(|e| e.to_string())? else {
            return Ok::<bool, String>(false);
        };
        if cur.manual || cur.status != APPROVED || cur.content.trim() != content.trim() {
            return Ok(false); // pinned, un-approved, or superseded by a newer edit
        }
        // Nothing to do if the classification is unchanged — avoids a no-op UPDATE
        // and, more importantly, the spurious `snippets://changed` reload/re-sort it
        // would otherwise trigger. (We only reach here on a genuine content change,
        // so re-deriving an empty category is correct: the content is now
        // uncategorizable, and blanking it is the right answer.)
        if cur.position == position && cur.category == category {
            return Ok(false);
        }
        let updated = repository::set_classification(&conn, snippet_id, position, &category)
            .map_err(|e| e.to_string())?;
        Ok(updated.is_some())
    })
    .await;

    match wrote {
        Ok(Ok(true)) => {
            // Nudge an open editor for this scope to reload and re-sort/re-chip.
            let _ = app.emit(SNIPPETS_CHANGED, scope);
        }
        Ok(Ok(false)) => {} // nothing written — no event
        Ok(Err(e)) => eprintln!("snippets: classify write failed: {e}"),
        Err(e) => eprintln!("snippets: classify write task panicked: {e}"),
    }
}

/// Parse Claude's reply into `(position, category)`. Tolerant, mirroring
/// `parse_healed`: takes the outermost `{...}` object (so a reply wrapped in prose
/// or ```` ```json ```` fences still parses). `position` is clamped to 0.0–1.0
/// (a non-finite or missing value falls back to mid-arc 0.5); `category` is the
/// trimmed, length-bounded string (missing = empty). Returns `None` only when no
/// JSON object is found at all.
fn parse_classification(raw: &str) -> Option<(f64, String)> {
    let slice = match (raw.find('{'), raw.rfind('}')) {
        (Some(a), Some(b)) if b > a => &raw[a..=b],
        _ => return None,
    };
    let obj = match serde_json::from_str::<serde_json::Value>(slice) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return None,
    };
    let position = obj
        .get("position")
        .and_then(|v| v.as_f64())
        .filter(|p| p.is_finite())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let category: String = obj
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_NAME_LEN)
        .collect();
    Some((position, category))
}

/// If `category` matches an existing one case-insensitively, return the existing
/// spelling so the category set doesn't fork on capitalization/whitespace; else
/// return `category` unchanged (a genuinely new label).
fn snap_to_existing(category: &str, existing: &[String]) -> String {
    if category.is_empty() {
        return String::new();
    }
    existing
        .iter()
        .find(|e| e.eq_ignore_ascii_case(category))
        .cloned()
        .unwrap_or_else(|| category.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_object() {
        let (pos, cat) = parse_classification(r#"{"position": 0.8, "category": "Book a call"}"#).unwrap();
        assert_eq!(pos, 0.8);
        assert_eq!(cat, "Book a call");
    }

    #[test]
    fn parses_object_wrapped_in_prose_or_fences() {
        let raw = "Sure:\n```json\n{\"position\": 0.1, \"category\": \"Opener\"}\n```\n";
        let (pos, cat) = parse_classification(raw).unwrap();
        assert_eq!(pos, 0.1);
        assert_eq!(cat, "Opener");
    }

    #[test]
    fn clamps_position_and_defaults_bad_or_missing() {
        assert_eq!(parse_classification(r#"{"position": 1.7, "category": "X"}"#).unwrap().0, 1.0);
        assert_eq!(parse_classification(r#"{"position": -3, "category": "X"}"#).unwrap().0, 0.0);
        // Missing / non-numeric position falls back to mid-arc.
        assert_eq!(parse_classification(r#"{"category": "X"}"#).unwrap().0, 0.5);
        assert_eq!(parse_classification(r#"{"position": "high", "category": "X"}"#).unwrap().0, 0.5);
    }

    #[test]
    fn missing_category_is_empty_and_garbage_is_none() {
        assert_eq!(parse_classification(r#"{"position": 0.5}"#).unwrap().1, "");
        assert!(parse_classification("no json here").is_none());
        assert!(parse_classification("").is_none());
    }

    #[test]
    fn snaps_category_to_existing_spelling() {
        let existing = vec!["Security".to_string(), "Pricing".to_string()];
        assert_eq!(snap_to_existing("security", &existing), "Security");
        assert_eq!(snap_to_existing("SECURITY", &existing), "Security");
        // A genuinely new label is kept as-is.
        assert_eq!(snap_to_existing("Integrations", &existing), "Integrations");
        assert_eq!(snap_to_existing("", &existing), "");
    }
}
