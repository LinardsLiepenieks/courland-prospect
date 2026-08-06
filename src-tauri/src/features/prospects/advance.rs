//! Deciding whether a prospect has outgrown the stage they're sitting in.
//!
//! Every stage of the cycle carries a `goal` — what has to become true before a
//! prospect belongs in the next column. Whenever a new message lands in a thread
//! (in EITHER direction — see `messages::repository::StoreOutcome`), this asks
//! the local Claude Code CLI whether that goal is now visibly satisfied. If it
//! is, the verdict is stored as a *suggestion* on the prospect's row and the card
//! grows an accept/dismiss chip.
//!
//! **Nothing here ever moves a prospect.** The stage move happens only when the
//! user accepts, via `commands::accept_stage_suggestion`. That's the whole design
//! premise: there is no move history in this app, so a silent auto-advance would
//! be invisible and unauditable, while a suggestion costs one click to wave off.
//!
//! Shaped after `features::snippets::proposals`, which does the same thing for
//! snippet extraction off the same ingest hook — fire-and-forget, background CLI
//! capacity only, every failure logged and dropped rather than surfaced. A missed
//! analysis is cheap: the next message in the thread runs it again.
//!
//! The stages that opt OUT are as important as the ones that opt in:
//!   - a stage with an empty `goal` has nothing to test, so it never advances;
//!   - the last stage has nowhere to advance to;
//!   - a prospect whose suggestion already points at the same stage is left alone
//!     rather than burning a CLI call to re-derive it.

use tauri::{AppHandle, Emitter, Manager};

use super::repository;
use crate::ai::{self, AdvanceContext, DraftCustomer, DraftMessage, DraftStage, Prompt};
use crate::database::AppState;
use crate::features::messages;
use crate::features::product;
use crate::features::stages;
use crate::ingest::server::PROSPECTS_CHANGED;

/// How many of a thread's most recent messages the verdict is based on. A stage
/// decision turns on how the conversation is going now, not how it opened, and
/// the whole thread would put an unbounded scrape into a CLI argument.
const CONVERSATION_WINDOW: usize = 30;

/// Upper bound on the stored `reason`. The prompt asks for at most 15 words; this
/// is the backstop that keeps a runaway model from writing an essay onto a card.
const MAX_REASON_LEN: usize = 200;

/// Who asked for this verdict, which decides how it competes for the one capped
/// pool of Claude Code processes.
#[derive(Clone, Copy)]
pub(crate) enum Priority {
    /// Triggered by a captured message. Takes a CLI permit only if one is free
    /// right now, so a burst of captures can never make a user-facing draft wait;
    /// a skipped pass costs nothing, since the next message re-runs it.
    Background,
    /// Triggered by the user clicking "re-check" on a card. Queues for a permit
    /// and reports its errors, because somebody is watching this one.
    Foreground,
}

/// Fire the advance check for every prospect whose thread just gained a message,
/// off the request path. Returns immediately; failures are logged, never
/// propagated. A no-op when nothing changed.
pub(crate) fn spawn(app: AppHandle, prospect_ids: Vec<i64>) {
    if prospect_ids.is_empty() {
        return;
    }
    tokio::spawn(async move {
        run(app, prospect_ids).await;
    });
}

/// One analysis pass per prospect. Sequential — the client already caps CLI
/// concurrency app-wide, and this is background work with no latency budget.
/// Emits once at the end if any row changed, so a batch covering ten threads
/// refreshes the board once rather than ten times.
async fn run(app: AppHandle, prospect_ids: Vec<i64>) {
    let mut changed = false;
    for id in prospect_ids {
        match analyze(&app, id, Priority::Background).await {
            Ok(wrote) => changed |= wrote,
            Err(e) => eprintln!("advance: prospect {id}: {e}"),
        }
    }
    if changed {
        let _ = app.emit(PROSPECTS_CHANGED, ());
    }
}

/// Everything one verdict needs, read under a single connection lock.
struct Gathered {
    product_name: String,
    product_description: String,
    customer: Option<crate::features::customers::model::Customer>,
    current_stage: stages::model::Stage,
    next_stage: stages::model::Stage,
    conversation: Vec<messages::repository::StoredMessage>,
    /// The suggestion columns as they stood when this pass read the prospect, used as a
    /// compare-and-swap token when it writes. Two passes over one prospect are ordinary
    /// (nothing serializes them, and up to `MAX_CONCURRENT` run at once), and each takes
    /// up to a minute — so between one pass reading and writing, the user can dismiss a
    /// chip, or a second pass can land a newer verdict. Without this token the stale pass
    /// wins by writing last.
    suggestion_at_gather: (Option<i64>, String),
}

