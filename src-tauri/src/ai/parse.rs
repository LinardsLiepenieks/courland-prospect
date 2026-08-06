//! Locating the JSON value in a Claude reply — and refusing when there's more than one.
//!
//! Every machine-parsed prompt here ends by insisting on "a bare JSON array (or object)
//! and nothing else", and models mostly comply — but not reliably enough to hand the raw
//! output to `serde_json`. A reply arrives with a sentence of preamble, wrapped in
//! ```` ```json ```` fences, or both. So before any parse can read fields it has to
//! *find* the JSON, and that finding is the same problem every time. It used to be
//! solved three times over — in the snippet propose parser, its reviewer, and the
//! classifier — which is how it came to carry the same flaw in every copy.
//!
//! # The flaw, and why this scans balanced spans
//!
//! The obvious approach — pair each opening bracket with the *last* closing one and take
//! the first slice that parses — **fails open on an echo**. When a model answers and then
//! restates the shape it was asked for ("`[]` — for example: `[{…}]`"), the slice from
//! the real answer to the final bracket is invalid JSON, the search slides forward, and
//! the *example* becomes the only candidate that parses. The reply then reads as the
//! opposite of what the model actually said: a refusal becomes an assertion, an empty
//! result becomes a fabricated one. Because the echoed text is usually lifted verbatim
//! from the instruction's own `Example output:` line, this is not a rare shape.
//!
//! Two fixes that look sufficient are not, and were tested: preferring the *last*
//! parseable slice returns the same echo, and refusing when more than one *slice* parses
//! sees only one candidate. Both miss because the slide algorithm only ever produces one
//! parseable slice in these cases.
//!
//! So this scans **balanced spans** instead — the complete, top-level `[…]`/`{…}`
//! structures actually present in the text — and then applies the rule that makes the
//! difference: **exactly one span may parse as the wanted shape, or nothing is found.**
//! An echo yields two parseable spans and is refused. A bracket in the preamble
//! (`Here you go [note]: [{…}]`) yields two spans of which only one is JSON, so it still
//! works. Fences still work. `[]` still reads as the meaningful empty answer.
//!
//! # What this rule cannot do
//!
//! Counting parseable spans arbitrates between an echo and an answer **only when both
//! parse**. Every way the real answer fails to become a candidate leaves the echo standing
//! alone, the rule silent for want of a rival, and the fabricated example returned as the
//! answer. Two of those ways are closed here, in [`balanced_spans`]: a reply cut off
//! mid-string, and one cut off mid-bracket. Both refuse the whole scan.
//!
//! One is NOT closed here and cannot be: a real answer that is balanced but *unparseable*
//! — one unescaped quote in a prose field is enough — is skipped by [`sole`] as "not the
//! shape being looked for", indistinguishable from a preamble bracket. Refusing on it
//! would mean refusing `Here you go [note]: [{…}]` too.
//!
//! So the echo problem is not fully decidable from the reply, and the real fix lives
//! upstream: a machine-read instruction must print an unparseable `<…>` **shape**, never a
//! valid example, so that an echo can never be a candidate in the first place. That rule
//! and its regression pin live in [`crate::ai::prompt`]. Treat the two guards below as
//! defence in depth, not as the defence.
//!
//! Refusing is the right failure for every caller here: each treats "not found" as a
//! dropped pass that re-runs, which is strictly better than acting on text the model
//! didn't assert.
//!
//! The rule is applied to arrays as well as objects. For objects it restores what the code
//! did before the parsers were consolidated (a single strict attempt), so it costs nothing.
//! For arrays it is new strictness, and **a refusal here is not automatically the safe
//! answer** — that depends entirely on what the caller does with `None`, and the two array
//! callers in `snippets::proposals` face opposite ways:
//!
//!   - `parse_proposals` fails **closed** — `None` yields no candidates, and the pass is
//!     dropped. Refusing costs nothing; the phrase re-proposes on the next send.
//!   - `parse_review` fails **open** — it gates candidates, so "no verdicts" must not be
//!     read as "keep them all". Its caller therefore treats a refusal as *skip the pass*,
//!     not as *accept un-reviewed*. Getting that wrong would make this rule let through
//!     strictly more than the fail-open it replaced.
//!
//! So one rule for both shapes is right, but only because each caller's `None` handling was
//! checked against it. A stricter parser is safe only where "unknown" is handled as unknown.
//!
//! Deliberately lenient about what surrounds the JSON, and deliberately not lenient
//! about the JSON itself: malformed content is never repaired, only not found.

