//! The blank ("placeholder") syntax a snippet may carry — `[first name]`,
//! `[their company]` — and the grounding rules that syntax complicates.
//!
//! A blank is a stand-in for a detail that isn't known until a message is actually
//! written. The draft composer already fills them (see the PLACEHOLDERS rule in
//! `ai::prompt::DRAFT_INSTRUCTION`) from the prospect's name, the conversation, the
//! profile, the product, or the customer profile, and is forbidden from emitting a
//! literal bracket. The snippet editor teaches the same convention. This module is
//! the other end: it lets the *propose* pass template a line instead of throwing it
//! away, without loosening the guarantee that a proposal is the founder's own text.
//!
//! Why the guarantee needs care: `proposals` accepts a candidate only if it appears
//! in the message the user actually sent. A blank punches a wildcard into that
//! check, so the rules here are deliberately tight — the model may only *blank out*
//! a span it saw, never reword one:
//!   - **Literals stay verbatim, in order** — [`Template::is_grounded_in`] requires
//!     every non-blank segment to appear in the sent message, in sequence.
//!   - **A blank must stand in for something** — each gap between literals must hold
//!     at least one non-whitespace character of the original, so a blank can't be
//!     wedged into a space that was already there.
//!   - **The line stays mostly the founder's words** — at most [`MAX_BLANKS`] blanks;
//!     [`MIN_LITERAL_PERCENT`] of the matched span must survive as literal text; no one
//!     blank may absorb more than [`MAX_BLANK_SPAN`] characters; and a blanked line must
//!     carry at least [`MIN_LITERAL_CHARS`] literal characters. The three floors catch
//!     different things and none of them subsumes the others — a ratio can't tell
//!     `we ship weekly` from `we [x]`, and an absolute cap can't tell a long line from a
//!     short one.
//!
//! # What code here does NOT decide
//!
//! Two judgements are semantic and stay in the propose/review prompts: whether a blank
//! asks for something *fillable*, and **whether blanking a span changed what the sentence
//! means**. The second is worth naming explicitly, because it is the sharpest limit of
//! this module and it is not obvious.
//!
//! No character-counting rule can catch it. Blanking `not SOC2` out of "we are not SOC2
//! compliant" leaves a line that is 93% literal text, well clear of the share floor, and
//! deletes seven characters, well under any cap that still permits the legitimate
//! `Congrats on [their funding round]` (which stands in for fifteen). The floors here
//! bound *scale*; they cannot bound *meaning*. So the guarantee this module provides is
//! precisely "the literals are verbatim and in order, and the blanks are bounded in
//! size" — which is weaker than "the line still says what the founder said", and the
//! propose and review instructions carry that rule instead.

/// The most blanks one proposed snippet may carry.
///
/// Two, not more: every blank is a wildcard gap in [`Template::is_grounded_in`], so
/// a short line with three of them is mostly wildcard and "the literals appear in
/// order" stops proving much. Two also covers the real cases — the shape worth
/// rescuing is `saw [their company] is hiring across [their team]`.
pub(crate) const MAX_BLANKS: usize = 2;

/// Longest a blank's label may be, in characters.
const MAX_LABEL_LEN: usize = 40;

/// The smallest share (percent) of the ORIGINAL SPAN that must survive as literal text.
///
/// Measured against what the blanks actually replaced in the sent message, not against
/// the length of their labels. Labels are model-authored and say nothing about how much
/// of the founder's sentence was removed, so a label-based share was measuring the wrong
/// quantity entirely: one blank with a short label could delete an arbitrarily long span
/// and the floor would still read as satisfied. See [`Template::is_grounded_in`], which
/// is where this is now enforced, because only the matcher knows the span.
const MIN_LITERAL_PERCENT: usize = 60;

/// The most non-whitespace characters of the original message a SINGLE blank may stand in
/// for.
///
/// A percentage floor alone can't bound this: a long line with one short blank passes at
/// 90-something percent no matter what that blank swallowed. This is the absolute bound —
/// it stops a blank quietly consuming a whole clause.
///
/// Deliberately generous rather than tight. `Congrats on [their funding round]` legitimately
/// stands in for "the Series A round" (15 characters), so a cap low enough to catch every
/// meaning-changing deletion would reject the shape blanking exists to rescue. What this
/// catches is scale, not semantics — see the module docs on what code does NOT decide here.
const MAX_BLANK_SPAN: usize = 40;

