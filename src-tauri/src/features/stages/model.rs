use rusqlite::Row;
use serde::Serialize;

/// A stage of the one shared pipeline. `kind` is `"standard"` or `"messaging"`;
/// the pipeline has exactly one messaging stage, always first.
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Stage {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub position: i64,
    /// Palette token (see `STAGE_COLORS`) — the stage's color in the pipeline
    /// and list. A token, not a hex, so the frontend re-themes it per mode.
    pub color: String,
    /// What this step of the cycle is FOR — what has to become true before a
    /// prospect belongs in the next stage. Read by two things: the draft
    /// composer (where the thread sits) and the advance analyzer (has this been
    /// met?). Empty is fine and common — a stage with no goal simply doesn't
    /// steer drafts and is never auto-advanced out of.
    ///
    /// Distinct from `customers.goal`, which is the *buyer's* destination across
    /// the whole funnel. This is the step; that is the journey.
    pub goal: String,
    /// Days of silence from your side before a card in this stage is nudged
    /// (yellow), then flagged as rotting (red). Per-stage because a freshly
    /// messaged prospect goes cold faster than one mid-onboarding.
    pub warn_days: i64,
    pub stale_days: i64,
    pub created_at: String,
}

impl Stage {
    /// Map a DB row (columns as selected by the repository) into a `Stage`.
    pub(super) fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Stage {
            id: row.get("id")?,
            name: row.get("name")?,
            kind: row.get("kind")?,
            position: row.get("position")?,
            color: row.get("color")?,
            goal: row.get("goal")?,
            warn_days: row.get("warn_days")?,
            stale_days: row.get("stale_days")?,
            created_at: row.get("created_at")?,
        })
    }
}

/// Bounds on a stage's staleness thresholds. The floor is 1 (a card can't be
/// stale the instant you message someone — that would light up the whole board),
/// and the ceiling is a year, generous enough for any real cadence while still
/// rejecting a fat-fingered paste. Kept here beside the model so the command
/// layer and the migration default read from one place.
pub const MIN_STALENESS_DAYS: i64 = 1;
pub const MAX_STALENESS_DAYS: i64 = 365;

/// Validate a `(warn_days, stale_days)` pair. Both must sit in range and warn
/// must come strictly first — equal or inverted thresholds would make the yellow
/// band unreachable, so a card would jump straight from fine to rotting.
pub(crate) fn validate_thresholds(warn_days: i64, stale_days: i64) -> Result<(), String> {
    for (label, value) in [("Warn days", warn_days), ("Stale days", stale_days)] {
        if !(MIN_STALENESS_DAYS..=MAX_STALENESS_DAYS).contains(&value) {
            return Err(format!(
                "{label} must be between {MIN_STALENESS_DAYS} and {MAX_STALENESS_DAYS}."
            ));
        }
    }
    if warn_days >= stale_days {
        return Err("Warn days must be fewer than stale days.".into());
    }
    Ok(())
}

/// The `"messaging"` stage kind — the single, always-first stage that tracks a
/// prospect's `messages_sent` counter.
pub const KIND_MESSAGING: &str = "messaging";
/// The `"standard"` stage kind — an ordinary funnel step.
pub const KIND_STANDARD: &str = "standard";

/// The palette tokens a stage color may be. Mirrors the `--stage-<token>` CSS
/// variables in `global.css`. Order is the default rotation for new stages.
///
/// This is the source of truth. Two hand-kept copies must stay in step, both
/// guarded: the TS `STAGE_COLORS` in `src/api/stages.ts` (Rust↔TS can't share),
/// and migration `0006`'s `CASE` backfill (pinned by
/// `migration_0006_case_matches_color_for_position` below).
pub const STAGE_COLORS: &[&str] = &[
    "blue", "amber", "green", "purple", "teal", "pink", "red", "gray",
];

/// Whether `color` is a known palette token.
pub(crate) fn is_valid_color(color: &str) -> bool {
    STAGE_COLORS.contains(&color)
}

