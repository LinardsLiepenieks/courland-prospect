//! Proposing snippets from a message the user just sent.
//!
//! When the Chrome extension captures a genuinely-new outgoing message for a
//! prospect (see `features::messages::store_batch`), the ingest server hands the
//! new messages here. For each prospect we ask the local Claude Code CLI to extract
//! reusable pitch material the message contains that isn't already a snippet, and
//! store each as a `proposed` snippet on that prospect's pitch — shown in the editor
//! in a distinct color, awaiting the user's approve/reject. Proposals never compose
//! a draft until approved.
//!
//! This runs fire-and-forget off the ingest response (`spawn`): a missed proposal
//! is logged and dropped, never surfaced as an error — the same phrase re-proposes
//! next time the user sends something new.
//!
//! Two hard guarantees are enforced in code, not left to the model:
//!   - **Verbatim** — a proposal's content must appear (whitespace-normalized) in the
//!     message the user actually sent, or it's discarded ([`is_verbatim`]).
//!   - **No exact duplicates** — a proposal whose content already exists on the pitch
//!     (any status) is skipped; the dedup read and the insert share one connection
//!     lock, so concurrent passes can't both insert the same text ([`run_one`]).
//!
//! Between extraction and insert sits a second, best-effort LLM pass — the reviewer
//! ([`review_candidates`]). `propose` is generative and errs toward proposing; the
//! reviewer gates each candidate on the two axes the generator is weakest at:
//! reusability (a line that only makes sense in one conversation is rejected) and
//! *semantic* duplication (a line an existing snippet already conveys, even if worded
//! differently — which the exact-match dedup above can't catch). The reviewer is an
//! enhancement, not a guarantee: no spare CLI capacity skips the whole pass (the
//! phrase re-proposes next send), and an error or unparseable verdict degrades to the
//! un-reviewed set rather than dropping good candidates.

use std::collections::HashMap;

use tauri::{AppHandle, Emitter, Manager};

use super::repository;
use super::SNIPPETS_CHANGED;
use crate::ai::{self, Prompt, ProposeContext, ReviewContext};
use crate::database::AppState;
use crate::features::messages::repository::NewOutgoing;
use crate::features::{pitches, prospects};
use crate::util::{MAX_NAME_LEN, MAX_TEXT_LEN};

/// Upper bound on proposals accepted from a single analysis pass — keeps a confused
/// or runaway model from flooding the queue.
const MAX_PROPOSALS: usize = 5;

/// Fire the snippet-proposal pass for a batch of new outgoing messages, off the
/// request path. Returns immediately; the work runs on the tokio runtime and any
/// failure is logged, never propagated. A no-op when there's nothing new.
pub(crate) fn spawn(app: AppHandle, new_outgoing: Vec<NewOutgoing>) {
    if new_outgoing.is_empty() {
        return;
    }
    tokio::spawn(async move {
        run(app, new_outgoing).await;
    });
}

/// Group the new outgoing messages by prospect and run one analysis pass each.
/// Sequential — `run_capped` already caps CLI concurrency app-wide, and this is
/// background work with no latency budget.
async fn run(app: AppHandle, new_outgoing: Vec<NewOutgoing>) {
    let mut by_prospect: HashMap<i64, Vec<String>> = HashMap::new();
    for m in new_outgoing {
        by_prospect.entry(m.prospect_id).or_default().push(m.body);
    }
    for (prospect_id, messages) in by_prospect {
        run_one(&app, prospect_id, &messages).await;
    }
}