/// The fewest literal, non-whitespace characters a BLANKED proposal must carry.
///
/// The percentage floor is a ratio, so it can't tell `we ship weekly` from `we [x]` — two
/// literal characters against one swallowed character is 66%, comfortably over the share
/// floor, and yet the line is a bare skeleton that matches almost any message. An absolute
/// minimum is what rules that out.
///
/// Applies only when the content actually has a blank. With no blank the content is
/// entirely the founder's own words and its length is nobody's business — `book a call?`
/// is a perfectly good short snippet.
///
/// Calibrated against the shortest *legitimate* blanked lines rather than picked round:
/// `saw [their company] is hiring` carries 11 literal characters and
/// `congrats on [the milestone]` carries 10, so the floor sits at 10. It still rejects
/// every skeleton shape (`we [a]` = 2, `SOC2 [x]` = 5, `for [their kind of team]` = 3).
/// The margin is deliberately thin: this exists to catch lines that are almost entirely
/// blank, not to impose a house style on snippet length.
const MIN_LITERAL_CHARS: usize = 10;

/// A parsed snippet content: the literal segments, and the canonical text to store.
///
/// `literals` always holds `blanks + 1` entries — the text before the first blank,
/// between each pair, and after the last. Any of them may be empty (a blank at the
/// very start or end), which the matcher handles.
pub(crate) struct Template {
    /// The content with its labels canonicalized — what actually gets stored.
    pub(crate) content: String,
    literals: Vec<String>,
}

impl Template {
    /// Whether this content is grounded in one of the messages the user actually
    /// sent: every literal segment present, in order, with each blank standing in
    /// for at least one non-whitespace character of the original.
    ///
    /// With no blanks this is exactly a whitespace-normalized substring test — the
    /// original verbatim guarantee, unchanged.
    pub(crate) fn is_grounded_in(&self, messages: &[String]) -> bool {
        let literals: Vec<Vec<char>> =
            self.literals.iter().map(|l| normalize_ws(l).chars().collect()).collect();
        // A content of nothing but blanks anchors nowhere; `parse` already rejects it
        // via the literal floor, but the matcher must not vacuously agree.
        if literals.iter().all(|l| l.is_empty()) {
            return false;
        }
        let literal_chars: usize = literals.iter().map(|l| non_ws_chars(l)).sum();
        messages.iter().any(|m| {
            let hay: Vec<char> = normalize_ws(m).chars().collect();
            let Some(blanks) = matches(&hay, &literals) else {
                return false;
            };
            // How much of the founder's sentence each blank actually stood in for. This
            // is the quantity the floors below are about, and it's knowable only here —
            // `parse` sees the label, never the span.
            let absorbed: Vec<usize> =
                blanks.iter().map(|&(from, to)| non_ws_chars(&hay[from..to])).collect();
            if absorbed.iter().any(|&n| n > MAX_BLANK_SPAN) {
                return false; // one blank swallowed a whole clause
            }
            let swallowed: usize = absorbed.iter().sum();
            literal_chars * 100 >= MIN_LITERAL_PERCENT * (literal_chars + swallowed)
        })
    }
}

/// Walk `literals` through `hay` left to right, taking the earliest placement of
/// each. Earliest is optimal here — it leaves the most room for what follows — so a
/// single greedy pass is a complete test, not a heuristic.
///
/// Returns the `hay` range each blank stands in for, in order, so the caller can measure
/// how much of the original was actually removed. Earliest placement also means those
/// ranges are the SMALLEST consistent with the literals, which is the generous reading —
/// a line is rejected for dilution only when it is diluted on every possible match.
fn matches(hay: &[char], literals: &[Vec<char>]) -> Option<Vec<(usize, usize)>> {
    let first = literals.first()?;
    let start = find_from(hay, first, 0)?;
    let mut pos = start + first.len();
    let mut blanks = Vec::with_capacity(literals.len().saturating_sub(1));

    for lit in &literals[1..] {
        // The blank before this literal must cover real text, not just the space
        // that was already there — so find the next non-whitespace character and
        // require this literal to start strictly after it.
        let gap = (pos..hay.len()).find(|&i| !hay[i].is_whitespace())?;
        let at = find_from(hay, lit, gap + 1)?;
        blanks.push((pos, at));
        pos = at + lit.len();
    }
    Some(blanks)
}

