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
    // Gather under one lock: the snippet, and the library's existing categories (so
    // the model reuses a fitting one). Only approved, non-blank snippets are classified —
    // a blank card has nothing to place, and a proposal is shown separately until approved.
    //
    // A MANUAL row is classified too, deliberately. The pin protects the stage the user
    // chose, and `repository::set_classification` enforces that per column; skipping the
    // row entirely also withheld its `topic`, which has no hand-set counterpart to protect
    // and so has nothing to gain from the pin. Bailing here is what made a hand-organized
    // library permanently untopiced.
    let gathered = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let Some(snippet) = repository::find(&conn, snippet_id).map_err(|e| e.to_string())?
            else {
                return Ok::<_, String>(None);
            };
            if snippet.content.trim().is_empty() || snippet.status != APPROVED {
                return Ok(None);
            }
            let existing = repository::existing_categories(&conn).map_err(|e| e.to_string())?;
            let topics = repository::existing_topics(&conn).map_err(|e| e.to_string())?;
            Ok(Some((snippet.content, existing, topics)))
        })
        .await
    };

    let (content, existing, topics) = match gathered {
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

    let ctx = ClassifyContext {
        content: &content,
        existing_categories: &existing,
        existing_topics: &topics,
    };
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

    // One shared rule for what the reply means — see `decide`. Pins a canonical stage to
    // its anchor + spelling, snaps a freeform stage or topic to an existing spelling so
    // "security" doesn't fork "Security", and keeps the snippet where it is when the reply
    // names no stage.
    let Decision::Write { position, category, topic } =
        decide(parse_classification(&raw), &existing, &topics)
    else {
        // Unreadable reply. Dropping it leaves the row as it was, which is the safe
        // outcome: the next edit re-classifies.
        eprintln!("snippets: classify could not read the classifier's reply; leaving the snippet as it was");
        return;
    };

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
        if cur.status != APPROVED || cur.content.trim() != content.trim() {
            return Ok(false); // un-approved, or superseded by a newer edit
        }
        let updated = repository::set_classification(
            &conn,
            snippet_id,
            position.resolve(cur.position),
            &category,
            &topic,
        )
        .map_err(|e| e.to_string())?;
        // Report whether anything actually MOVED, not whether a row matched. The write
        // honours a manual pin per column, so asking the returned row is the only way to
        // know without restating that rule here — and restating it is how the two would
        // drift. This gates the `snippets://changed` emit below, and a spurious emit is
        // what made cards visibly blink out and re-home mid-pass.
        Ok(updated.is_some_and(|u| {
            u.position != cur.position || u.category != cur.category || u.topic != cur.topic
        }))
    })
    .await;

    match wrote {
        Ok(Ok(true)) => {
            // Nudge an open editor to reload and re-sort/re-chip.
            let _ = app.emit(SNIPPETS_CHANGED, ());
        }
        Ok(Ok(false)) => {} // nothing written — no event
        Ok(Err(e)) => eprintln!("snippets: classify write failed: {e}"),
        Err(e) => eprintln!("snippets: classify write task panicked: {e}"),
    }
}