use serde_json::{Map, Value};

/// How many balanced spans to scan before giving up. Bounds the work on pathological or
/// adversarial output riddled with brackets.
///
/// Exceeding it **refuses** rather than returning what was found so far. Returning a
/// partial result would reintroduce the fail-open this module exists to prevent: a reply
/// whose real answer sits past the cap would be judged on its first few spans alone.
pub(crate) const MAX_SPANS: usize = 32;

/// The single JSON array in `raw`, or `None` when there isn't exactly one.
pub(crate) fn json_array(raw: &str) -> Option<Vec<Value>> {
    sole(raw, '[', ']', |v| match v {
        Value::Array(items) => Some(items),
        _ => None,
    })
}

/// The single JSON object in `raw`, or `None` when there isn't exactly one.
pub(crate) fn json_object(raw: &str) -> Option<Map<String, Value>> {
    sole(raw, '{', '}', |v| match v {
        Value::Object(fields) => Some(fields),
        _ => None,
    })
}

/// Every balanced object span in `raw` that parses and satisfies `wanted`.
///
/// The looser sibling of [`json_object`], for a reply that may legitimately carry an
/// unrelated object beside the answer — a settings blob, a worked aside — where only the
/// answer-shaped ones should count as candidates. `wanted` is what draws that line, so the
/// caller keeps the "exactly one candidate, or refuse" judgement instead of this module
/// making it on a shape it can't recognise.
///
/// It inherits the guards in [`balanced_spans`]: a reply cut off mid-string or mid-bracket
/// yields nothing at all rather than a partial set. That inheritance is the reason to use
/// this rather than hand-roll a brace scanner, which is how `prospects::advance` came to
/// carry a weaker variant of the same code.
pub(crate) fn json_objects_where(
    raw: &str,
    wanted: impl Fn(&Map<String, Value>) -> bool,
) -> Vec<Map<String, Value>> {
    let Some(spans) = balanced_spans(raw, '{', '}') else {
        return Vec::new(); // an unfinished scan is evidence of nothing — see below
    };
    spans
        .into_iter()
        .filter_map(|span| match serde_json::from_str::<Value>(span) {
            Ok(Value::Object(fields)) if wanted(&fields) => Some(fields),
            _ => None,
        })
        .collect()
}

/// Parse every balanced span and return the one result satisfying `pick` — or `None` when
/// none do, or when more than one does (an ambiguous reply, which is refused rather than
/// guessed at).
fn sole<T>(raw: &str, open: char, close: char, pick: impl Fn(Value) -> Option<T>) -> Option<T> {
    let mut found: Option<T> = None;
    for span in balanced_spans(raw, open, close)? {
        let Some(value) = serde_json::from_str::<Value>(span).ok().and_then(&pick) else {
            continue; // not JSON, or not the shape being looked for
        };
        if found.is_some() {
            return None; // two candidates — the model said one thing and echoed another
        }
        found = Some(value);
    }
    found
}