/// The leftmost index at or after `from` where `needle` occurs in `hay`. An empty
/// needle matches at `from` itself (the trailing-blank case), provided `from` is
/// still within the haystack.
fn find_from(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(from);
    }
    let last = hay.len().checked_sub(needle.len())?;
    (from..=last).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Parse `content` into a [`Template`], or `None` when it isn't usable: malformed
/// brackets (unbalanced, nested, unterminated), a label that's empty, over-long, or
/// carrying characters a blank has no business holding, more than [`MAX_BLANKS`]
/// blanks, or too little literal substance left around them.
///
/// A plain content with no brackets at all parses fine, with zero blanks — callers
/// get one path for both.
pub(crate) fn parse(content: &str) -> Option<Template> {
    let mut literals: Vec<String> = Vec::new();
    let mut literal = String::new();
    let mut canonical = String::new();
    let mut label = String::new();
    let mut in_label = false;

    for ch in content.chars() {
        match ch {
            '[' => {
                if in_label {
                    return None; // nested
                }
                in_label = true;
                label.clear();
            }
            ']' => {
                if !in_label {
                    return None; // unbalanced close
                }
                in_label = false;
                let clean = canonical_label(&label)?;
                canonical.push('[');
                canonical.push_str(&clean);
                canonical.push(']');
                literals.push(std::mem::take(&mut literal));
            }
            c if in_label => label.push(c),
            c => {
                literal.push(c);
                canonical.push(c);
            }
        }
    }
    if in_label {
        return None; // unterminated
    }
    literals.push(literal);

    if literals.len() - 1 > MAX_BLANKS {
        return None;
    }
    if !has_literal_substance(&literals) {
        return None;
    }
    Some(Template { content: canonical, literals })
}

/// Normalize one blank's label, or reject it. Whitespace is collapsed, and the
/// charset is held to letters, digits, spaces, apostrophes, and hyphens — a label is
/// model-authored text that ends up inside the draft prompt as snippet content, so
/// it stays a short noun phrase rather than a free-text channel.
fn canonical_label(raw: &str) -> Option<String> {
    let label = normalize_ws(raw);
    if label.is_empty() || label.chars().count() > MAX_LABEL_LEN {
        return None;
    }
    if !label.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '\'' || c == '-') {
        return None;
    }
    // An ALL-CAPS label is the template-variable spelling ([FIRST NAME]); fold it to
    // the prose spelling the editor teaches, so a snippet reads as a sentence with a
    // gap. Mixed case is left alone — that's how a proper noun survives
    // ([their LinkedIn post]).
    if label.chars().any(char::is_uppercase) && !label.chars().any(char::is_lowercase) {
        return Some(label.to_lowercase());
    }
    Some(label)
}

/// Whether the content carries enough of the founder's own words to be worth templating
/// at all — the message-independent half of the floor.
///
/// Only an absolute minimum lives here, and only for blanked content. The *share* that
/// must survive is checked in [`Template::is_grounded_in`] instead, because it depends on
/// how much of the sent message each blank replaced, which is knowable only once a match
/// is found. This used to compare literal characters against LABEL characters, which
/// measured nothing useful: labels are model-authored, so a short label made an
/// arbitrarily large deletion look fine.
fn has_literal_substance(literals: &[String]) -> bool {
    let literal_chars: usize = literals.iter().map(|l| non_ws(l)).sum();
    if literal_chars == 0 {
        return false; // nothing but blanks — anchors nowhere
    }
    // No blanks at all: it's entirely the founder's own text, so any length is fine.
    if literals.len() == 1 {
        return true;
    }
    literal_chars >= MIN_LITERAL_CHARS
}