/// One prospect's pass: gather, ask, store. `Ok(true)` means the row CHANGED —
/// a suggestion was written, or an existing one retracted — so the caller knows
/// the board needs refreshing. `Ok(false)` means nothing happened: no goal on
/// the stage, no next stage, an empty thread, no spare CLI capacity, or a "not
/// yet" with no stale chip to withdraw. Errors are returned rather than logged
/// here, so the background caller can log them and the command layer can show
/// them.
pub(crate) async fn analyze(
    app: &AppHandle,
    prospect_id: i64,
    priority: Priority,
) -> Result<bool, String> {
    let gathered = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || gather(&app, prospect_id, priority))
            .await
            .map_err(|e| format!("gather task panicked: {e}"))??
    };
    let Some(gathered) = gathered else {
        // Nothing to judge (no goal, last stage, prospect gone). Not an error.
        return Ok(false);
    };

    let conversation: Vec<DraftMessage> = gathered
        .conversation
        .iter()
        .map(|m| DraftMessage { incoming: m.incoming, body: m.body.clone() })
        .collect();
    // An empty thread can't evidence anything, and the instruction would refuse
    // it anyway — don't spend a CLI call to be told no.
    if conversation.is_empty() {
        return Ok(false);
    }

    let ctx = AdvanceContext {
        product_name: &gathered.product_name,
        product_description: &gathered.product_description,
        customer: gathered.customer.as_ref().map(|c| DraftCustomer {
            name: &c.name,
            who_they_are: &c.who_they_are,
            pain: &c.pain,
            goal: &c.goal,
        }),
        current_stage: DraftStage {
            name: &gathered.current_stage.name,
            goal: &gathered.current_stage.goal,
        },
        next_stage: DraftStage {
            name: &gathered.next_stage.name,
            goal: &gathered.next_stage.goal,
        },
        conversation: &conversation,
    };

    let prompt = Prompt::assess_stage(&ctx);
    let raw = match priority {
        Priority::Background => match ai::client::run_capped_background(prompt).await {
            // No spare CLI capacity — skip. The next message re-runs this.
            Ok(None) => return Ok(false),
            Ok(Some(text)) => text,
            Err(e) => return Err(e),
        },
        Priority::Foreground => ai::client::run_capped(prompt).await?,
    };

    let target = gathered.next_stage.id;
    let from_stage = gathered.current_stage.id;
    // Carried into the write below as a compare-and-swap token — see `Gathered`.
    let suggestion_at_gather = gathered.suggestion_at_gather.clone();

    let verdict = match parse_verdict(&raw) {
        Some(reason) => Verdict::Advance(reason),
        // A clean "not yet" and an answer we couldn't read are the same
        // non-event to the board, but very different things to DEBUG — so the
        // malformed one is logged and the refusal stays quiet. Without this the
        // two are indistinguishable even in stderr, including on the foreground
        // path where someone is watching a card and getting nothing back.
        None if looks_like_a_refusal(&raw) => Verdict::NotYet,
        None => {
            eprintln!(
                "advance: prospect {prospect_id}: unparseable verdict ({} chars): {}",
                raw.len(),
                raw.trim().chars().take(200).collect::<String>()
            );
            // Deliberately NOT `NotYet`: garbage must not retract a suggestion
            // the user can see, any more than it may create one. Fail closed in
            // both directions.
            return Ok(false);
        }
    };

    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        // Re-checked under the lock: the user may have moved this prospect by
        // hand while the CLI was thinking, in which case the verdict is about a
        // stage they've already left. `suggest_stage` refuses a target they're
        // now in; this catches the rest.
        let Some(current) = repository::find(&conn, prospect_id).map_err(|e| e.to_string())?
        else {
            return Ok::<_, String>(false);
        };
        if current.stage_id != Some(from_stage) {
            return Ok(false);
        }
        // The suggestion columns must also still be as this pass found them. The stage
        // check above catches a hand-move but says nothing about the chip itself, and the
        // chip is what this writes — so without this, the two ways a stale pass wins are:
        //   - the user DISMISSED a chip while the CLI was thinking, and an older
        //     affirmative verdict re-writes it, resurrecting something they just cleared
        //     with no new message having arrived; or
        //   - a second, newer pass already wrote its verdict, and this older one
        //     overwrites it — or, on a "not yet", silently retracts a chip the user
        //     explicitly asked for by re-checking.
        // Both are lost updates, and both come from writing on the strength of a read
        // that is up to a minute old. Refusing is right: whoever moved it last saw a
        // fresher thread, and the next captured message re-runs this anyway.
        if (current.suggested_stage_id, current.suggested_reason.as_str())
            != (suggestion_at_gather.0, suggestion_at_gather.1.as_str())
        {
            return Ok(false);
        }
        match verdict {
            Verdict::Advance(reason) => repository::suggest_stage(&conn, prospect_id, target, &reason)
                .map(|updated| updated.is_some())
                .map_err(|e| e.to_string()),
            // The newest read of the thread supersedes an older one, so a
            // refusal retracts the chip it disagrees with. Only reachable from a
            // manual re-check (the background pass short-circuits before the CLI
            // when a suggestion already points here) — and that is exactly the
            // case that needs it: you tightened the stage's goal, re-checked, and
            // the pending chip no longer holds under it.
            Verdict::NotYet if current.suggested_stage_id == Some(target) => {
                repository::clear_suggestion(&conn, prospect_id)
                    .map(|updated| updated.is_some())
                    .map_err(|e| e.to_string())
            }
            Verdict::NotYet => Ok(false),
        }
    })
    .await
    .map_err(|e| format!("store task panicked: {e}"))?
}