/// Re-score AND re-categorize every approved snippet in the library — the
/// user-initiated "reorganize my whole library" action. Unlike the per-edit [`run`]
/// pass, this is a full reset: it classifies each snippet in turn and force-writes the
/// result, deliberately overriding a hand-picked (`manual`) category and handing the row
/// back to auto. It runs on the interactive CLI path ([`run_capped`], which queues rather
/// than skipping) so it always completes, and emits `SNIPPETS_CHANGED` once when it
/// finishes so any other open editor reconciles in a single reshuffle.
/// Returns how many snippets it changed.
///
/// Snippets are processed openers-first and the stage-label set is accumulated as we
/// go (starting empty), so the batch mints a fresh, self-consistent set of stages
/// instead of snapping back to the library's old (topic-style) categories.
pub(crate) async fn reclassify_all(app: AppHandle) -> Result<usize, String> {
    let items: Vec<(i64, String)> = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let mut approved = repository::list_approved(&conn).map_err(|e| e.to_string())?;
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
    // Topics accumulate the same way stages do, and from empty for the same reason: the
    // batch mints one self-consistent vocabulary instead of snapping back to whatever the
    // library happened to hold before.
    let mut topics: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut gen_errors = 0usize;
    let mut last_err = String::new();
    for (id, content) in items {
        let ctx = ClassifyContext {
        content: &content,
        existing_categories: &existing,
        existing_topics: &topics,
    };
        let raw = match ai::client::run_capped(Prompt::classify_snippet(&ctx)).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("snippets: reclassify generation failed: {e}");
                gen_errors += 1;
                last_err = e;
                continue;
            }
        };
        // Same rule as the per-edit pass — see `decide`. An UNREADABLE reply is a failed
        // classification, not a no-op: count it with the generation errors so a classifier
        // that answers every prompt with garbage can't come back as a reassuring
        // `0 changed`. An empty STAGE is not in that category — it is the answer the prompt
        // asks for when a line serves no conversational role, so it is written (the snippet
        // becomes unstaged but does not move on the arc) and never counted as an error.
        // Counting it was wrong twice over: a library of pure conversational moves failed
        // outright with a message blaming an unreachable CLI, and skipping the write left a
        // stale topic-style label in the stage field that the next classify prompt then
        // offered back as a stage to reuse — undoing the cleanup this batch exists to do.
        let (position, category, topic) =
            match decide(parse_classification(&raw), &existing, &topics) {
                Decision::Write { position, category, topic } => (position, category, topic),
                Decision::Failed(why) => {
                    eprintln!("snippets: reclassify skipped a snippet — {why}");
                    gen_errors += 1;
                    last_err = why.to_string();
                    continue;
                }
            };

        // Force-write, but only if the row still exists and still holds the content we
        // classified — a mid-batch edit's own pass will place the newer text.
        // `None` = the row vanished or was edited mid-batch (contributes nothing);
        // `Some(wrote)` = the row is present with this stage, `wrote` = an UPDATE ran.
        let app2 = app.clone();
        let classified = content.clone();
        let cat = category.clone();
        let top = topic.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let st = app2.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let Some(cur) = repository::find(&conn, id).map_err(|e| e.to_string())? else {
                return Ok::<Option<bool>, String>(None);
            };
            if cur.status != APPROVED || cur.content.trim() != classified.trim() {
                return Ok(None);
            }
            // A reply that named no stage carries no arc information, so the snippet keeps
            // the position it already had rather than being relocated to a guess.
            let target = position.resolve(cur.position);
            // Nothing to write if this row already holds this classification AND is
            // already auto — skip the no-op UPDATE and its spurious `snippets://changed`
            // reload (mirrors the per-edit `run` guard). A manual row with the same
            // labels still needs writing: the force resets it to auto (`manual = 0`).
            if cur.position == target && cur.category == cat && cur.topic == top && !cur.manual {
                return Ok(Some(false));
            }
            let did = repository::force_classification(&conn, id, target, &cat, &top)
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
            if !topic.is_empty() && !topics.iter().any(|t| t == &topic) {
                topics.push(topic.clone());
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
    // other open editor reconcile to the finished state in a single reshuffle; the
    // window that launched the batch reloads via its own await.
    if count > 0 {
        let _ = app.emit(SNIPPETS_CHANGED, ());
    }
    Ok(count)
}

/// One classification as the model actually answered it, before any judgement about what
/// to store.
struct Parsed {
    /// Clamped to 0.0–1.0; mid-arc 0.5 when absent or non-finite.
    position: f64,
    /// `None` when the reply carried no usable `category` field at all (absent, null, or
    /// not a string) — a malformed answer. `Some("")` is different and important: it is the
    /// answer `CLASSIFY_INSTRUCTION` explicitly asks for when a line serves no
    /// conversational role, and must not be read as a failure.
    ///
    /// Collapsing those two was the bug this type exists to prevent. It made a compliant
    /// reply indistinguishable from a broken one, and the two passes then guessed
    /// differently: the batch counted it as a generation error (so a library of pure
    /// conversational moves failed outright, blaming an unreachable CLI), while the
    /// per-edit pass wrote it *and* moved the snippet to the 0.5 fallback.
    category: Option<String>,
    /// Always a plain string: an empty topic is a normal answer on every path, so there is
    /// nothing here to distinguish.
    topic: String,
}

/// Where a classification places a snippet on the conversation arc.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ArcPosition {
    At(f64),
    /// Leave the snippet where it is. A reply that names no stage says nothing about where
    /// on the arc the line belongs either, so its `position` is not evidence — relocating
    /// on it would scramble the order the library and the draft composer both read.
    Unchanged,
}

impl ArcPosition {
    fn resolve(&self, current: f64) -> f64 {
        match self {
            ArcPosition::At(p) => *p,
            ArcPosition::Unchanged => current,
        }
    }
}

