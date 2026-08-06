//! Proposing snippets from a message the user just sent.
//!
//! When the Chrome extension captures a genuinely-new outgoing message for a
//! prospect (see `features::messages::store_batch`), the ingest server hands the
//! new messages here. For each prospect we ask the local Claude Code CLI to extract
//! reusable material the message contains that isn't already a snippet, and store
//! each as a `proposed` snippet in the library — shown in the editor in a distinct
//! color, awaiting the user's approve/reject. Proposals never compose a draft until
//! approved.
//!
//! Messages are still grouped per prospect so each pass sees one coherent thread,
//! but proposals land in the one shared library regardless of who they came from —
//! and every prospect now yields proposals, including ones with no customer profile
//! assigned (there is no per-scope library left to be missing).
//!
//! This runs fire-and-forget off the ingest response (`spawn`): a missed proposal
//! is logged and dropped, never surfaced as an error — the same phrase re-proposes
//! next time the user sends something new.
//!
//! Two hard guarantees are enforced in code, not left to the model:
//!   - **Grounded in what was sent** — a proposal's content must appear
//!     (whitespace-normalized) in the message the user actually sent, or it's
//!     discarded. A proposal may carry BLANKS (`[first name]`) standing in for the
//!     one detail that was welded to a single person, in which case every word
//!     around them must still be verbatim and in order — see `placeholder`, which
//!     owns that syntax and its shape rules.
//!   - **No exact duplicates** — a proposal whose content already exists in the
//!     library (any status) is skipped; the dedup read and the insert share one
//!     connection lock, so concurrent passes can't both insert the same text
//!     ([`run_one`]). The key sees past a blank's wording, so the same line doesn't
//!     re-land each time the model names its blank differently.
//!
//! Between extraction and insert sits a second, best-effort LLM pass — the reviewer
//! ([`review_candidates`]). `propose` is generative and errs toward proposing; the
//! reviewer gates each candidate on the axes the generator is weakest at:
//! reusability (a line that only makes sense in one conversation is rejected),
//! *semantic* duplication (a line an existing snippet already conveys, even if worded
//! differently — which the exact-match dedup above can't catch), and whether a blank
//! is one anything could actually fill. The reviewer is an
//! enhancement, not a guarantee: no spare CLI capacity skips the whole pass, and so does a
//! verdict that can't be read. Either way the phrase re-proposes on the next send.
//!
//! Note "skip", not "accept un-reviewed". An unreadable verdict used to fall back to the
//! un-reviewed set on the reasoning that a reviewer hiccup shouldn't discard good material —
//! but this gate's `None` means "no verdicts", and reading that as "keep them all" let
//! through strictly MORE than having no reviewer at all.

use std::collections::HashMap;

use tauri::{AppHandle, Emitter, Manager};

use super::placeholder;
use super::repository;
use super::SNIPPETS_CHANGED;
use crate::ai::{self, Prompt, ProposeContext, ReviewContext};
use crate::database::AppState;
use crate::features::messages::repository::NewOutgoing;
use crate::features::product;
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
    for (_prospect_id, messages) in by_prospect {
        run_one(&app, &messages).await;
    }
}

