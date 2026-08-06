//! Finding redundant snippets — the second half of the "Organize library" action.
//!
//! A library accumulates duplicates: the same point written twice because it was added
//! again months later, or captured from several sent messages that each made it. The
//! propose pass guards the *entrance* (see `proposals`, whose reviewer rejects a
//! candidate the library already conveys), but nothing looked back over what was
//! already inside. This does.
//!
//! Sibling of `classify`, and the two run back to back off one click: `reclassify_all`
//! re-stages the library, then this searches it for redundancy. They stay separate
//! commands sequenced by the frontend rather than one combined pass, so that a failure
//! here can degrade quietly without casting doubt on the re-stage that already wrote
//! rows.
//!
//! **This module writes nothing.** It reads the library, asks one question about it,
//! and returns groups for a review panel; deleting is the user picking rows and the UI
//! calling the existing `delete_snippet`. That asymmetry with `classify` (which
//! force-writes) is deliberate — a wrong stage label is a shrug, a wrong deletion is
//! lost material, so no amount of model confidence gets to remove a snippet here.
//!
//! Two invariants are enforced in code rather than trusted to the model, both because
//! the review UI depends on them:
//!   - **A snippet belongs to at most one group.** The prompt asks for this, but if a
//!     reply put one snippet in two groups the panel would show two checkboxes for the
//!     same row and deleting via one would strand the other.
//!   - **A group holds at least two real snippets.** Indices are resolved back to ids
//!     and anything out of range is dropped, so a group can shrink; one that shrinks
//!     below two members isn't a duplicate pair any more and disappears.

use std::collections::HashMap;

use tauri::{AppHandle, Manager};

use super::repository;
use crate::ai::{self, DedupContext, Prompt};
use crate::database::AppState;
use crate::features::product;
use crate::util::MAX_NAME_LEN;

/// The largest group this pass will report.
///
/// A real redundancy group is two or three ways of saying one thing; a dozen is not a
/// duplicate, it's a model that decided a whole topic was one point. Such a group is
/// dropped rather than truncated — an arbitrary subset of an untrustworthy grouping is
/// no more trustworthy, and the panel would render it as a single button offering to
/// delete everything in it.
const MAX_GROUP_MEMBERS: usize = 10;

/// One snippet in a reported group: which row it is, and the exact text that was judged.
///
/// Both halves are needed, and the content is the important one. An id alone is not a
/// stable name for a snippet — `snippets.id` is a plain rowid alias (no `AUTOINCREMENT`),
/// so a deleted row's id gets handed to the next insert — and the panel outlives this
/// reply by however long the user leaves it open. Carrying the analyzed text lets the UI
/// re-check that a row still says what the model judged, for as long as it holds the
/// report; see [`still_current`], which does the same check for the generation window.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct RedundancyMember {
    pub id: i64,
    /// The content exactly as sent to the model, trimmed — so the UI compares like
    /// with like against a row's current, trimmed content.
    pub analyzed: String,
}

/// One group of snippets that say the same thing, for the review panel. Output-only —
/// returned by the command, never accepted as input, and never persisted (a redundancy
/// report describes the library as it is right now, and goes stale the moment the user
/// acts on it).
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct RedundancyGroup {
    /// Every snippet in the group, including the suggested keeper. Always two or more
    /// and at most [`MAX_GROUP_MEMBERS`], and no snippet appears in more than one group.
    pub members: Vec<RedundancyMember>,
    /// The model's pick for the version worth keeping — always one of `members`.
    /// A suggestion the UI pre-selects, not a decision.
    pub keep_id: i64,
    /// A brief phrase naming the point the group shares ("both state SOC2 compliance"),
    /// so a group can be judged without re-reading every line in it.
    pub reason: String,
}

