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
            created_at: row.get("created_at")?,
        })
    }
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
    fn color_for_position_rotates_and_wraps() {
        assert_eq!(color_for_position(0), "blue");
        assert_eq!(color_for_position(3), "purple");
        assert_eq!(color_for_position(8), "blue"); // wraps
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