/// One prospect's pass: gather the pitch context + existing snippets, ask Claude for
/// proposals, then verbatim-check, dedup, and insert the survivors. All fallible
/// steps log and return rather than propagate — this is fire-and-forget.
async fn run_one(app: &AppHandle, prospect_id: i64, messages: &[String]) {
    // Gather everything the prompt needs under one lock. `None` = no pitch to
    // propose into (a prospect with no pitch has no snippet library) → skip.
    let gathered = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let Some(pitch_id) = prospects::repository::pitch_id(&conn, prospect_id)
                .map_err(|e| e.to_string())?
            else {
                return Ok::<_, String>(None);
            };
            let pitch = pitches::repository::get(&conn, pitch_id).map_err(|e| e.to_string())?;
            let existing: Vec<(String, String)> = repository::list(&conn, Some(pitch_id))
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|s| !s.content.trim().is_empty())
                .map(|s| (s.name, s.content))
                .collect();
            Ok(Some((pitch_id, pitch.name, pitch.skill, existing)))
        })
        .await
    };

    let (pitch_id, pitch_name, pitch_skill, existing) = match gathered {
        Ok(Ok(Some(v))) => v,
        Ok(Ok(None)) => return, // prospect has no pitch — nothing to propose into
        Ok(Err(e)) => {
            eprintln!("snippets: propose gather failed: {e}");
            return;
        }
        Err(e) => {
            eprintln!("snippets: propose gather task panicked: {e}");
            return;
        }
    };

    let ctx = ProposeContext {
        pitch_name: &pitch_name,
        pitch_skill: &pitch_skill,
        existing_snippets: &existing,
        messages,
    };
    let raw = match ai::client::run_capped_background(Prompt::propose_snippets(&ctx)).await {
        Ok(Some(t)) => t,
        // No spare CLI capacity — interactive work has the permits. Skip; the phrase
        // re-proposes on the user's next send.
        Ok(None) => return,
        // AI unavailable / errored — a proposal is best-effort, so just drop it.
        Err(e) => {
            eprintln!("snippets: propose generation failed: {e}");
            return;
        }
    };

    // Keep only proposals that are genuinely verbatim from what was sent and within
    // bounds. The MAX_PROPOSALS cap is applied later, over the *deduped* set, so a
    // run where several candidates already exist doesn't crowd out a genuinely-new
    // one.
    let candidates: Vec<(String, String)> = parse_proposals(&raw)
        .into_iter()
        .filter(|(_, content)| is_verbatim(messages, content))
        .filter(|(_, content)| content.chars().count() <= MAX_TEXT_LEN)
        .map(|(name, content)| (bound_name(&name), content))
        .collect();
    if candidates.is_empty() {
        return;
    }

    // Reviewer gate: a second LLM pass judges each candidate against the pitch and the
    // existing library, rejecting one-off (conversation-specific) lines and semantic
    // duplicates the exact-match dedup below can't catch. `None` = no spare CLI
    // capacity, so skip the whole pass and let the phrase re-propose on the next send.
    let candidates = match review_candidates(&pitch_name, &pitch_skill, &existing, candidates).await
    {
        Some(kept) => kept,
        None => return,
    };
    if candidates.is_empty() {
        return; // reviewer rejected everything — nothing to insert
    }

    // Dedup against the pitch's existing contents and insert — one lock, so a
    // concurrent pass can't slip an identical proposal in between our check and
    // insert. Also dedups within this batch itself.
    let app2 = app.clone();
    let inserted = tokio::task::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let mut seen: Vec<String> = repository::dedup_contents(&conn, pitch_id)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|c| normalize_key(c))
            .collect();
        let mut count = 0usize;
        for (name, content) in candidates {
            if count >= MAX_PROPOSALS {
                break; // cap the accepted (deduped) proposals, not the raw candidates
            }
            let key = normalize_key(&content);
            if seen.contains(&key) {
                continue; // already approved, already proposed, or a repeat in this batch
            }
            repository::create_proposed(&conn, pitch_id, &name, &content)
                .map_err(|e| e.to_string())?;
            seen.push(key);
            count += 1;
        }
        Ok::<_, String>(count)
    })
    .await;

    match inserted {
        Ok(Ok(0)) => {} // everything was a duplicate — nothing to announce
        Ok(Ok(_)) => {
            // Nudge an open editor for this pitch to reload and show the proposals.
            // Payload is the scope (`Some(pitch_id)` — proposals are always pitched).
            let _ = app.emit(SNIPPETS_CHANGED, Some(pitch_id));
        }
        Ok(Err(e)) => eprintln!("snippets: propose insert failed: {e}"),
        Err(e) => eprintln!("snippets: propose insert task panicked: {e}"),
    }
}

/// The reviewer gate: run the extracted `candidates` through a second LLM pass and
/// return the survivors. Returns `Some(kept)` on a decision (including `Some(vec![])`
/// when everything was rejected), and `None` ONLY when there's no spare CLI capacity —
/// the caller treats that as "skip, retry next send". A generation error or an
/// unparseable verdict degrades to `Some(candidates)` (the un-reviewed set), so a
/// reviewer hiccup never silently discards genuinely-new material.
async fn review_candidates(
    pitch_name: &str,
    pitch_skill: &str,
    existing: &[(String, String)],
    candidates: Vec<(String, String)>,
) -> Option<Vec<(String, String)>> {
    let ctx = ReviewContext {
        pitch_name,
        pitch_skill,
        existing_snippets: existing,
        candidates: &candidates,
    };
    let raw = match ai::client::run_capped_background(Prompt::review_proposals(&ctx)).await {
        Ok(Some(t)) => t,
        Ok(None) => return None, // no capacity — caller skips and retries on next send
        Err(e) => {
            eprintln!("snippets: propose review generation failed: {e}");
            return Some(candidates); // degrade: keep the verbatim+deduped set un-reviewed
        }
    };
    match parse_review(&raw, candidates.len()) {
        Some(verdicts) => Some(
            candidates
                .into_iter()
                .zip(verdicts)
                .filter_map(|(c, keep)| keep.then_some(c))
                .collect(),
        ),
        None => {
            eprintln!("snippets: propose review returned no parseable verdicts; keeping candidates");
            Some(candidates) // degrade rather than drop
        }
    }
}