/// Search the approved library for redundancy and return the groups found.
///
/// User-initiated and its result is displayed, so unlike the fire-and-forget passes
/// this propagates failure instead of logging and shrugging: the caller needs to tell
/// "nothing is redundant" apart from "the check didn't run". An empty `Vec` is the
/// former and a real answer; `Err` is the latter.
///
/// Runs on the interactive CLI path ([`run_capped`], which queues rather than skipping)
/// so a click always produces an answer.
pub(crate) async fn find_redundant(app: AppHandle) -> Result<Vec<RedundancyGroup>, String> {
    // Gather under one lock: what's being sold (redundancy is judged against the
    // product, not by surface wording) and the approved, content-bearing library.
    // Blank cards are excluded — an empty snippet duplicates everything and nothing.
    // Cloned because the handle is needed a second time after the generation, for the
    // freshness re-read (see `still_current`).
    let app_for_recheck = app.clone();
    let (product_name, product_description, items) = {
        tokio::task::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let conn = st.conn.lock().map_err(|e| e.to_string())?;
            let product = product::repository::get(&conn).map_err(|e| e.to_string())?;
            let items: Vec<(i64, String, String)> = repository::list_approved(&conn)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|s| !s.content.trim().is_empty())
                .map(|s| (s.id, s.name, s.content))
                .collect();
            Ok::<_, String>((product.name, product.description, items))
        })
        .await
        .map_err(|e| {
            eprintln!("snippets: dedup gather task panicked: {e}");
            "Couldn't read the snippet library.".to_string()
        })??
    };

    // Nothing can duplicate anything below two snippets — skip the call rather than
    // spend a generation confirming it.
    if items.len() < 2 {
        return Ok(Vec::new());
    }

    // `analyzed` and `snippets` share an index: the prompt presents the list 1-indexed
    // and the reply keys back by that number, so position is the only link between the
    // model's answer and real rows. Built together, from one list, in one order.
    // `analyzed` keeps each row's content too, for the freshness check below.
    let analyzed: Vec<(i64, String)> =
        items.iter().map(|(id, _, content)| (*id, content.clone())).collect();
    let snippets: Vec<(String, String)> = items
        .into_iter()
        .map(|(_, name, content)| (name, content))
        .collect();

    let ctx = DedupContext {
        product_name: &product_name,
        product_description: &product_description,
        snippets: &snippets,
    };
    let raw = ai::client::run_capped(Prompt::find_redundant(&ctx)).await?;

    let ids = still_current(app_for_recheck, analyzed).await?;

    parse_groups(&raw, &ids).ok_or_else(|| {
        eprintln!("snippets: dedup reply couldn't be parsed");
        "Couldn't read the redundancy check's answer. Your snippets were re-scored; \
         try the check again."
            .to_string()
    })
}

/// Re-read the library and report, per analyzed slot, whether that row still holds the
/// text that was actually judged — `Some(id)` if it does, `None` if it changed or is gone.
///
/// The generation takes up to a minute, and the library stays fully editable throughout:
/// a snippet can be rewritten from its card, deleted, or added while the reply is in
/// flight. Two distinct things go wrong without this check, and the second is why an
/// existence test isn't enough:
///   - **Edited** — the id still resolves, but the verdict describes text that no longer
///     exists, so the panel would offer to delete a line on the strength of a judgement
///     about what it used to say.
///   - **Recycled** — `snippets.id` is `INTEGER PRIMARY KEY` with no `AUTOINCREMENT`
///     (migration 0013, carried through 0026), so it's a plain rowid alias: delete the
///     highest-id row and the next insert is handed that same id. A stale id could then
///     resolve to an unrelated new snippet, which the panel would show pre-ticked.
///
/// This mirrors the freshness guard `classify` already applies for the same reason
/// ("never stamp stale content"). Strictly conservative — it can only drop members, and
/// `parse_groups` then re-applies the two-or-more rule, so a group hollowed out by an
/// edit disappears rather than shrinking to a group of one.
async fn still_current(
    app: AppHandle,
    analyzed: Vec<(i64, String)>,
) -> Result<Vec<Option<RedundancyMember>>, String> {
    tokio::task::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let conn = st.conn.lock().map_err(|e| e.to_string())?;
        let current: HashMap<i64, String> = repository::list_approved(&conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| (s.id, s.content))
            .collect();
        Ok::<_, String>(
            analyzed
                .into_iter()
                .map(|(id, content)| {
                    let analyzed = content.trim().to_string();
                    let unchanged =
                        current.get(&id).is_some_and(|now| now.trim() == analyzed);
                    unchanged.then(|| RedundancyMember { id, analyzed })
                })
                .collect(),
        )
    })
    .await
    .map_err(|e| {
        eprintln!("snippets: dedup re-read task panicked: {e}");
        "Couldn't re-read the snippet library.".to_string()
    })?
}

