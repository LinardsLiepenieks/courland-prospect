//! Small cross-cutting helpers that don't belong to a single feature or to the
//! database/ingest infra — leaf utilities safe for anything to depend on.

use rand::distr::Alphanumeric;
use rand::Rng;

/// A random `n`-char alphanumeric string. Used for the ingest shared token and
/// for capture-profile sandbox slugs.
pub fn random_alphanumeric(n: usize) -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(n)
        .map(char::from)
        .collect()
}

/// Upper bound (in characters) on a short single-line field — pitch/stage/
/// snippet names, a prospect's name/headline.
pub const MAX_NAME_LEN: usize = 200;

/// Upper bound (in characters) on a long free-text field — skill/profile copy,
/// snippet content, notes, message bodies. Generous: real content never nears
/// it. Guards against a pathological paste bloating the DB or (for AI-bound
/// fields) ballooning a CLI argument.
pub const MAX_TEXT_LEN: usize = 20_000;

/// Trim `value` and reject it when it exceeds `max` characters, naming the
/// `field` in the error. Returns the trimmed slice so callers keep the trim.
/// Centralizes the "validate length at the entry point" rule the command and
/// ingest layers share.
pub fn bounded<'a>(value: &'a str, max: usize, field: &str) -> Result<&'a str, String> {
    let trimmed = value.trim();
    if trimmed.chars().count() > max {
        return Err(format!("{field} is too long (max {max} characters)."));
    }
    Ok(trimmed)
}