/// One thread's pass: gather the product context + the existing library, ask Claude
/// for proposals, then verbatim-check, dedup, and insert the survivors. All fallible
/// steps log and return rather than propagate — this is fire-and-forget.
async fn run_one(app: &AppHandle, messages: &[String]) {
    // Gather everything the prompt needs under one lock: what's being sold, and
    // every snippet already in the library (all statuses, so the model doesn't
    // re-propose something already awaiting review).
    let gathered = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let product = product::repository::get(&conn).map_err(|e| e.to_string())?;
            let existing: Vec<(String, String)> = repository::list(&conn)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|s| !s.content.trim().is_empty())
                .map(|s| (s.name, s.content))
                .collect();
            Ok::<_, String>((product.name, product.description, existing))
        })
        .await
    };

    let (product_name, product_description, existing) = match gathered {
        Ok(Ok(v)) => v,
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
        product_name: &product_name,
        product_description: &product_description,
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

    let candidates = select_candidates(&raw, messages);
    if candidates.is_empty() {
        return;
    }

    // Reviewer gate: a second LLM pass judges each candidate against the product and
    // the existing library, rejecting one-off (conversation-specific) lines, semantic
    // duplicates the exact-match dedup below can't catch, and the two ways a blank
    // goes wrong — asking for a detail nothing could ever fill, or hollowing the line
    // out into a form. `None` = no spare CLI capacity, so skip the whole pass and let
    // the phrase re-propose next send.
    let candidates =
        match review_candidates(&product_name, &product_description, &existing, candidates).await {
            Some(kept) => kept,
            None => return,
        };
    if candidates.is_empty() {
        return; // reviewer rejected everything — nothing to insert
    }

    // Dedup against the library's existing contents and insert — one lock, so a
    // concurrent pass can't slip an identical proposal in between our check and
    // insert. Also dedups within this batch itself.
    let app2 = app.clone();
    let inserted = tokio::task::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let mut seen: Vec<String> = repository::dedup_contents(&conn)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|c| placeholder::dedup_key(c))
            .collect();
        let mut count = 0usize;
        for (name, content) in candidates {
            if count >= MAX_PROPOSALS {
                break; // cap the accepted (deduped) proposals, not the raw candidates
            }
            let key = placeholder::dedup_key(&content);
            if seen.contains(&key) {
                continue; // already approved, already proposed, or a repeat in this batch
            }
            repository::create_proposed(&conn, &name, &content).map_err(|e| e.to_string())?;
            seen.push(key);
            count += 1;
        }
        Ok::<_, String>(count)
    })
    .await;

    match inserted {
        Ok(Ok(0)) => {} // everything was a duplicate — nothing to announce
        Ok(Ok(_)) => {
            // Nudge an open editor to reload and show the proposals.
            let _ = app.emit(SNIPPETS_CHANGED, ());
        }
        Ok(Err(e)) => eprintln!("snippets: propose insert failed: {e}"),
        Err(e) => eprintln!("snippets: propose insert task panicked: {e}"),
    }
}

/// The reviewer gate: run the extracted `candidates` through a second LLM pass and
/// return the survivors.
///
/// `Some(kept)` is a decision, including `Some(vec![])` when everything was rejected.
/// `None` means **skip the pass** — no verdict was obtained — and the caller drops the
/// candidates rather than admitting them, so the phrase re-proposes on the next send. Two
/// things produce it: no spare CLI capacity, and a reply whose verdict couldn't be read
/// (unparseable, or ambiguous — `ai::parse` refuses a reply carrying two candidate arrays).
///
/// The one case that does NOT skip is a generation error, which still degrades to
/// `Some(candidates)`: the pass ran and failed on its own terms, and dropping material over
/// a transient CLI failure would make the reviewer worse than not having one.
///
/// Do not "restore" the un-reviewed fallback for the unreadable case. This gate's job is to
/// remove candidates, so treating "no verdicts" as "keep them all" inverts it and lets
/// through strictly more than having no reviewer at all.
async fn review_candidates(
    product_name: &str,
    product_description: &str,
    existing: &[(String, String)],
    candidates: Vec<(String, String)>,
) -> Option<Vec<(String, String)>> {
    let ctx = ReviewContext {
        product_name,
        product_description,
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
        // An unreadable reply means the verdicts are UNKNOWN, and "unknown" must not
        // resolve to "keep everything" in a gate whose entire job is to reject. Skipping
        // (like the no-capacity path) is the safe reading: nothing is lost, because the
        // same phrase is re-proposed and re-reviewed on the user's next send.
        //
        // This used to degrade to `Some(candidates)`, which was tolerable while an
        // unreadable reply meant "no JSON at all". It stopped being tolerable once the
        // shared parser began refusing AMBIGUOUS replies too: a reviewer that answered
        // "reject both" and then echoed the instruction's own example now lands here, and
        // degrading would let through strictly more than the old fail-open did.
        None => {
            eprintln!("snippets: propose review verdicts unreadable; skipping the pass");
            None
        }
    }
}

