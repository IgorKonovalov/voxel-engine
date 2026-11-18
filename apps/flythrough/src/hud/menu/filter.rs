//! The palette: the entries of the whole tree whose label, mode in words or group name
//! contain every word of the filter, case-insensitive, in the tree's order. A plain
//! substring per word, no fuzzy scoring, so a filter's matches are exact.

use super::{Entry, RunView, entries};
use crate::hud::Modes;

/// Whether `entry` at `m` and `v` matches every word of `filter`.
pub fn matches(entry: &Entry, filter: &str, m: &Modes, v: &RunView) -> bool {
    let value = entry.value(m, v).unwrap_or_default().to_lowercase();
    let label = entry.label.to_lowercase();
    let group = entry.group.name();
    filter
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|w| label.contains(&w) || value.contains(&w) || group.contains(&w))
}

/// The entries `filter` matches at `m` and `v`, in the tree's order; every entry for a filter
/// with no word.
pub fn palette(filter: &str, m: &Modes, v: &RunView) -> Vec<Entry> {
    entries().filter(|e| matches(e, filter, m, v)).collect()
}