fn non_ws(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

fn non_ws_chars(chars: &[char]) -> usize {
    chars.iter().filter(|c| !c.is_whitespace()).count()
}

/// Check hand-authored content for a broken blank, returning the problem for the editor
/// to show. `None` means the brackets are usable.
///
/// Deliberately much weaker than [`parse`], and the difference is the point. `parse` runs
/// on MODEL-authored candidates, where the whole apparatus — [`MAX_BLANKS`], the literal
/// floors, the label charset — exists to stop a model templating away the founder's
/// sentence. None of that applies to the founder writing their own line: they are the
/// author, there is nothing to protect them from, and a deliberate three-blank template is
/// their business. Enforcement was nonetheless *inverted* before this — the model was both
/// instructed and code-checked, the human neither — so what's checked here is only what is
/// broken rather than merely unconventional:
///
///   - **unbalanced or nested brackets**, which can't be filled and put a literal bracket
///     in front of a prospect, and
///   - **an empty label**, which asks for nothing.
///
/// Note this runs on an autosaved field, so a half-typed `[first` legitimately trips it.
/// That's acceptable, and arguably useful: the editor surfaces it inline without discarding
/// what was typed, and it clears itself as soon as the bracket is closed.
pub(crate) fn shape_error(content: &str) -> Option<&'static str> {
    let mut depth = 0usize;
    let mut label = String::new();
    for ch in content.chars() {
        match ch {
            '[' => {
                if depth > 0 {
                    return Some("A blank can't contain another blank — check the [brackets].");
                }
                depth += 1;
                label.clear();
            }
            ']' => {
                if depth == 0 {
                    return Some("There's a stray ']' — check the [brackets].");
                }
                depth -= 1;
                if label.trim().is_empty() {
                    return Some("A blank needs a name, like [first name].");
                }
            }
            c if depth > 0 => label.push(c),
            _ => {}
        }
    }
    if depth > 0 {
        return Some("A blank is missing its closing ']'.");
    }
    None
}

/// Replace every blank with an ellipsis, so a blanked line can be used somewhere a
/// bracket must never appear — while still reading as the founder's own prose.
///
/// Written for the commenter's voice corpus. Dropping blanked snippets there kept the
/// bracket out but threw the *voice* out with it: a founder whose library is mostly
/// blanked lines got a corpus of two samples, or none, and the comment was then written
/// with no style reference at all — the templating feature quietly eroding a different
/// feature as it succeeded. A blank's label is the only part that isn't the founder's
/// writing, so eliding just the label keeps the sentence shape, the word choice and the
/// rhythm, which is all a style sample is for.
///
/// Guarantees no `[` or `]` survives, whatever the input. Like [`dedup_key`] it is
/// deliberately lenient — it runs over hand-authored content, so an unbalanced or nested
/// bracket must still come out clean rather than error. An unterminated `[` elides the
/// rest of the line; a stray `]` is dropped.
pub(crate) fn elide_blanks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_label = false;
    for ch in content.chars() {
        match ch {
            '[' if !in_label => {
                in_label = true;
                out.push('…');
            }
            ']' if in_label => in_label = false,
            // A stray close outside a label, and any bracket nested inside one, are
            // dropped rather than emitted — the no-bracket guarantee is absolute.
            '[' | ']' => {}
            // Label text is the model's wording, not the founder's, so it goes.
            _ if in_label => {}
            c => out.push(c),
        }
    }
    normalize_ws(&out)
}

/// A dedup key for snippet content: whitespace- and case-normalized, with every
/// blank collapsed to a bare `[]`.
///
/// Collapsing the label is the point — `congrats on [the milestone]` and
/// `congrats on [your news]` are the same snippet, and without this each new
/// wording of the same blank would land as a fresh proposal.
///
/// Deliberately lenient, unlike [`parse`]: it runs over whatever is already in the
/// library, including hand-authored content, so malformed brackets must produce a
/// stable key rather than an error.
pub(crate) fn dedup_key(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_label = false;
    for ch in content.chars() {
        match ch {
            '[' if !in_label => {
                in_label = true;
                out.push_str("[]");
            }
            ']' if in_label => in_label = false,
            _ if in_label => {} // label text (and any nested bracket) is dropped
            c => out.push(c),
        }
    }
    normalize_ws(&out).to_lowercase()
}

