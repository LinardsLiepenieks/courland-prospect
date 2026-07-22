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
    // Pin a canonical stage to its anchor + spelling; snap a freeform label to an
    // existing spelling so "security" doesn't fork "Security".
    let (position, category) = finalize_classification(position, &category, &existing);

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

/// Re-score AND re-categorize every approved snippet in a scope — the user-initiated
/// "reorganize my whole library" action. Unlike the per-edit [`run`] pass, this is a
/// full reset: it classifies each snippet in turn and force-writes the result,
/// deliberately overriding a hand-picked (`manual`) category and handing the row back
/// to auto. It runs on the interactive CLI path ([`run_capped`], which queues rather
/// than skipping) so it always completes, and emits `SNIPPETS_CHANGED` once when it
/// finishes so any other open editor for the scope reconciles in a single reshuffle.
/// Returns how many snippets it changed.
///
/// Snippets are processed openers-first and the stage-label set is accumulated as we
/// go (starting empty), so the batch mints a fresh, self-consistent set of stages
/// instead of snapping back to the scope's old (topic-style) categories.
pub(crate) async fn reclassify_all(app: AppHandle, pitch_id: Option<i64>) -> Result<usize, String> {
    let items: Vec<(i64, String)> = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let mut approved =
                repository::list_approved(&conn, pitch_id).map_err(|e| e.to_string())?;
            approved.retain(|s| !s.content.trim().is_empty());
            // Openers first, so the earliest items seed the labels later ones snap to.
            approved.sort_by(|a, b| a.position.total_cmp(&b.position));
            Ok::<_, String>(approved.into_iter().map(|s| (s.id, s.content)).collect())
        })
        .await
        .map_err(|e| format!("snippets: reclassify gather task panicked: {e}"))??
    };

    let total = items.len();
    let mut existing: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut gen_errors = 0usize;
    let mut last_err = String::new();
    for (id, content) in items {
        let ctx = ClassifyContext { content: &content, existing_categories: &existing };
        let raw = match ai::client::run_capped(Prompt::classify_snippet(&ctx)).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("snippets: reclassify generation failed: {e}");
                gen_errors += 1;
                last_err = e;
                continue;
            }
        };
        let Some((position, category)) = parse_classification(&raw) else {
            continue;
        };
        let (position, category) = finalize_classification(position, &category, &existing);

        // Force-write, but only if the row still exists and still holds the content we
        // classified — a mid-batch edit's own pass will place the newer text.
        // `None` = the row vanished or was edited mid-batch (contributes nothing);
        // `Some(wrote)` = the row is present with this stage, `wrote` = an UPDATE ran.
        let app2 = app.clone();
        let classified = content.clone();
        let cat = category.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let st = app2.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let Some(cur) = repository::find(&conn, id).map_err(|e| e.to_string())? else {
                return Ok::<Option<bool>, String>(None);
            };
            if cur.status != APPROVED || cur.content.trim() != classified.trim() {
                return Ok(None);
            }
            // Nothing to write if this row already holds this classification AND is
            // already auto — skip the no-op UPDATE and its spurious `snippets://changed`
            // reload (mirrors the per-edit `run` guard). A manual row with the same
            // labels still needs writing: the force resets it to auto (`manual = 0`).
            if cur.position == position && cur.category == cat && !cur.manual {
                return Ok(Some(false));
            }
            let did = repository::force_classification(&conn, id, position, &cat)
                .map_err(|e| e.to_string())?
                .is_some();
            Ok(Some(did))
        })
        .await
        .map_err(|e| format!("snippets: reclassify write task panicked: {e}"))??;

        // Accumulate this row's stage whenever the row is present (written OR an
        // already-correct no-op), so later snippets snap to it and the batch converges
        // on one label per stage even across an idempotent re-run.
        if let Some(wrote) = outcome {
            if !category.is_empty() && !existing.iter().any(|c| c == &category) {
                existing.push(category.clone());
            }
            if wrote {
                count += 1;
            }
        }
    }
    // If there were snippets to organize but the classifier failed on every single
    // one (CLI down/erroring), that's an outright failure — surface it rather than
    // returning a misleading `0 changed`, which the UI can't tell apart from "already
    // organized". A partial failure (some classified, some errored) still succeeds.
    if total > 0 && gen_errors == total {
        return Err(format!("couldn't reach the classifier — no snippets were organized: {last_err}"));
    }

    // Emit once, after the whole batch — not per row. A per-row emit made an open editor
    // reload and re-group repeatedly mid-batch, so cards visibly blinked out as they
    // re-homed into (collapsed) sections one at a time. One terminal event lets any
    // other open editor for this scope reconcile to the finished state in a single
    // reshuffle; the window that launched the batch reloads via its own await.
    if count > 0 {
        let _ = app.emit(SNIPPETS_CHANGED, pitch_id);
    }
    Ok(count)
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