/// What the model actually decided, once its output has been read. Separated
/// from "we couldn't read it", which is handled before this point and never
/// reaches the database.
enum Verdict {
    /// Advance, with the one-line justification to show on the card.
    Advance(String),
    /// The current stage's goal is not met yet.
    NotYet,
}

/// Read everything one verdict needs under a single lock. `Ok(None)` means there
/// is legitimately nothing to judge, which is the common case and not an error:
/// the prospect vanished, has no stage, sits in a stage with no goal, or is
/// already in the last stage of the cycle.
fn gather(
    app: &AppHandle,
    prospect_id: i64,
    priority: Priority,
) -> Result<Option<Gathered>, String> {
    let st = app.state::<AppState>();
    let conn = st.conn.lock().map_err(|e| e.to_string())?;

    let Some(prospect) = repository::find(&conn, prospect_id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some(stage_id) = prospect.stage_id else {
        return Ok(None);
    };
    let Some(current_stage) =
        stages::repository::find(&conn, stage_id).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    // A stage with no goal has no exit condition, so there is nothing to test.
    // This is how a user opts a column out of auto-advance: leave its goal blank.
    if current_stage.goal.trim().is_empty() {
        return Ok(None);
    }
    let Some(next_stage) = stages::repository::next_stage(&conn, current_stage.position)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None); // already at the end of the cycle
    };
    // Already suggesting exactly this — re-deriving it would burn a CLI call to
    // reach the same card. A dismissal clears the suggestion, so this doesn't
    // suppress a fresh look after the user waves one off.
    //
    // BACKGROUND ONLY. A manual re-check must always run: the reason to click it
    // is usually that you just rewrote the stage's goal and want to know whether
    // the pending chip still holds under the new one. Skipping there returns an
    // unchanged row that is indistinguishable from a genuine "not yet", so the
    // one case the button exists for would be the one case it did nothing.
    if matches!(priority, Priority::Background)
        && prospect.suggested_stage_id == Some(next_stage.id)
    {
        return Ok(None);
    }

    let product = product::repository::get(&conn).map_err(|e| e.to_string())?;
    let customer = match prospect.customer_id {
        Some(id) => {
            crate::features::customers::repository::find(&conn, id).map_err(|e| e.to_string())?
        }
        None => None,
    };
    let conversation =
        messages::repository::recent_for_prospect(&conn, prospect_id, CONVERSATION_WINDOW)
            .map_err(|e| e.to_string())?;

    Ok(Some(Gathered {
        product_name: product.name,
        product_description: product.description,
        customer,
        current_stage,
        next_stage,
        conversation,
        suggestion_at_gather: (prospect.suggested_stage_id, prospect.suggested_reason.clone()),
    }))
}

/// Pull the verdict out of the model's output. `Some(reason)` means advance;
/// `None` means don't — a "no", a refusal, an unparseable answer, or an
/// AMBIGUOUS one. Every non-answer fails closed, matching the instruction's own
/// bias: a missed advance costs one drag of a card, a wrong one misfiles a live
/// deal.
///
/// Tolerant of a single object wrapped in prose or a code fence, which is the
/// common deviation. It is deliberately NOT tolerant of more than one — see
/// [`advance_objects`] for why that distinction is the whole point of this
/// function. The reason is trimmed and truncated, and a blank one still counts
/// as an advance: the flag is the verdict, the reason only its label, and
/// dropping a real "yes" because the model forgot to explain itself would be the
/// wrong trade.
fn parse_verdict(raw: &str) -> Option<String> {
    // Exactly one verdict, or none. Two means we cannot tell which is the
    // answer, and guessing is how a "no" becomes a "yes".
    let objects = advance_objects(raw);
    let [value] = objects.as_slice() else {
        return None;
    };
    if !value.get("advance")?.as_bool()? {
        return None;
    }
    let reason = value.get("reason").and_then(|r| r.as_str()).unwrap_or_default();
    Some(truncate(reason.trim(), MAX_REASON_LEN))
}