/// The complete, top-level `open`…`close` spans in `raw`, in order. `None` when there are
/// more than [`MAX_SPANS`].
///
/// Only top-level spans are returned: the nested `[2, 7]` inside `[{"snippets": [2, 7]}]`
/// is part of the outer span, not a sibling of it. An unmatched *closing* bracket is
/// ignored, since it can't bound a structure; an unmatched *opening* one voids the entire
/// scan (see below), since it means the reply stopped partway.
///
/// String literals are skipped, escapes included, so a bracket *inside* a JSON string
/// can't throw the depth count off. That matters: without it, a perfectly good reply
/// carrying an unbalanced bracket in some `reason` field would stop being found at all.
///
/// Strings are tracked at every depth, prose included, so a bracket inside a `reason`
/// field can't throw the count off.
///
/// # Why an unfinished scan voids everything
///
/// The scan ends in one of three states, and **two of them refuse**: an unterminated
/// string (`in_string`), or an unclosed bracket (`depth > 0`). Both mean the reply was cut
/// off or malformed partway, and both are load-bearing rather than tidiness.
///
/// The danger is not the lost span, it's what losing it does to the ambiguity rule in
/// [`sole`]. That rule is the entire defence against a model echoing the example it was
/// given, and it only fires when TWO candidates parse. So any way the real answer stops
/// being a candidate silently disarms it and leaves the echo standing alone — and the
/// fabricated example is then returned AS the model's answer. Two reply shapes do exactly
/// that, and both are ordinary rather than contrived:
///
/// - echo, then a real answer truncated mid-string (a token-limit cutoff inside
///   `"reason": "same pric`) — the unterminated string swallows the brackets after it;
/// - echo, then a real answer truncated anywhere *outside* a string (after a `}`, a digit,
///   a comma, or a closed string) — the bracket never closes, so no span is emitted.
///
/// The same text completed refuses correctly, which is what makes each a fail-open rather
/// than a missed parse: the truncation converts a refusal into acceptance. Refusing on
/// either is right because the parity this scan depends on was unreliable, so no span
/// found under it is evidence of anything.
///
/// A caveat worth stating plainly, because the guards above do NOT cover it: a real answer
/// that is *balanced but unparseable* (one unescaped quote in a prose field) still leaves
/// the echo as the sole candidate — [`sole`] skips a span that fails to parse. That hole
/// cannot be closed here without refusing ordinary prose brackets, so it is closed at the
/// source instead: a machine-read instruction must print an unparseable `<…>` shape rather
/// than a valid example, so there is nothing echoable to win. See [`crate::ai::prompt`].
fn balanced_spans(raw: &str, open: char, close: char) -> Option<Vec<&str>> {
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (i, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
        } else if ch == open {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        } else if ch == close && depth > 0 {
            depth -= 1;
            if depth == 0 {
                // `close` is ASCII, so the inclusive end sits on a char boundary.
                spans.push(&raw[start..=i]);
                if spans.len() > MAX_SPANS {
                    return None;
                }
            }
        }
    }
    // The scan didn't finish cleanly: a string or a bracket is still open, so the reply was
    // cut off or malformed partway and the parity above was unreliable. Whatever spans it
    // produced are not evidence of anything — see the doc comment for why returning them
    // is actively dangerous rather than merely lossy.
    if in_string || depth > 0 {
        return None;
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_plain_array_or_object() {
        assert_eq!(json_array(r#"[1, 2]"#).unwrap().len(), 2);
        assert!(json_object(r#"{"a": 1}"#).unwrap().contains_key("a"));
    }

    #[test]
    fn finds_json_wrapped_in_prose_or_fences() {
        let raw = "Sure! Here you go:\n```json\n[{\"a\": 1}]\n```\nHope that helps.";
        assert_eq!(json_array(raw).unwrap().len(), 1);
        let raw = "Sure:\n```json\n{\"position\": 0.1}\n```\n";
        assert!(json_object(raw).unwrap().contains_key("position"));
    }

    /// A bracket in the preamble produces a second span, but it isn't JSON — so this
    /// still resolves, and is NOT confused with the two-real-candidates case below.
    #[test]
    fn a_non_json_bracket_in_the_preamble_is_ignored() {
        assert_eq!(json_array("Here you go [note]: [{\"a\": 1}]").unwrap().len(), 1);
        assert!(json_object("Note {see below}: {\"a\": 1}").unwrap().contains_key("a"));
    }

    /// The reason this module exists. A model that answers and then restates the shape
    /// it was asked for must not have the example read as its answer — which is what the
    /// old first-open-to-last-close slide did, because the real answer's slice was
    /// invalid and the echo was the only thing left that parsed.
    #[test]
    fn an_echoed_example_is_refused_not_mistaken_for_the_answer() {
        // The empty answer plus an echo of the dedup instruction's own example. The old
        // algorithm returned the fabricated group; two candidates must refuse.
        let raw = r#"[] (for example: [{"snippets": [2, 7], "keep": 7, "reason": "both state SOC2"}])"#;
        assert!(json_array(raw).is_none());

        // A genuine answer plus an echo is equally ambiguous — refuse rather than pick.
        let raw = r#"[{"snippets": [3, 9], "keep": 3}] e.g. [{"snippets": [2, 7], "keep": 7}]"#;
        assert!(json_array(raw).is_none());

        // Objects, both orders. Left unrefused, the classifier wrote the echo's
        // position 0.5 / empty category over a real stage label.
        let raw = r#"{"position": 0.86, "category": "Calling to meet"} - for reference the shape is {"position": 0.5, "category": ""}"#;
        assert!(json_object(raw).is_none());
        let raw = r#"The shape is {"position": 0.5}. My answer: {"position": 0.08}"#;
        assert!(json_object(raw).is_none());
    }

    /// The nastiest shape found here, and the reason an unterminated string voids the
    /// scan. Skipping string contents means a stray `"` eats the brackets after it, and in
    /// THIS ordering — echo, stray quote, real answer — that eats the real answer, leaving
    /// the echo as the only span. One span is never ambiguous, so the refusal above cannot
    /// fire and the fabricated example is returned AS the answer. The balanced-quote
    /// version of the same text refuses, which is what makes this a fail-open and not
    /// merely a lost reply.
    #[test]
    fn an_unterminated_string_cannot_smuggle_an_echo_past_the_ambiguity_rule() {
        let echo = r#"[{"snippets": [2, 7], "keep": 7}]"#;
        let real = r#"[{"snippets": [3, 9], "keep": 3}]"#;

        let smuggled = format!(r#"{echo} - that was the "example. Mine: {real}"#);
        assert!(
            json_array(&smuggled).is_none(),
            "an unterminated string must void the scan, not hand back the echo"
        );
        // Balanced quotes over the same content: two spans, correctly refused.
        let balanced = format!(r#"{echo} - that was the "example". Mine: {real}"#);
        assert!(json_array(&balanced).is_none());

        // The mirror ordering is also voided — the parity was unreliable either way.
        let mirrored = format!(r#"{real} not the "example: {echo}"#);
        assert!(json_array(&mirrored).is_none());

        // Objects too; the guard lives in the shared scan.
        assert!(json_object(r#"{"position": 0.86} was the "example: {"position": 0.5}"#).is_none());
    }

    /// The realistic way the case above arrives, and the reason it isn't exotic: a reply
    /// cut off mid-string by a token limit leaves the scan inside a string with **no stray
    /// prose quote anywhere**. Pair that with an echoed example first — both ordinary model
    /// behaviours — and the truncated real answer vanishes while the echo stands.
    ///
    /// Note this is also why the tempting precedent of tracking strings only at
    /// `depth > 0` would not have been enough: a truncated string sits INSIDE the JSON, at
    /// depth > 0, where that variant tracks it exactly as this one does. Only refusing on
    /// an unterminated string closes both routes.
    #[test]
    fn a_truncated_reply_after_an_echo_refuses_rather_than_returning_the_echo() {
        let echo = r#"[{"snippets": [2, 7], "keep": 7}]"#;
        let cut = r#"[{"snippets": [3, 9], "keep": 3, "reason": "same pric"#;
        assert!(
            json_array(&format!("{echo} and my answer is {cut}")).is_none(),
            "a truncated answer must not leave an echoed example as the result"
        );

        // Same shape on the object callers, whose parses have nothing downstream of them.
        assert!(json_object(
            r#"{"position": 0.5, "category": ""} then {"position": 0.86, "category": "Calling to me"#
        )
        .is_none());

        // Control: plain truncation with no echo has nothing to smuggle, and still refuses
        // rather than half-parsing.
        assert!(json_array(cut).is_none());
    }

    #[test]
    fn empty_structures_are_found_not_rejected() {
        // An empty array is a meaningful verdict for some callers ("nothing is
        // redundant"), so it must come back as Some(vec![]), not None.
        assert_eq!(json_array("[]").unwrap().len(), 0);
        assert_eq!(json_object("{}").unwrap().len(), 0);
    }

    #[test]
    fn a_nested_structure_is_part_of_its_parent_not_a_sibling() {
        // `[2, 7]` must not count as a second candidate array.
        let raw = r#"[{"snippets": [2, 7], "keep": 7}]"#;
        assert_eq!(json_array(raw).unwrap().len(), 1);
    }

    /// Without string-awareness the depth count would never return to zero here and a
    /// perfectly good reply would stop being found.
    #[test]
    fn a_bracket_inside_a_string_does_not_break_the_scan() {
        let raw = r#"[{"reason": "the [first name] blank"}]"#;
        assert_eq!(json_array(raw).unwrap().len(), 1);
        // Balanced brackets inside a string are fine too, as are escaped quotes.
        let raw = r#"[{"reason": "a [b] c", "note": "say \"hi\" [x"}]"#;
        assert_eq!(json_array(raw).unwrap().len(), 1);
        let raw = r#"{"category": "Opener [draft"}"#;
        assert!(json_object(raw).unwrap().contains_key("category"));
    }

    #[test]
    fn missing_or_malformed_json_is_not_found() {
        assert!(json_array("no json here").is_none());
        assert!(json_array("").is_none());
        assert!(json_object("no json here").is_none());
        // Malformed is never repaired — just not found.
        assert!(json_array(r#"[{"a": }]"#).is_none());
        assert!(json_object(r#"{"a": }"#).is_none());
        // An unmatched close can't bound a span; nor can an unterminated open.
        assert!(json_array("] [").is_none());
        assert!(json_array(r#"[{"a": 1}"#).is_none());
    }

    #[test]
    fn a_wrong_shape_is_not_coerced() {
        assert!(json_array(r#"{"a": 1}"#).is_none());
        assert!(json_object(r#"[1, 2]"#).is_none());
    }

    /// The sibling of the mid-string truncation above, and the more common one: a reply cut
    /// off anywhere OUTSIDE a string leaves the bracket open rather than the quote, so the
    /// `in_string` guard alone never saw it. Each cut point below left the echo as the sole
    /// candidate and returned it as the model's answer.
    #[test]
    fn a_reply_truncated_outside_a_string_refuses_rather_than_returning_the_echo() {
        let echo = r#"[{"snippets": [2, 7], "keep": 7, "reason": "both state SOC2"}]"#;
        for cut in [
            r#"[{"snippets": [3, 9], "keep": 3, "reason": "same pricing"}"#, // after a `}`
            r#"[{"snippets": [3, 9], "keep": 3"#,                            // after a digit
            r#"[{"snippets": [3, 9],"#,                                      // after a comma
            r#"[{"snippets": [3, 9], "keep""#,       // after a closed string
            r#"[{"#,                                 // barely started
        ] {
            assert!(
                json_array(&format!("{echo} and my answer is {cut}")).is_none(),
                "a reply truncated at `{cut}` must refuse, not return the echoed example"
            );
        }

        // Objects too — this is the route that blanks a snippet's stage.
        assert!(json_object(
            r#"{"position": 0.5, "category": ""} then {"position": 0.86, "category": "Warm"#
        )
        .is_none());
    }

    /// The limit the two guards do NOT close, pinned deliberately so it isn't mistaken for
    /// a guarantee: a real answer that is balanced but unparseable (one unescaped quote) is
    /// skipped as "not the shape", leaving a well-formed echo as the sole candidate. This is
    /// why the instructions print unparseable shapes instead of valid examples — see
    /// `ai::prompt`'s `no_machine_read_instruction_prints_a_parseable_answer`.
    #[test]
    fn a_balanced_but_unparseable_answer_still_loses_to_a_well_formed_echo() {
        let echo = r#"[{"snippets": [2, 7], "keep": 7, "reason": "both state SOC2"}]"#;
        let malformed = r#"[{"snippets": [3, 9], "keep": 3, "reason": "both say "maybe""}]"#;
        assert_eq!(
            json_array(&format!("{echo} My answer: {malformed}"))
                .map(|items| items.len()),
            Some(1),
            "if this ever starts refusing, the upstream shape rule has a backstop and this \
             test's premise should be revisited"
        );
        // The same malformed answer alone still fails closed — it is only dangerous when
        // there is a well-formed rival for it to lose to.
        assert!(json_array(malformed).is_none());
    }

    /// Spans are sliced by byte index with an inclusive end, which is safe only because
    /// `close` is ASCII. Nothing enforces that, so pin it: multibyte text around and inside
    /// the JSON must not panic.
    #[test]
    fn multibyte_text_around_the_json_does_not_panic() {
        let raw = "Voilà — 日本語 説明: [{\"reason\": \"café naïve — 見積\"}]";
        let items = json_array(raw).expect("the array is still found");
        assert_eq!(items.len(), 1);
        assert!(json_object("émoji 🎉 {\"position\": 0.5, \"category\": \"Warm\"}").is_some());
    }

    /// Exceeding the scan cap refuses. Returning the spans found so far would reopen the
    /// fail-open: a real answer sitting past the cap would be judged on the decoys.
    #[test]
    fn exceeding_the_span_cap_refuses() {
        let decoys = "[] ".repeat(MAX_SPANS + 2);
        assert!(json_array(&decoys).is_none());
        // Just under the cap still resolves — one real array among non-JSON spans.
        let decoys = "[x] ".repeat(MAX_SPANS - 1);
        assert_eq!(json_array(&format!("{decoys}[{{\"a\": 1}}]")).unwrap().len(), 1);
    }
}