/// The default color for a stage at `position` — rotates through the palette so
/// a fresh pipeline / appended stage gets a distinct color. Matches migration
/// 0006's backfill.
pub(crate) fn color_for_position(position: i64) -> &'static str {
    let len = STAGE_COLORS.len() as i64;
    STAGE_COLORS[position.rem_euclid(len) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_thresholds_rejects_out_of_range_and_inverted_pairs() {
        assert!(validate_thresholds(3, 7).is_ok());
        assert!(validate_thresholds(1, 365).is_ok());
        assert!(validate_thresholds(0, 7).is_err(), "below the floor");
        assert!(validate_thresholds(3, 366).is_err(), "above the ceiling");
        assert!(validate_thresholds(7, 3).is_err(), "inverted");
        // Equal thresholds would leave no yellow band at all — a card would go
        // straight from fine to rotting, which is not a state the board can show.
        assert!(validate_thresholds(5, 5).is_err(), "equal leaves no warn band");
    }

    #[test]
    fn color_for_position_rotates_and_wraps() {
        assert_eq!(color_for_position(0), "blue");
        assert_eq!(color_for_position(3), "purple");
        assert_eq!(color_for_position(8), "blue"); // wraps
    }

    /// The staleness bounds are a second hand-kept mirror in `src/api/stages.ts`, used
    /// there to clamp the day inputs. Unpinned, widening the Rust range left the UI
    /// silently clamping at the old ceiling — so the user could not type a value the
    /// backend would have accepted, with no error to explain why.
    #[test]
    fn ts_staleness_bounds_mirror_the_rust_limits() {
        const TS: &str = include_str!("../../../../src/api/stages.ts");
        let number_after = |decl: &str| -> i64 {
            TS.split(decl)
                .nth(1)
                .unwrap_or_else(|| panic!("stages.ts declares {decl}"))
                .split(';')
                .next()
                .expect("the declaration is terminated")
                .trim()
                .parse()
                .expect("the bound is a plain integer literal")
        };
        assert_eq!(
            number_after("MIN_STALENESS_DAYS = "),
            MIN_STALENESS_DAYS,
            "src/api/stages.ts MIN_STALENESS_DAYS must match the Rust limit"
        );
        assert_eq!(
            number_after("MAX_STALENESS_DAYS = "),
            MAX_STALENESS_DAYS,
            "src/api/stages.ts MAX_STALENESS_DAYS must match the Rust limit"
        );
    }

    /// The stage editor hard-codes the cadence a newly-added stage starts with, mirroring
    /// migration 0027's column defaults so a stage created in the UI matches one created by
    /// any other path. A third hand-kept mirror; pin it like the other two.
    #[test]
    fn ts_new_stage_cadence_matches_the_migration_defaults() {
        const SQL: &str = include_str!("../../database/migrations/0027_stage_goals.sql");
        const TSX: &str = include_str!("../../../../src/prospects/StageEditor.tsx");
        for (column, ts_field) in [("warn_days", "warnDays"), ("stale_days", "staleDays")] {
            let sql_default: i64 = SQL
                .split(&format!("ADD COLUMN {column} INTEGER NOT NULL DEFAULT "))
                .nth(1)
                .unwrap_or_else(|| panic!("0027 gives {column} a default"))
                .split(';')
                .next()
                .expect("the statement is terminated")
                .trim()
                .parse()
                .expect("the default is a plain integer literal");
            // The name also appears as a type annotation and as a parameter, so collect
            // every `<field>: <token>` and keep the ones that are integer literals. There
            // should be exactly one — the value a newly-added stage starts with.
            let literals: Vec<i64> = TSX
                .split(&format!("{ts_field}: "))
                .skip(1)
                .filter_map(|rest| {
                    rest.split([',', '}', '\n'])
                        .next()?
                        .trim()
                        .parse()
                        .ok()
                })
                .collect();
            assert_eq!(
                literals.as_slice(),
                [sql_default],
                "StageEditor.tsx must set {ts_field} exactly once, to 0027's {column} \
                 default ({sql_default})"
            );
        }
    }

    /// The TS palette (`STAGE_COLORS` in `src/api/stages.ts`) is a hand-kept
    /// mirror of this Rust source of truth — Rust and TS can't share the literal.
    /// The stage editor picks a newly-added stage's color in TS by the same
    /// rotation the backend applies on append, so a silent drift would show the
    /// user one color and persist another. Pin them so editing one list without
    /// the other fails here instead of shipping a mismatch.
    #[test]
    fn ts_stage_colors_mirror_the_rust_palette() {
        const TS: &str = include_str!("../../../../src/api/stages.ts");
        let decl = TS
            .split("STAGE_COLORS: StageColor[] = [")
            .nth(1)
            .expect("stages.ts declares STAGE_COLORS");
        let block = decl.split(']').next().expect("the array literal is closed");
        // Pull the quoted tokens out of the array, in order.
        let colors: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
        assert_eq!(
            colors.as_slice(),
            STAGE_COLORS,
            "src/api/stages.ts STAGE_COLORS must match the Rust palette (same tokens, same order)"
        );
    }

    /// Migration 0006 backfills `color` with a hand-written `CASE (position % 8)`
    /// — a second encoding of `color_for_position`. Pin them together so editing
    /// one without the other fails loudly here instead of shipping a mismatch.
    #[test]
    fn migration_0006_case_matches_color_for_position() {
        const SQL: &str = include_str!("../../database/migrations/0006_add_stage_color.sql");
        // The CASE lists one color literal per position (WHEN 0..6, then ELSE for
        // 7), in order. Pull the quoted literals out of the CASE block.
        let case = SQL.split("CASE").nth(1).expect("migration has a CASE block");
        let colors: Vec<&str> = case.split('\'').skip(1).step_by(2).collect();
        assert_eq!(colors.len(), STAGE_COLORS.len(), "one literal per palette slot");
        for (position, color) in colors.iter().enumerate() {
            assert_eq!(*color, color_for_position(position as i64));
        }
    }

    /// Migration 0024 seeds the Full-cycle pipeline (colors written out by hand
    /// in SQL) when there was no pitch to inherit one from. That's now the ONLY
    /// place a pipeline is born, so its colors must still agree with the palette
    /// rotation — otherwise a fresh install's board is colored differently from
    /// every stage the user adds afterwards.
    #[test]
    fn migration_0024_template_colors_match_the_palette_rotation() {
        const SQL: &str = include_str!("../../database/migrations/0024_one_pipeline.sql");
        // The template is the SELECT/UNION block naming a color per position, in
        // order. Take the seed statement and pull its quoted literals: each row
        // contributes name, kind, then color.
        let seed = SQL
            .split("INSERT INTO stages_new (name, kind, position, color)")
            .nth(1)
            .expect("0024 seeds a template pipeline");
        let block = seed.split(')').next().expect("the seed subquery is closed");
        let literals: Vec<&str> = block.split('\'').skip(1).step_by(2).collect();
        // name, kind, color per row — the color is every third literal.
        let colors: Vec<&str> = literals.iter().skip(2).step_by(3).copied().collect();
        assert!(!colors.is_empty(), "found the template's color literals");
        for (position, color) in colors.iter().enumerate() {
            assert_eq!(
                *color,
                color_for_position(position as i64),
                "0024's seeded stage at position {position} must use the palette color"
            );
        }
    }
}