/// Every balanced `{…}` span in `raw` that parses as an object carrying an
/// `advance` key.
///
/// This exists because the obvious parse — first `{` to last `}` — fails OPEN.
/// Given `{"advance": false, …} (schema: {"advance": true})` that slice isn't
/// valid JSON, so the scan slides to the next `{` and lands on the trailing
/// fragment: an explicit refusal read as an approval. A model echoing its own
/// output schema after answering is an entirely ordinary thing to do.
///
/// The scan itself is [`ai::parse::json_objects_where`]. It used to be a second
/// brace scanner written here, and that copy had drifted into the weaker variant:
/// it tracked strings only at `depth > 0` and refused on neither an unterminated
/// string nor an unclosed brace, so a reply that echoed the shape and then got cut
/// off returned the ECHO as the verdict — writing an advance the model refused.
/// Sharing the scan means both guards apply here too, and there is one place left
/// to reason about parity.
///
/// Not `json_object`, though: that refuses when a reply holds more than one object
/// at all, and an unrelated object beside the answer is not an ambiguity about the
/// verdict. `wanted` is what narrows "a candidate" to "answers the question", and
/// the caller still refuses unless exactly one does.
fn advance_objects(raw: &str) -> Vec<serde_json::Map<String, serde_json::Value>> {
    ai::parse::json_objects_where(raw, |fields| fields.contains_key("advance"))
}

/// Whether the model's output is a well-formed "not yet" rather than something
/// we failed to read. Only used to decide whether to LOG — the verdict itself is
/// already settled (and already `false`) by the time this is asked.
fn looks_like_a_refusal(raw: &str) -> bool {
    advance_objects(raw)
        .first()
        .and_then(|v| v.get("advance"))
        .and_then(|a| a.as_bool())
        == Some(false)
}