/// What one classification reply means for one snippet.
enum Decision {
    Write { position: ArcPosition, category: String, topic: String },
    /// The reply couldn't be read as a classification. Not a no-op: a classifier answering
    /// every prompt with garbage must not come back as a reassuring "0 changed", so the
    /// batch pass counts these toward its all-failed check.
    Failed(&'static str),
}

/// Turn a parsed reply into what to store — the whole per-row rule, in one pure function
/// both passes call.
///
/// It lives apart from the orchestrators on purpose. This logic used to be inlined in each
/// of them, where an `AppHandle` and a live CLI put it out of reach of any test, and it
/// promptly drifted: the two paths disagreed about what an empty stage meant, in opposite
/// directions, and nothing caught it. There is one rule now, and it is testable.
fn decide(
    parsed: Option<Parsed>,
    existing_categories: &[String],
    existing_topics: &[String],
) -> Decision {
    let Some(parsed) = parsed else {
        return Decision::Failed("the classifier's reply couldn't be read");
    };
    let Some(category) = parsed.category else {
        return Decision::Failed("the classifier's reply named no stage field");
    };
    let (position, category, topic) = finalize_classification(
        parsed.position,
        &category,
        &parsed.topic,
        existing_categories,
        existing_topics,
    );
    // An empty stage is an answer ("this line serves no conversational role"), so it IS
    // written — leaving the row's old label in place would keep a stale, often topic-style
    // category that the next classify prompt then offers back as a stage to reuse, against
    // that prompt's own rule. But the snippet stays put on the arc: see `ArcPosition`.
    let position = if category.is_empty() {
        ArcPosition::Unchanged
    } else {
        ArcPosition::At(position)
    };
    Decision::Write { position, category, topic }
}

/// Parse Claude's reply into a [`Parsed`]. Locating the `{...}` object past any prose or
/// ```` ```json ```` fences is [`ai::parse::json_object`]'s job. Labels are trimmed and
/// length-bounded. Returns `None` only when no JSON object is found at all.
fn parse_classification(raw: &str) -> Option<Parsed> {
    let obj = ai::parse::json_object(raw)?;
    let position = obj
        .get("position")
        .and_then(|v| v.as_f64())
        .filter(|p| p.is_finite())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let label = |key: &str| -> Option<String> {
        Some(obj.get(key)?.as_str()?.trim().chars().take(MAX_NAME_LEN).collect())
    };
    Some(Parsed {
        position,
        category: label("category"),
        topic: label("topic").unwrap_or_default(),
    })
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
/// `topic` gets only the snap — no canonical table and no anchor, because there is no
/// fixed set of topics and a topic says nothing about where on the arc a line sits.
/// Snapping still matters, and arguably more: stages have a canonical list to fall back
/// on, whereas the topic vocabulary is *only* what's already in the library, so an
/// unsnapped "security" beside "Security" permanently forks a subject.
fn finalize_classification(
    position: f64,
    category: &str,
    topic: &str,
    existing_categories: &[String],
    existing_topics: &[String],
) -> (f64, String, String) {
    let topic = snap_to_existing(topic, existing_topics);
    if let Some((name, anchor)) = canonical_stage(category) {
        return (anchor, name.to_string(), topic);
    }
    (position, snap_to_existing(category, existing_categories), topic)
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
        let p = parse_classification(
            r#"{"position": 0.8, "category": "Book a call", "topic": "Pricing"}"#,
        )
        .unwrap();
        assert_eq!(p.position, 0.8);
        assert_eq!(p.category.as_deref(), Some("Book a call"));
        assert_eq!(p.topic, "Pricing");
    }

    /// A reply from before the topic axis existed still parses — the field is simply
    /// empty, which is a legitimate value rather than a failure.
    #[test]
    fn a_reply_with_no_topic_field_parses_with_an_empty_topic() {
        let p = parse_classification(r#"{"position": 0.8, "category": "Objection"}"#).unwrap();
        assert_eq!(p.category.as_deref(), Some("Objection"));
        assert_eq!(p.topic, "");
    }

    #[test]
    fn parses_object_wrapped_in_prose_or_fences() {
        let raw = "Sure:\n```json\n{\"position\": 0.1, \"category\": \"Opener\", \"topic\": \"Hiring\"}\n```\n";
        let p = parse_classification(raw).unwrap();
        assert_eq!(p.position, 0.1);
        assert_eq!(p.category.as_deref(), Some("Opener"));
        assert_eq!(p.topic, "Hiring");
    }

    #[test]
    fn clamps_position_and_defaults_bad_or_missing() {
        let pos = |raw: &str| parse_classification(raw).unwrap().position;
        assert_eq!(pos(r#"{"position": 1.7, "category": "X"}"#), 1.0);
        assert_eq!(pos(r#"{"position": -3, "category": "X"}"#), 0.0);
        // Missing / non-numeric position falls back to mid-arc.
        assert_eq!(pos(r#"{"category": "X"}"#), 0.5);
        assert_eq!(pos(r#"{"position": "high", "category": "X"}"#), 0.5);
    }

    /// The distinction the whole per-row rule turns on: a reply that OMITS the stage field
    /// is malformed, while one that answers it with `""` is the compliant "no clear
    /// conversational role" answer `CLASSIFY_INSTRUCTION` asks for. Collapsing them is what
    /// let the two passes disagree about it in opposite directions.
    #[test]
    fn an_absent_stage_field_is_distinguished_from_an_explicitly_empty_one() {
        assert_eq!(parse_classification(r#"{"position": 0.5}"#).unwrap().category, None);
        // Present but not a string is equally unusable.
        assert_eq!(
            parse_classification(r#"{"position": 0.5, "category": null}"#).unwrap().category,
            None
        );
        // Explicitly empty — an answer, not an omission. Whitespace still counts as answered.
        assert_eq!(
            parse_classification(r#"{"position": 0.5, "category": ""}"#).unwrap().category,
            Some(String::new())
        );
        assert_eq!(
            parse_classification(r#"{"position": 0.5, "category": "  "}"#).unwrap().category,
            Some(String::new())
        );

        assert!(parse_classification("no json here").is_none());
        assert!(parse_classification("").is_none());
    }

    /// The per-row rule, which used to live inlined in two orchestrators behind an
    /// `AppHandle` and a live CLI where nothing could test it.
    #[test]
    fn decide_writes_a_real_stage_and_anchors_it() {
        let d = decide(parse_classification(r#"{"position": 0.2, "category": "Objection", "topic": "Pricing"}"#), &[], &[]);
        let Decision::Write { position, category, topic } = d else {
            panic!("a well-formed reply must be written");
        };
        // Canonical stage: pinned to its anchor, not the model's 0.2.
        assert_eq!(position, ArcPosition::At(0.72));
        assert_eq!(category, "Objection");
        assert_eq!(topic, "Pricing");
    }

    /// An unreadable reply is a failure on both paths — the batch counts it toward the
    /// all-failed check, the per-edit pass drops it. Neither writes anything.
    #[test]
    fn decide_fails_on_an_unreadable_reply_or_a_missing_stage_field() {
        assert!(matches!(decide(parse_classification("not json"), &[], &[]), Decision::Failed(_)));
        assert!(matches!(
            decide(parse_classification(r#"{"position": 0.9}"#), &[], &[]),
            Decision::Failed(_)
        ));
    }

    /// The compliant empty answer: written (so a stale topic-style label can't survive in
    /// the stage field and get re-offered as a stage), never counted as an error, and the
    /// snippet does NOT move on the arc — a reply naming no stage is not evidence about
    /// where the line sits, and the `position` beside it is the 0.5 fallback or model noise.
    #[test]
    fn decide_writes_an_empty_stage_without_relocating_the_snippet() {
        let d = decide(parse_classification(r#"{"position": 0.4, "category": ""}"#), &[], &[]);
        let Decision::Write { position, category, topic } = d else {
            panic!("an explicitly empty stage is an answer, not a failure");
        };
        assert_eq!(category, "");
        assert_eq!(topic, "");
        assert_eq!(position, ArcPosition::Unchanged);
        // Unchanged keeps whatever the row already had, whatever that was.
        assert_eq!(position.resolve(0.58), 0.58);
        assert_eq!(ArcPosition::At(0.72).resolve(0.58), 0.72);
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
            finalize_classification(0.5, "follow up", "", &[], &[]),
            (0.96, "Follow-up".to_string(), String::new())
        );
        // A freeform label keeps its position and snaps to an existing spelling.
        let existing = vec!["Discovery".to_string()];
        assert_eq!(
            finalize_classification(0.33, "discovery", "", &existing, &[]),
            (0.33, "Discovery".to_string(), String::new())
        );
        // Empty stays empty, position untouched.
        assert_eq!(
            finalize_classification(0.4, "", "", &[], &[]),
            (0.4, String::new(), String::new())
        );
    }

    /// The topic is snapped but never anchored: it can't move `position` and it isn't
    /// held to the canonical stage table, because a topic says nothing about arc order.
    #[test]
    fn finalize_snaps_the_topic_without_anchoring_it() {
        let topics = vec!["Security".to_string()];
        // Case/punctuation drift snaps to the existing spelling rather than forking it.
        assert_eq!(
            finalize_classification(0.58, "Engaged", "security", &[], &topics),
            (0.58, "Engaged".to_string(), "Security".to_string())
        );
        // A genuinely new topic is kept as-is, and does NOT shift the anchored position.
        assert_eq!(
            finalize_classification(0.2, "follow up", "Integrations", &[], &topics),
            (0.96, "Follow-up".to_string(), "Integrations".to_string())
        );
        // A stage label must never leak into the topic slot's vocabulary and vice versa —
        // they snap against separate sets.
        let stages = vec!["Objection".to_string()];
        assert_eq!(
            finalize_classification(0.72, "objection", "objection", &stages, &topics),
            (0.72, "Objection".to_string(), "objection".to_string()),
            "the topic must not snap to a STAGE spelling"
        );
    }
}
