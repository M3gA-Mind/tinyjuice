//! The `⟦tj:<hash>⟧` recovery marker: pure parsing of what a model passes in a
//! tool's `handle` slot. No I/O; hosts decide what a path-shaped argument means.

/// Opening delimiter of the recovery marker.
pub const MARKER_PREFIX: &str = "⟦tj:";
/// Closing delimiter of the recovery marker.
pub const MARKER_SUFFIX: char = '⟧';
/// Longest handle accepted. Real handles are 32 hex characters.
pub const MAX_HANDLE_LEN: usize = 64;

/// What the model passed in the `handle` slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleArg {
    /// A handle (bare or marker form), normalized.
    Handle(String),
    /// Something path-shaped, possibly a persisted tool-result artifact.
    Path(String),
    /// Neither: a tool call id, prose, an over-long token.
    Invalid,
}

/// Accept a bare handle or the `⟦tj:<handle>⟧` marker form; `None` when the
/// result is empty, longer than [`MAX_HANDLE_LEN`], or not ASCII alphanumeric.
#[must_use]
pub fn parse(raw: &str) -> Option<&str> {
    let handle = raw
        .trim()
        .trim_start_matches(MARKER_PREFIX)
        .trim_end_matches(MARKER_SUFFIX)
        .trim();
    let valid = !handle.is_empty()
        && handle.len() <= MAX_HANDLE_LEN
        && handle.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some(handle)
}

/// Classify a `handle` argument as a handle, a path-shaped value, or invalid.
#[must_use]
pub fn classify(raw: &str) -> HandleArg {
    let trimmed = raw.trim();
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.ends_with(".txt") {
        return HandleArg::Path(trimmed.to_string());
    }
    match parse(trimmed) {
        Some(handle) => HandleArg::Handle(handle.to_string()),
        None => HandleArg::Invalid,
    }
}

#[cfg(test)]
#[path = "handle_tests.rs"]
mod tests;