/// Turn the model's reply into groups of real snippet ids.
///
/// `ids` maps the 1-based indices the prompt presented back to rows, with a `None` slot
/// wherever that row no longer holds the text that was judged (see [`still_current`]) —
/// so a stale member drops out exactly like an out-of-range index does. Returns `None`
/// only when there's no JSON array at all (an unparseable reply — the caller surfaces
/// that as a failure); `Some(vec![])` is the meaningful, common answer that nothing is
/// redundant.
///
/// Every group is filtered rather than trusted: out-of-range indices are dropped, an
/// index repeated inside a group counts once, a snippet already claimed by an earlier
/// group is skipped, an implausibly large group is discarded whole (see
/// [`MAX_GROUP_MEMBERS`]), and what's left must still be two or more snippets. An absent
/// or nonsensical `keep` falls back to the group's first member — the group is real
/// either way, and dropping it over a bad suggestion would hide a genuine duplicate.
fn parse_groups(raw: &str, slots: &[Option<RedundancyMember>]) -> Option<Vec<RedundancyGroup>> {
    let items = ai::parse::json_array(raw)?;
    let mut groups = Vec::new();
    // Ids already claimed by an earlier group — the "at most one group" invariant.
    let mut claimed: Vec<i64> = Vec::new();

    for item in items {
        let Some(listed) = item.get("snippets").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut members: Vec<&RedundancyMember> = Vec::new();
        for index in listed {
            let Some(member) = index.as_i64().and_then(|i| resolve(i, slots)) else {
                continue; // not a number, an index outside the library, or a stale row
            };
            if members.iter().any(|m| m.id == member.id) || claimed.contains(&member.id) {
                continue; // repeated within this group, or already in an earlier one
            }
            members.push(member);
        }
        if members.len() < 2 {
            continue; // not a duplicate pair (any more) — nothing to review
        }
        if members.len() > MAX_GROUP_MEMBERS {
            eprintln!(
                "snippets: dedup dropped a group of {} — over the {MAX_GROUP_MEMBERS} cap",
                members.len()
            );
            continue;
        }

        // The keeper must be one of this group's own members; anything else (missing,
        // out of range, or pointing at a snippet in a different group) falls back to
        // the first member.
        let keep_id = item
            .get("keep")
            .and_then(|v| v.as_i64())
            .and_then(|i| resolve(i, slots))
            .map(|m| m.id)
            .filter(|id| members.iter().any(|m| m.id == *id))
            .unwrap_or(members[0].id);

        let reason: String = item
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .chars()
            .take(MAX_NAME_LEN)
            .collect();

        claimed.extend(members.iter().map(|m| m.id));
        let members = members
            .into_iter()
            .map(|m| RedundancyMember { id: m.id, analyzed: m.analyzed.clone() })
            .collect();
        groups.push(RedundancyGroup { members, keep_id, reason });
    }
    Some(groups)
}