/// Trim a proposal name to the shared bound (proposals arrive fully formed, so
/// unlike the command layer we clamp rather than reject — a long name shouldn't
/// sink an otherwise-good snippet).
fn bound_name(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_LEN).collect()
}

/// How many candidate `[` starts `parse_proposals` will try before giving up —
/// bounds the work on pathological/adversarial output riddled with brackets.
const MAX_JSON_STARTS: usize = 32;

/// Parse Claude's reply into `(name, content)` pairs, tolerating prose or
/// ```` ```json ```` fences around the JSON. Each `[` (leftmost first, so the
/// outermost array wins) is tried as the array start, paired with the last `]`, and
/// the first slice that parses as a JSON array is used — a plain first-`[`..last-`]`
/// mis-slices and drops an otherwise-good pass when the model prepends chatter that
/// itself contains stray brackets (e.g. `Here you go [note]: [{...}]`). Per element
/// it pulls the `content` string (required, non-blank) and `name` string (optional);
/// a malformed element is skipped, not fatal.
fn parse_proposals(raw: &str) -> Vec<(String, String)> {
    let Some(last) = raw.rfind(']') else {
        return Vec::new();
    };
    let items = raw
        .match_indices('[')
        .take(MAX_JSON_STARTS)
        .filter(|&(a, _)| a <= last)
        .find_map(
            |(a, _)| match serde_json::from_str::<serde_json::Value>(&raw[a..=last]) {
                Ok(serde_json::Value::Array(items)) => Some(items),
                _ => None,
            },
        );
    let Some(items) = items else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in items {
        let content = it.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let name = it.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        out.push((name.to_string(), content.to_string()));
    }
    out
}

/// Parse the reviewer's reply into one keep/reject verdict per candidate. Tolerant
/// like [`parse_proposals`]: it finds the outermost JSON array (allowing surrounding
/// prose or ```` ```json ```` fences), then reads each element's `index` (1-based) and
/// `keep` flag. Returns a `Vec<bool>` of length `n` — a candidate whose index never
/// appears, or appears without an affirmative `keep`, defaults to REJECT (the strict
/// side: a missed snippet is fine, clutter is not). Returns `None` only when no JSON
/// array is found at all, letting the caller distinguish a genuine "reject some"
/// verdict from an unparseable reply and degrade accordingly.
fn parse_review(raw: &str, n: usize) -> Option<Vec<bool>> {
    let last = raw.rfind(']')?;
    let items = raw
        .match_indices('[')
        .take(MAX_JSON_STARTS)
        .filter(|&(a, _)| a <= last)
        .find_map(
            |(a, _)| match serde_json::from_str::<serde_json::Value>(&raw[a..=last]) {
                Ok(serde_json::Value::Array(items)) => Some(items),
                _ => None,
            },
        )?;
    let mut verdicts = vec![false; n];
    for it in items {
        let Some(idx) = it.get("index").and_then(|v| v.as_i64()) else {
            continue;
        };
        if idx >= 1 && (idx as usize) <= n {
            verdicts[idx as usize - 1] = it.get("keep").and_then(|v| v.as_bool()).unwrap_or(false);
        }
    }
    Some(verdicts)
}

/// Collapse a string to trimmed, single-spaced form so a whitespace-only difference
/// between what the model returned and what was scraped doesn't defeat the checks.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A dedup key: whitespace-normalized + lowercased, so "We ship weekly" and "we
/// ship  weekly" collapse to the same phrase.
fn normalize_key(s: &str) -> String {
    normalize_ws(s).to_lowercase()
}