/// Truncate to `max` characters on a char boundary, appending an ellipsis when
/// anything was cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_positive_verdict_with_its_reason() {
        let reason = parse_verdict(r#"{"advance": true, "reason": "they confirmed Thursday 3pm"}"#);
        assert_eq!(reason.as_deref(), Some("they confirmed Thursday 3pm"));
    }

    #[test]
    fn a_negative_verdict_yields_nothing_to_store() {
        assert!(parse_verdict(r#"{"advance": false, "reason": ""}"#).is_none());
        assert!(parse_verdict(r#"{"advance": false, "reason": "only vague interest"}"#).is_none());
    }

    /// The model wrapping its JSON in prose or a code fence is the single most
    /// common deviation, and it must not cost a real verdict.
    #[test]
    fn tolerates_prose_and_code_fences_around_the_object() {
        let raw = "Here's my assessment:\n```json\n{\"advance\": true, \"reason\": \"invite sent\"}\n```\nHope that helps.";
        assert_eq!(parse_verdict(raw).as_deref(), Some("invite sent"));
    }

    /// Fail closed. Anything we can't read as an explicit `advance: true` is a
    /// "don't" — a garbled response must never file someone into the next stage.
    #[test]
    fn garbage_and_missing_fields_fail_closed() {
        assert!(parse_verdict("").is_none());
        assert!(parse_verdict("I think they're ready!").is_none());
        assert!(parse_verdict("{}").is_none());
        assert!(parse_verdict(r#"{"reason": "they confirmed"}"#).is_none());
        assert!(parse_verdict(r#"{"advance": "yes"}"#).is_none(), "a string is not a bool");
        assert!(parse_verdict(r#"{"advance": true"#).is_none(), "unterminated object");
    }

    /// Regression: the scan used to run from the first `{` to the LAST `}`, so a
    /// model that echoed its output schema after answering had the echo parsed as
    /// the answer — turning an explicit refusal into an approval. Two candidate
    /// verdicts is now an ambiguity, and ambiguity is a refusal.
    #[test]
    fn a_trailing_schema_echo_cannot_flip_a_refusal_into_an_advance() {
        let raw = r#"{"advance": false, "reason": "only vague interest"} (schema: {"advance": true, "reason": "..."})"#;
        assert!(parse_verdict(raw).is_none(), "a second verdict object must refuse, not win");

        // The same shape can't smuggle a "yes" past us either — even when the
        // FIRST object is the affirmative one, two answers means we can't tell.
        let raw = r#"{"advance": true, "reason": "booked"} or maybe {"advance": false}"#;
        assert!(parse_verdict(raw).is_none());
    }

    /// The scan cap must refuse, not return what it found. Padding an affirmative
    /// verdict out to the cap with decoys used to bury the model's real "no" past the
    /// break — leaving one candidate held and a suggestion written off an answer the
    /// model had retracted. Same fail-open the two-verdict rule exists to stop, reached
    /// by exhausting the budget instead of by ambiguity.
    ///
    /// The cap is the shared one now that the scan is shared, so this also pins that
    /// consolidating onto `ai::parse` didn't quietly raise the ceiling.
    #[test]
    fn exhausting_the_span_cap_refuses_rather_than_keeping_a_partial_verdict() {
        let decoys = "{} ".repeat(ai::parse::MAX_SPANS);
        let raw =
            format!(r#"{{"advance": true, "reason": "booked"}} {decoys} {{"advance": false}}"#);
        assert!(
            parse_verdict(&raw).is_none(),
            "a verdict buried past the scan cap must refuse, not return the early one"
        );
    }

    /// A `}` inside the reason must not end the span early — otherwise the
    /// leftover tail parses as a second object and a legitimate verdict is
    /// refused. Escaped quotes likewise.
    #[test]
    fn braces_and_quotes_inside_the_reason_stay_inside_the_span() {
        let raw = r#"{"advance": true, "reason": "they replied \"yes {Thursday}\" to the invite"}"#;
        assert_eq!(
            parse_verdict(raw).as_deref(),
            Some(r#"they replied "yes {Thursday}" to the invite"#)
        );
    }

    /// A reply that echoes the shape and then gets CUT OFF must refuse. The scanner
    /// written here used to return the echo instead: it refused on neither an
    /// unterminated string nor an unclosed brace, so the real answer simply stopped
    /// being a candidate and the ambiguity rule above had nothing left to fire on.
    /// Both of these wrote "Ready for <stage>" — with the echo's fabricated reason —
    /// off a verdict the model had explicitly refused.
    #[test]
    fn a_truncated_reply_after_an_echo_refuses_rather_than_advancing_on_the_echo() {
        let echo = r#"{"advance": true, "reason": "they confirmed Thursday 3pm"}"#;
        for cut in [
            r#"{"advance": false, "reason": "only vague inter"#, // cut mid-string
            r#"{"advance": false, "reason": "only vague interest""#, // cut after a string
            r#"{"advance": false,"#,                             // cut after a comma
            r#"{"advance": false"#,                              // cut after the flag
        ] {
            assert!(
                parse_verdict(&format!("{echo}\nActually, my verdict: {cut}")).is_none(),
                "a reply truncated at `{cut}` must refuse, not advance on the echo"
            );
        }
    }

    /// Objects that aren't verdicts are ignored rather than counted as rivals, so
    /// a model that narrates with some unrelated JSON still gets its answer read.
    #[test]
    fn a_non_verdict_object_alongside_the_answer_is_not_an_ambiguity() {
        let raw = r#"Context: {"stage": "Messaged"}. Verdict: {"advance": true, "reason": "invite sent"}"#;
        assert_eq!(parse_verdict(raw).as_deref(), Some("invite sent"));
    }

    /// The flag is the verdict; the reason is only its label. A model that
    /// answers yes but forgets to explain itself still advances.
    #[test]
    fn a_positive_verdict_survives_a_missing_reason() {
        assert_eq!(parse_verdict(r#"{"advance": true}"#).as_deref(), Some(""));
        assert_eq!(parse_verdict(r#"{"advance": true, "reason": "   "}"#).as_deref(), Some(""));
    }

    #[test]
    fn a_runaway_reason_is_truncated_to_fit_a_card() {
        let long = "x".repeat(500);
        let raw = format!(r#"{{"advance": true, "reason": "{long}"}}"#);
        let reason = parse_verdict(&raw).unwrap();
        assert_eq!(reason.chars().count(), MAX_REASON_LEN);
        assert!(reason.ends_with('…'));
    }

    #[test]
    fn truncate_leaves_short_strings_and_multibyte_text_intact() {
        assert_eq!(truncate("short", 10), "short");
        // Cutting must land on a char boundary, not mid-codepoint.
        let cut = truncate("üüüüü", 3);
        assert_eq!(cut.chars().count(), 3);
        assert!(cut.ends_with('…'));
    }
}