/// Resolve one 1-based index from the reply into the snippet id it refers to, or `None`
/// when it points outside the list that was sent — or at a row that has since changed,
/// which [`still_current`] has already blanked to `None`.
///
/// The `index < 1` guard is what keeps the cast safe: it runs before `as usize`, so a
/// zero or negative index (including `i64::MIN`) returns early rather than underflowing.
fn resolve(index: i64, slots: &[Option<RedundancyMember>]) -> Option<&RedundancyMember> {
    if index < 1 {
        return None;
    }
    slots.get(index as usize - 1)?.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One live slot: a row that still holds the text that was analyzed. Content is
    /// `c<id>`, which only has to be distinct per row.
    fn live(id: i64) -> Option<RedundancyMember> {
        Some(RedundancyMember { id, analyzed: format!("c{id}") })
    }

    /// Five live slots, with ids deliberately unlike their indices so a test can't pass
    /// by confusing the two.
    fn slots() -> Vec<Option<RedundancyMember>> {
        [101, 102, 103, 104, 105].into_iter().map(live).collect()
    }

    /// A group's member ids, for terse assertions.
    fn ids_of(group: &RedundancyGroup) -> Vec<i64> {
        group.members.iter().map(|m| m.id).collect()
    }

    #[test]
    fn resolves_one_based_indices_to_rows() {
        let raw = r#"[{"snippets": [1, 3], "keep": 3, "reason": "both state SOC2"}]"#;
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(ids_of(&groups[0]), vec![101, 103]);
        assert_eq!(groups[0].keep_id, 103);
        assert_eq!(groups[0].reason, "both state SOC2");
        // Each member carries the text that was judged, so the UI can re-check that the
        // row still says it for as long as the report is on screen.
        assert_eq!(groups[0].members[0].analyzed, "c101");
        assert_eq!(groups[0].members[1].analyzed, "c103");
    }

    #[test]
    fn tolerates_prose_and_fences() {
        let raw = "Sure:\n```json\n[{\"snippets\":[1,2],\"keep\":1,\"reason\":\"same ask\"}]\n```\n";
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(ids_of(&groups[0]), vec![101, 102]);
    }

    #[test]
    fn an_empty_array_means_nothing_is_redundant() {
        // Distinct from `None` — the library is fine, the check ran.
        assert_eq!(parse_groups("[]", &slots()), Some(vec![]));
    }

    #[test]
    fn returns_none_only_when_unparseable() {
        assert!(parse_groups("no json here", &slots()).is_none());
        assert!(parse_groups("", &slots()).is_none());
    }

    #[test]
    fn drops_out_of_range_indices_and_the_groups_they_hollow_out() {
        // 9 doesn't exist; 1 and 2 do, so this group survives at two members.
        let raw = r#"[{"snippets": [1, 9, 2], "keep": 1, "reason": "x"}]"#;
        assert_eq!(ids_of(&parse_groups(raw, &slots()).unwrap()[0]), vec![101, 102]);

        // Only one index survives — no longer a duplicate pair, so the group goes.
        let raw = r#"[{"snippets": [1, 9], "keep": 1, "reason": "x"}]"#;
        assert_eq!(parse_groups(raw, &slots()), Some(vec![]));

        // Zero and negatives are out of range, not wrapped around to the end.
        let raw = r#"[{"snippets": [0, -1, 1, 2], "keep": 0, "reason": "x"}]"#;
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(ids_of(&groups[0]), vec![101, 102]);
        assert_eq!(groups[0].keep_id, 101, "an out-of-range keep falls back");

        // `resolve`'s doc rests on the `index < 1` guard running BEFORE the `as usize`
        // cast, since that cast would otherwise wrap a negative into a huge index. The
        // extremes are what pin it: `i64::MIN` has no positive counterpart to negate, and
        // `i64::MAX` casts cleanly but is far past the end.
        let raw = format!(
            r#"[{{"snippets": [{}, {}, 1, 2], "keep": {}, "reason": "x"}}]"#,
            i64::MIN,
            i64::MAX,
            i64::MIN
        );
        let groups = parse_groups(&raw, &slots()).unwrap();
        assert_eq!(ids_of(&groups[0]), vec![101, 102]);
        assert_eq!(groups[0].keep_id, 101);
    }

    /// A row edited or deleted while the generation was in flight, or whose id was
    /// recycled onto a different row, arrives as a `None` slot and must drop out of its
    /// group exactly like an out-of-range index — otherwise the panel would offer to
    /// delete a line on the strength of a judgement about text it no longer holds.
    #[test]
    fn a_slot_that_went_stale_drops_out_of_its_group() {
        // Slot 2 went stale; 1 and 3 survive, so the group stands at two members.
        let stale = vec![live(101), None, live(103), live(104), live(105)];
        let raw = r#"[{"snippets": [1, 2, 3], "keep": 3, "reason": "x"}]"#;
        let groups = parse_groups(raw, &stale).unwrap();
        assert_eq!(ids_of(&groups[0]), vec![101, 103]);
        assert_eq!(groups[0].keep_id, 103);

        // The group is hollowed out below two members — it disappears rather than
        // becoming a "duplicate" group of one.
        let raw = r#"[{"snippets": [1, 2], "keep": 1, "reason": "x"}]"#;
        assert_eq!(parse_groups(raw, &stale), Some(vec![]));

        // A stale keeper falls back to a surviving member, not to the stale id.
        let raw = r#"[{"snippets": [1, 2, 3], "keep": 2, "reason": "x"}]"#;
        assert_eq!(parse_groups(raw, &stale).unwrap()[0].keep_id, 101);
    }

    /// A dozen snippets isn't a duplicate, it's a model that decided a whole topic was
    /// one point — and the panel would render it as one button offering to delete all of
    /// it. Dropped whole rather than truncated: an arbitrary subset of an untrustworthy
    /// grouping is no more trustworthy.
    #[test]
    fn an_implausibly_large_group_is_dropped_whole() {
        let many: Vec<Option<RedundancyMember>> =
            (1..=MAX_GROUP_MEMBERS as i64 + 1).map(live).collect();
        let all: Vec<String> = (1..=many.len()).map(|i| i.to_string()).collect();
        let raw = format!(r#"[{{"snippets": [{}], "keep": 1}}]"#, all.join(","));
        assert_eq!(parse_groups(&raw, &many), Some(vec![]));

        // Exactly at the cap is still reported — the boundary is inclusive.
        let at_cap: Vec<String> = (1..=MAX_GROUP_MEMBERS).map(|i| i.to_string()).collect();
        let raw = format!(r#"[{{"snippets": [{}], "keep": 1}}]"#, at_cap.join(","));
        assert_eq!(
            parse_groups(&raw, &many).unwrap()[0].members.len(),
            MAX_GROUP_MEMBERS
        );
    }

    #[test]
    fn a_snippet_lands_in_at_most_one_group() {
        // Index 2 appears in both groups; the second group loses it and, left with one
        // member, disappears entirely.
        let raw = r#"[
            {"snippets": [1, 2], "keep": 1, "reason": "first"},
            {"snippets": [2, 3], "keep": 3, "reason": "second"}
        ]"#;
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(ids_of(&groups[0]), vec![101, 102]);

        // With a third member the second group survives, minus the claimed snippet.
        let raw = r#"[
            {"snippets": [1, 2], "keep": 1, "reason": "first"},
            {"snippets": [2, 3, 4], "keep": 3, "reason": "second"}
        ]"#;
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(ids_of(&groups[1]), vec![103, 104]);
    }

    #[test]
    fn an_index_repeated_within_a_group_counts_once() {
        let raw = r#"[{"snippets": [1, 1, 1], "keep": 1, "reason": "x"}]"#;
        // One real snippet — not a pair, so no group.
        assert_eq!(parse_groups(raw, &slots()), Some(vec![]));

        let raw = r#"[{"snippets": [1, 2, 2], "keep": 2, "reason": "x"}]"#;
        assert_eq!(ids_of(&parse_groups(raw, &slots()).unwrap()[0]), vec![101, 102]);
    }

    #[test]
    fn a_keep_outside_its_own_group_falls_back_to_the_first_member() {
        // Index 4 is a real snippet, but not in this group — the UI pre-ticks by
        // keeper, so a keeper it can't find would leave the group unusable.
        let raw = r#"[{"snippets": [1, 2], "keep": 4, "reason": "x"}]"#;
        assert_eq!(parse_groups(raw, &slots()).unwrap()[0].keep_id, 101);
        // Missing entirely, likewise.
        let raw = r#"[{"snippets": [2, 3]}]"#;
        assert_eq!(parse_groups(raw, &slots()).unwrap()[0].keep_id, 102);
    }

    #[test]
    fn skips_malformed_groups_without_losing_the_good_ones() {
        let raw = r#"[
            {"reason": "no snippets field"},
            {"snippets": "not an array"},
            {"snippets": [1, 2], "keep": 2, "reason": "good"},
            {"snippets": []}
        ]"#;
        let groups = parse_groups(raw, &slots()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].reason, "good");
    }

    #[test]
    fn a_missing_reason_is_empty_and_a_long_one_is_bounded() {
        let raw = r#"[{"snippets": [1, 2], "keep": 1}]"#;
        assert_eq!(parse_groups(raw, &slots()).unwrap()[0].reason, "");

        let long = "x".repeat(MAX_NAME_LEN + 50);
        let raw = format!(r#"[{{"snippets": [1, 2], "keep": 1, "reason": "{long}"}}]"#);
        assert_eq!(
            parse_groups(&raw, &slots()).unwrap()[0].reason.chars().count(),
            MAX_NAME_LEN
        );
    }
}