/// Whether `content` actually appears (whitespace-normalized) in one of the messages
/// the user sent — the hard verbatim guarantee. A model that paraphrases or invents
/// fails this and its proposal is dropped.
fn is_verbatim(messages: &[String], content: &str) -> bool {
    let needle = normalize_ws(content);
    if needle.is_empty() {
        return false;
    }
    messages.iter().any(|m| normalize_ws(m).contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_json_array() {
        let raw = r#"[{"name": "Proof", "content": "we are SOC2 compliant"}]"#;
        let out = parse_proposals(raw);
        assert_eq!(out, vec![("Proof".to_string(), "we are SOC2 compliant".to_string())]);
    }

    #[test]
    fn parses_array_wrapped_in_prose_or_fences() {
        let raw = "Sure! Here you go:\n```json\n[{\"name\":\"A\",\"content\":\"ship weekly\"}]\n```\nHope that helps.";
        let out = parse_proposals(raw);
        assert_eq!(out, vec![("A".to_string(), "ship weekly".to_string())]);
    }

    #[test]
    fn parses_json_array_after_prose_with_stray_brackets() {
        // A plain first-'['..last-']' would slice from the '[' in "[note]" and fail.
        let raw = "Here you go [note]: [{\"name\":\"A\",\"content\":\"ship weekly\"}]";
        assert_eq!(parse_proposals(raw), vec![("A".to_string(), "ship weekly".to_string())]);
    }

    #[test]
    fn empty_array_and_garbage_yield_nothing() {
        assert!(parse_proposals("[]").is_empty());
        assert!(parse_proposals("no json here").is_empty());
        assert!(parse_proposals("").is_empty());
    }

    #[test]
    fn skips_elements_missing_or_blank_content_but_keeps_the_rest() {
        let raw = r#"[
            {"name": "Good", "content": "we ship weekly"},
            {"name": "NoContent"},
            {"name": "Blank", "content": "   "},
            {"content": "nameless is fine"}
        ]"#;
        let out = parse_proposals(raw);
        assert_eq!(
            out,
            vec![
                ("Good".to_string(), "we ship weekly".to_string()),
                ("".to_string(), "nameless is fine".to_string()),
            ]
        );
    }

    #[test]
    fn verbatim_accepts_a_span_present_in_the_message() {
        let messages = ["Hi Ada, we are SOC2 compliant and ship weekly.".to_string()];
        assert!(is_verbatim(&messages, "we are SOC2 compliant"));
        // Whitespace differences don't matter.
        assert!(is_verbatim(&messages, "we are  SOC2   compliant"));
    }

    #[test]
    fn verbatim_rejects_a_paraphrase_or_invention() {
        let messages = ["Hi Ada, we are SOC2 compliant.".to_string()];
        assert!(!is_verbatim(&messages, "we hold SOC2 certification")); // paraphrase
        assert!(!is_verbatim(&messages, "we raised a seed round")); // invented
        assert!(!is_verbatim(&messages, "")); // empty never matches
    }

    #[test]
    fn normalize_key_collapses_whitespace_and_case() {
        assert_eq!(normalize_key("We  Ship\nWeekly"), normalize_key("we ship weekly"));
    }

    #[test]
    fn review_verdicts_keep_and_reject_by_index() {
        let raw = r#"[
            {"index": 1, "keep": true, "reason": "new proof point"},
            {"index": 2, "keep": false, "reason": "duplicate"},
            {"index": 3, "keep": true, "reason": "reusable"}
        ]"#;
        assert_eq!(parse_review(raw, 3), Some(vec![true, false, true]));
    }

    #[test]
    fn review_tolerates_prose_and_fences() {
        let raw = "Sure:\n```json\n[{\"index\":1,\"keep\":true}]\n```\n";
        assert_eq!(parse_review(raw, 1), Some(vec![true]));
    }

    #[test]
    fn review_defaults_missing_index_or_keep_to_reject() {
        // Candidate 2 never appears, and candidate 1 has no `keep` field — both reject.
        assert_eq!(parse_review(r#"[{"index": 1}]"#, 2), Some(vec![false, false]));
        // An out-of-range index is ignored, not a panic.
        assert_eq!(parse_review(r#"[{"index": 5, "keep": true}]"#, 2), Some(vec![false, false]));
        // An empty array is a valid "reject everything" verdict.
        assert_eq!(parse_review("[]", 2), Some(vec![false, false]));
    }

    #[test]
    fn review_returns_none_only_when_unparseable() {
        // No JSON array at all → None, so the caller degrades to keeping candidates.
        assert!(parse_review("no json here", 2).is_none());
        assert!(parse_review("", 2).is_none());
    }
}