/// Turn Claude's raw reply into the candidate `(name, content)` pairs worth
/// reviewing: parsed out of the JSON, within bounds, well-formed, and grounded in
/// what was actually sent — the whole content verbatim, or its literal text verbatim
/// around at most a couple of blanks (see `placeholder`).
///
/// Note the content that comes back is `placeholder::parse`'s canonical form, not
/// the model's raw string, so what gets stored is exactly what was checked.
///
/// The MAX_PROPOSALS cap is deliberately NOT applied here — it belongs over the
/// *deduped* set, so a run where several candidates already exist in the library
/// doesn't crowd out a genuinely-new one.
fn select_candidates(raw: &str, messages: &[String]) -> Vec<(String, String)> {
    parse_proposals(raw)
        .into_iter()
        .filter(|(_, content)| content.chars().count() <= MAX_TEXT_LEN)
        .filter_map(|(name, content)| {
            let template = placeholder::parse(&content)?;
            template
                .is_grounded_in(messages)
                .then(|| (bound_name(&name), template.content))
        })
        .collect()
}

/// Trim a proposal name to the shared bound (proposals arrive fully formed, so
/// unlike the command layer we clamp rather than reject — a long name shouldn't
/// sink an otherwise-good snippet).
fn bound_name(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_LEN).collect()
}

/// Parse Claude's reply into `(name, content)` pairs. Locating the JSON (past any
/// prose or ```` ```json ```` fences) is [`ai::parse::json_array`]'s job; this reads
/// the elements. Per element it pulls the `content` string (required, non-blank) and
/// `name` string (optional); a malformed element is skipped, not fatal.
fn parse_proposals(raw: &str) -> Vec<(String, String)> {
    let Some(items) = ai::parse::json_array(raw) else {
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

/// Parse the reviewer's reply into one keep/reject verdict per candidate: find the
/// JSON array (via [`ai::parse::json_array`]), then read each element's `index`
/// (1-based) and `keep` flag. Returns a `Vec<bool>` of length `n` — a candidate whose
/// index never appears, or appears without an affirmative `keep`, defaults to REJECT
/// (the strict side: a missed snippet is fine, clutter is not).
///
/// Returns `None` when no array was located — which covers a reply with no JSON in it AND a
/// reply `ai::parse::json_array` REFUSED as ambiguous (two parseable arrays, e.g. an echoed
/// shape beside the answer). That lets the caller tell a genuine "reject some" verdict from
/// a reply it couldn't read; `review_candidates` skips the pass on `None`.
fn parse_review(raw: &str, n: usize) -> Option<Vec<bool>> {
    let items = ai::parse::json_array(raw)?;
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

    fn sent(message: &str) -> Vec<String> {
        vec![message.to_string()]
    }

    #[test]
    fn selects_a_verbatim_span_and_drops_a_paraphrase() {
        let messages = sent("Hi Ada, we are SOC2 compliant and ship weekly.");
        let raw = r#"[
            {"name": "SOC2", "content": "we are SOC2 compliant"},
            {"name": "Made up", "content": "we hold SOC2 certification"}
        ]"#;
        assert_eq!(
            select_candidates(raw, &messages),
            vec![("SOC2".to_string(), "we are SOC2 compliant".to_string())]
        );
    }

    /// The point of the blank: a line whose only problem was one person's details
    /// now survives, canonicalized, instead of being thrown away.
    #[test]
    fn selects_a_blanked_span_and_stores_the_canonical_label() {
        let messages =
            sent("Since you're running ops at Acme, follow-ups slip once you pass 50 threads.");
        let raw = r#"[{
            "name": "Follow-ups slip",
            "content": "Since you're running [THEIR KIND OF TEAM], follow-ups slip once you pass 50 threads."
        }]"#;
        assert_eq!(
            select_candidates(raw, &messages),
            vec![(
                "Follow-ups slip".to_string(),
                "Since you're running [their kind of team], follow-ups slip once you pass 50 \
                 threads."
                    .to_string()
            )]
        );
    }

    /// A blank is not a licence to rewrite: the words around it are still held to
    /// the verbatim rule, and a skeleton is still refused.
    #[test]
    fn drops_a_blanked_candidate_that_reworded_or_hollowed_out_the_line() {
        let messages =
            sent("Since you're running ops at Acme, follow-ups slip once you pass 50 threads.");
        let raw = r#"[
            {"name": "Reworded", "content": "Because you run [their kind of team], follow-ups slip once you pass 50 threads."},
            {"name": "Skeleton", "content": "[the situation], [the problem]"}
        ]"#;
        assert!(select_candidates(raw, &messages).is_empty());
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
