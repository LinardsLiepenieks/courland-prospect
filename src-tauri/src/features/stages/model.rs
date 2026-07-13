use rusqlite::Row;
use serde::{Deserialize, Serialize};

/// A pipeline stage belonging to a pitch. `kind` is `"standard"` or
/// `"messaging"`; a pipeline has exactly one messaging stage, always first.
/// Output-only — returned by commands, never accepted as input.
#[derive(Debug, Serialize)]
pub struct Stage {
    pub id: i64,
    pub pitch_id: i64,
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
            pitch_id: row.get("pitch_id")?,
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

/// A stage as supplied by the frontend when seeding a pitch's pipeline at
/// creation. `position` is the array index; `validate_inputs` enforces ordering
/// (exactly one messaging stage, first) and a valid color.
#[derive(Debug, Deserialize)]
pub struct StageInput {
    pub name: String,
    pub kind: String,
    pub color: String,
}

/// Trim + validate a user-supplied pipeline: non-empty names, known kinds, and
/// exactly one messaging stage that sits first. Returns the cleaned list. Owned
/// by the stages feature since it enforces stage-domain invariants; callers
/// (e.g. `create_pitch`) delegate here rather than re-implementing the rules.
pub(crate) fn validate_inputs(input: Vec<StageInput>) -> Result<Vec<StageInput>, String> {
    let mut cleaned = Vec::with_capacity(input.len());
    let mut messaging_count = 0;
    for (i, stage) in input.into_iter().enumerate() {
        let name = stage.name.trim().to_string();
        if name.is_empty() {
            return Err("Stage names can't be empty.".into());
        }
        let kind = match stage.kind.as_str() {
            KIND_MESSAGING => {
                if i != 0 {
                    return Err("The messaging stage must be first.".into());
                }
                messaging_count += 1;
                KIND_MESSAGING
            }
            KIND_STANDARD => KIND_STANDARD,
            other => return Err(format!("Unknown stage kind: {other}")),
        };
        if !is_valid_color(&stage.color) {
            return Err(format!("Unknown stage color: {}", stage.color));
        }
        cleaned.push(StageInput {
            name,
            kind: kind.to_string(),
            color: stage.color,
        });
    }
    if messaging_count != 1 {
        return Err("A pipeline needs exactly one messaging stage.".into());
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(name: &str, kind: &str) -> StageInput {
        StageInput {
            name: name.into(),
            kind: kind.into(),
            color: "blue".into(),
        }
    }

    #[test]
    fn validates_and_trims_a_good_pipeline() {
        let out = validate_inputs(vec![
            stage("  Messaged ", KIND_MESSAGING),
            stage("Meeting", KIND_STANDARD),
        ])
        .unwrap();
        assert_eq!(out[0].name, "Messaged"); // trimmed
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn rejects_empty_name() {
        assert!(validate_inputs(vec![
            stage("Messaged", KIND_MESSAGING),
            stage("  ", KIND_STANDARD)
        ])
        .is_err());
    }

    #[test]
    fn rejects_messaging_not_first() {
        assert!(validate_inputs(vec![
            stage("Meeting", KIND_STANDARD),
            stage("Messaged", KIND_MESSAGING)
        ])
        .is_err());
    }

    #[test]
    fn rejects_zero_or_multiple_messaging() {
        assert!(validate_inputs(vec![stage("Meeting", KIND_STANDARD)]).is_err());
        assert!(validate_inputs(vec![
            stage("Messaged", KIND_MESSAGING),
            stage("Second", KIND_MESSAGING),
        ])
        .is_err());
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(validate_inputs(vec![stage("Weird", "phone-call")]).is_err());
    }

    #[test]
    fn rejects_unknown_color() {
        let bad = StageInput {
            name: "Messaged".into(),
            kind: KIND_MESSAGING.into(),
            color: "chartreuse".into(),
        };
        assert!(validate_inputs(vec![bad]).is_err());
    }

    #[test]
    fn color_for_position_rotates_and_wraps() {
        assert_eq!(color_for_position(0), "blue");
        assert_eq!(color_for_position(3), "purple");
        assert_eq!(color_for_position(8), "blue"); // wraps
    }

    /// The TS palette (`STAGE_COLORS` in `src/api/stages.ts`) is a hand-kept
    /// mirror of this Rust source of truth — Rust and TS can't share the literal.
    /// The create flow computes a new stage's color in TS, so a silent drift
    /// would persist colors that disagree with the backend rotation. Pin them so
    /// editing one list without the other fails here instead of shipping a
    /// mismatch.
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
}