/// The canonical conversation stages and their arc anchors, kept in lockstep with the
/// stage list the model is given in `CLASSIFY_INSTRUCTION` (ai/prompt.rs). The prompt
/// *asks* the model to use these exact labels and anchor positions; this table is where
/// code *enforces* it (see [`finalize_classification`]), so punctuation drift can't fork
/// a stage and a noisy `position` can't scramble the arc order the UI (and the draft
/// composer) derive from it.
const CANONICAL_STAGES: &[(&str, f64)] = &[
    ("Opener", 0.08),
    ("Warming up", 0.22),
    ("Warm", 0.40),
    ("Engaged", 0.58),
    ("Objection", 0.72),
    ("Calling to meet", 0.86),
    ("Follow-up", 0.96),
];

/// Fold a label to a comparison key that ignores case, whitespace, and punctuation, so
/// "Follow up", "follow-up", and "Follow-up" all collapse to the same stage.
fn normalize_label(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// If `category` names one of the canonical stages (matched loosely — case, spacing,
/// and punctuation ignored), return that stage's canonical spelling and arc anchor.
fn canonical_stage(category: &str) -> Option<(&'static str, f64)> {
    let key = normalize_label(category);
    if key.is_empty() {
        return None;
    }
    CANONICAL_STAGES
        .iter()
        .find(|(name, _)| normalize_label(name) == key)
        .map(|&(name, anchor)| (name, anchor))
}

/// Normalize a parsed classification into what actually gets stored. A canonical stage
/// is pinned to its anchor position and canonical spelling — this is what makes the
/// result deterministic: the UI orders stage sections by `position` and the draft
/// composer sorts snippets by it, and a re-run must be idempotent, none of which holds
/// if `position` is left to model noise. A freeform (non-canonical) label keeps its
/// clamped model position and is snapped to an existing spelling; empty stays empty.
fn finalize_classification(position: f64, category: &str, existing: &[String]) -> (f64, String) {
    if let Some((name, anchor)) = canonical_stage(category) {
        return (anchor, name.to_string());
    }
    (position, snap_to_existing(category, existing))
}

/// If `category` matches an existing one loosely (case, whitespace, and punctuation
/// ignored), return the existing spelling so the category set doesn't fork; else return
/// `category` unchanged (a genuinely new label).
fn snap_to_existing(category: &str, existing: &[String]) -> String {
    if category.is_empty() {
        return String::new();
    }
    let key = normalize_label(category);
    existing
        .iter()
        .find(|e| normalize_label(e) == key)
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
        let existing = vec!["Security".to_string(), "Follow-up".to_string()];
        assert_eq!(snap_to_existing("security", &existing), "Security");
        assert_eq!(snap_to_existing("SECURITY", &existing), "Security");
        // Punctuation/whitespace drift snaps to the existing spelling, not a fork.
        assert_eq!(snap_to_existing("follow up", &existing), "Follow-up");
        // A genuinely new label is kept as-is.
        assert_eq!(snap_to_existing("Integrations", &existing), "Integrations");
        assert_eq!(snap_to_existing("", &existing), "");
    }

    #[test]
    fn canonical_stage_matches_loosely_and_pins_anchor() {
        assert_eq!(canonical_stage("Follow up"), Some(("Follow-up", 0.96)));
        assert_eq!(canonical_stage("follow-up"), Some(("Follow-up", 0.96)));
        assert_eq!(canonical_stage("  OPENER "), Some(("Opener", 0.08)));
        assert_eq!(
            canonical_stage("calling to meet"),
            Some(("Calling to meet", 0.86))
        );
        // Freeform (non-canonical) and empty don't match.
        assert_eq!(canonical_stage("Discovery"), None);
        assert_eq!(canonical_stage(""), None);
    }

    #[test]
    fn finalize_pins_canonical_stage_and_preserves_freeform() {
        // A canonical stage is snapped to its anchor + spelling regardless of the
        // position the model returned — so ordering is stable and re-runs idempotent.
        assert_eq!(
            finalize_classification(0.5, "follow up", &[]),
            (0.96, "Follow-up".to_string())
        );
        // A freeform label keeps its position and snaps to an existing spelling.
        let existing = vec!["Discovery".to_string()];
        assert_eq!(
            finalize_classification(0.33, "discovery", &existing),
            (0.33, "Discovery".to_string())
        );
        // Empty stays empty, position untouched.
        assert_eq!(finalize_classification(0.4, "", &[]), (0.4, String::new()));
    }
}