/// Collapse a string to trimmed, single-spaced form, so a whitespace-only difference
/// between what the model returned and what was scraped doesn't defeat the checks.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grounded(messages: &[&str], content: &str) -> bool {
        let messages: Vec<String> = messages.iter().map(|m| m.to_string()).collect();
        parse(content).is_some_and(|t| t.is_grounded_in(&messages))
    }

    // ---- the plain (no-blank) path: the original verbatim guarantee, unchanged ----

    #[test]
    fn accepts_a_span_present_in_the_message() {
        let messages = ["Hi Ada, we are SOC2 compliant and ship weekly."];
        assert!(grounded(&messages, "we are SOC2 compliant"));
        // Whitespace differences don't matter.
        assert!(grounded(&messages, "we are  SOC2   compliant"));
    }

    #[test]
    fn rejects_a_paraphrase_or_invention() {
        let messages = ["Hi Ada, we are SOC2 compliant."];
        assert!(!grounded(&messages, "we hold SOC2 certification")); // paraphrase
        assert!(!grounded(&messages, "we raised a seed round")); // invented
        assert!(!grounded(&messages, "")); // empty never matches
    }

    // ---- templating ----

    #[test]
    fn accepts_a_span_with_the_personal_detail_blanked_out() {
        let messages =
            ["Hi Ada, we are SOC2 Type II certified and closed our first enterprise deal."];
        assert!(grounded(
            &messages,
            "Hi [first name], we are SOC2 Type II certified and closed our first enterprise deal."
        ));
    }

    #[test]
    fn accepts_a_blank_in_the_middle_and_at_the_end() {
        let messages = ["I saw Acme is hiring across the ops team and thought of you."];
        assert!(grounded(
            &messages,
            "I saw [their company] is hiring across the ops team and thought of you."
        ));
        assert!(grounded(&messages, "I saw Acme is hiring across [their team]"));
    }

    #[test]
    fn rejects_a_template_whose_literals_were_reworded_or_reordered() {
        let messages = ["I saw Acme is hiring across the ops team and thought of you."];
        // A literal that isn't in the message.
        assert!(!grounded(&messages, "I noticed [their company] is hiring across the ops team"));
        // The right literals, wrong order.
        assert!(!grounded(&messages, "hiring across the ops team [x] I saw Acme is"));
    }

    #[test]
    fn rejects_a_blank_wedged_where_nothing_was() {
        // The message has only a space between "ship" and "weekly" — a blank must
        // stand in for real text, not punch a hole in whitespace.
        let messages = ["we ship weekly without fail and never break the build"];
        assert!(!grounded(&messages, "we ship [cadence] weekly without fail and never break"));
        // The same shape IS accepted when a real word sits there.
        let messages = ["we ship every single weekly without fail and never break the build"];
        assert!(grounded(&messages, "we ship [cadence] weekly without fail and never break"));
    }

    #[test]
    fn a_blank_may_cover_several_words() {
        let messages = ["Congrats on the Series A round - most teams find hiring is the hard part."];
        assert!(grounded(
            &messages,
            "Congrats on [their funding round] - most teams find hiring is the hard part."
        ));
    }

    // ---- the shape floors ----

    #[test]
    fn rejects_more_than_two_blanks() {
        assert!(parse("we ship [a] weekly and never [b] break the build for [c] teams").is_none());
        // Two is still fine, given enough literal substance around them.
        assert!(parse("we ship [a] weekly and never break the build for [c] teams").is_some());
    }

    /// The floor used to compare literal characters against LABEL characters, so a short
    /// label made an arbitrarily large deletion look fine. `we [a]` is the compact proof:
    /// two literal characters against a one-character label is 66%, over the share floor,
    /// and it matches almost any message. Only an absolute minimum rules it out.
    #[test]
    fn rejects_a_skeleton_whose_short_label_flattered_the_old_floor() {
        assert!(parse("we [a]").is_none());
        assert!(parse("SOC2 [x]").is_none());
        assert!(parse("for [their kind of team]").is_none());
        // The minimum applies only to BLANKED content — an unblanked line is entirely
        // the founder's own words, so a short one is fine.
        assert!(parse("book a call?").is_some());
        assert!(parse("worth 15 minutes?").is_some());
        // Just enough literal substance around a blank still passes.
        assert!(parse("saw [their company] is hiring").is_some());
    }

    /// A percentage floor can't bound this on its own: a long line with one short blank
    /// passes at 90-something percent regardless of what the blank swallowed. The
    /// absolute cap is what stops a blank consuming a whole clause.
    #[test]
    fn rejects_a_blank_that_absorbs_a_whole_clause() {
        let messages = ["we ship weekly and we have never once missed a customer deadline \
                         in three years of operating, and every byte is encrypted at rest"];
        // One blank standing in for ~60 non-whitespace characters — over the cap.
        assert!(!grounded(
            &messages,
            "we ship weekly and [what we have managed], and every byte is encrypted at rest"
        ));
        // A blank standing in for a short phrase is still fine, cap untouched.
        let messages = ["Congrats on the Series A round - most teams find hiring is the hard part."];
        assert!(grounded(
            &messages,
            "Congrats on [their funding round] - most teams find hiring is the hard part."
        ));
    }

    /// The share floor is now measured against what the blanks actually replaced in the
    /// sent message, not against label length — so a line whose blanks swallowed most of
    /// the original is rejected even when the labels are short.
    #[test]
    fn rejects_a_line_whose_blanks_swallowed_most_of_the_original() {
        let messages = ["we are proud to say we ship weekly without ever breaking a build"];
        // Literals "we are " + " ship" survive; the blank replaced "proud to say we",
        // which is most of the span it spans.
        assert!(!grounded(&messages, "we are [x y] ship"));
    }

    /// The limit this module cannot enforce, pinned so it isn't mistaken for a guarantee.
    /// Blanking a negation leaves a line that is overwhelmingly literal and deletes only a
    /// few characters, so no share floor and no cap that still permits
    /// `[their funding round]` can reject it. Catching this is the propose/review
    /// instruction's job, and the module docs say so.
    #[test]
    fn a_blank_that_deletes_a_negation_is_not_caught_by_the_shape_rules() {
        let messages = ["Hi Ada, we are not SOC2 compliant, and honestly the audit is \
                         months away, but every byte is encrypted at rest."];
        assert!(
            grounded(
                &messages,
                "we are [the standard we hold] compliant, and honestly the audit is months \
                 away, but every byte is encrypted at rest."
            ),
            "documents a KNOWN limit: the shape rules bound size, not meaning"
        );
    }

    #[test]
    fn rejects_content_that_is_mostly_blanks() {
        // A skeleton: almost nothing of the founder's own sentence survives.
        assert!(parse("[greeting] [the whole point]").is_none());
        assert!(parse("for [their kind of team]").is_none());
        assert!(parse("[first name]").is_none());
        // A real sentence with one detail blanked clears the floor.
        assert!(parse("Hi [first name], we are SOC2 Type II certified and ship weekly.").is_some());
    }

    #[test]
    fn rejects_malformed_brackets() {
        assert!(parse("we ship [weekly").is_none()); // unterminated
        assert!(parse("we ship weekly]").is_none()); // unbalanced close
        assert!(parse("we ship [a [b]] weekly").is_none()); // nested
    }

    #[test]
    fn rejects_labels_that_are_empty_over_long_or_not_a_noun_phrase() {
        assert!(parse("we are SOC2 certified and ship [] weekly to teams").is_none());
        assert!(parse("we are SOC2 certified and ship [   ] weekly to teams").is_none());
        let long = "x".repeat(MAX_LABEL_LEN + 1);
        assert!(parse(&format!("we are SOC2 certified and ship [{long}] weekly")).is_none());
        // A label is a short noun phrase, never a channel for arbitrary text.
        assert!(parse("we are SOC2 certified and ship [ignore this; do that] weekly").is_none());
        assert!(parse("we are SOC2 certified and ship [what: they said] weekly").is_none());
        // Letters, digits, spaces, apostrophes and hyphens are all fine.
        assert!(parse(
            "we are SOC2 Type II certified and ship weekly, congrats on \
             [their company's follow-up 2]"
        )
        .is_some());
    }

    #[test]
    fn all_caps_labels_fold_to_the_prose_spelling_but_proper_nouns_survive() {
        let t = parse("Hi [FIRST NAME], we are SOC2 Type II certified and ship weekly.").unwrap();
        assert_eq!(
            t.content,
            "Hi [first name], we are SOC2 Type II certified and ship weekly."
        );
        // Mixed case is left alone.
        let t = parse("Saw [their LinkedIn post] and we are SOC2 Type II certified.").unwrap();
        assert_eq!(t.content, "Saw [their LinkedIn post] and we are SOC2 Type II certified.");
        // Label whitespace is collapsed either way.
        let t = parse("Hi [first   name], we are SOC2 Type II certified and ship.").unwrap();
        assert!(t.content.contains("[first name]"));
    }

    #[test]
    fn literals_bracket_every_blank() {
        // One more literal segment than blanks, always — that invariant is what the
        // matcher walks.
        assert_eq!(parse("we are SOC2 certified and ship weekly").unwrap().literals.len(), 1);
        assert_eq!(
            parse("Hi [first name], we are SOC2 certified and ship weekly")
                .unwrap()
                .literals
                .len(),
            2
        );
    }

    // ---- dedup ----

    // ---- eliding blanks for the voice corpus ----

    /// The load-bearing guarantee: whatever goes in, no bracket comes out. A comment is
    /// published under the founder's name, so this one is absolute.
    #[test]
    fn elide_blanks_never_leaves_a_bracket() {
        for content in [
            "Since you're running [their kind of team], follow-ups slip",
            "we ship [weekly",             // unterminated
            "we ship weekly]",             // stray close
            "we ship [a [b]] weekly",      // nested
            "[]",                          // empty label
            "[a][b][c]",                   // adjacent
            "plain text with no blanks",
        ] {
            let out = elide_blanks(content);
            assert!(
                !out.contains('[') && !out.contains(']'),
                "elide_blanks left a bracket in {out:?} (from {content:?})"
            );
        }
    }

    /// The point of eliding rather than dropping: the founder's sentence shape survives,
    /// so it still works as a style sample.
    #[test]
    fn elide_blanks_keeps_the_prose_around_the_gap() {
        assert_eq!(
            elide_blanks("Since you're running [their kind of team], follow-ups slip"),
            "Since you're running …, follow-ups slip"
        );
        // Content with no blanks is returned as-is (whitespace-normalized).
        assert_eq!(elide_blanks("we  ship\nweekly"), "we ship weekly");
        // A line that is nothing but a blank elides to just the gap, which the caller
        // then drops as having no prose left.
        assert_eq!(elide_blanks("[first name]"), "…");
    }

    // ---- the hand-authored shape check ----

    #[test]
    fn shape_error_flags_only_genuinely_broken_blanks() {
        // Broken: unfillable, and a literal bracket would reach the prospect.
        assert!(shape_error("we ship [weekly").is_some());
        assert!(shape_error("we ship weekly]").is_some());
        assert!(shape_error("we ship [a [b]] weekly").is_some());
        assert!(shape_error("we ship [] weekly").is_some());
        assert!(shape_error("we ship [   ] weekly").is_some());

        // Fine — including the shapes `parse` refuses for MODEL-authored candidates.
        // The founder is the author here; a deliberate template is their business.
        assert!(shape_error("we ship weekly").is_none());
        assert!(shape_error("Hi [first name], we ship weekly").is_none());
        assert!(shape_error("Hi [a], for [b] in [c] we can [d]").is_none(), "4 blanks are the author's call");
        assert!(shape_error("we [x]").is_none(), "short is the author's call");
        assert!(shape_error("ship [what: they said]").is_none(), "charset is not enforced by hand");
    }

    #[test]
    fn dedup_key_collapses_whitespace_and_case() {
        assert_eq!(dedup_key("We  Ship\nWeekly"), dedup_key("we ship weekly"));
    }

    #[test]
    fn dedup_key_collapses_differently_worded_blanks() {
        assert_eq!(
            dedup_key("congrats on [the milestone]"),
            dedup_key("congrats on [your news]")
        );
        // A blank is still distinct from no blank at all.
        assert_ne!(dedup_key("congrats on [the milestone]"), dedup_key("congrats on it"));
    }

    #[test]
    fn dedup_key_is_stable_on_malformed_content() {
        // Hand-authored library content may hold anything; the key must not panic or
        // depend on the brackets being well formed.
        assert_eq!(dedup_key("we ship [weekly"), "we ship []");
        assert_eq!(dedup_key("we ship weekly]"), "we ship weekly]");
        assert_eq!(dedup_key("we ship [a [b]] weekly"), "we ship []] weekly");
    }
}
