//! The README quotes the minimal window example: the `rust` fence after the README's marker
//! line is the span between the example's `readme:` marker comments, byte for byte, so the two
//! cannot drift.

const README: &str = include_str!("../../../README.md");
const EXAMPLE: &str = include_str!("../examples/minimal_window.rs");

/// The README line that stands right before the quoted fence.
const README_MARKER: &str =
    "<!-- quoted from crates/svet-app/examples/minimal_window.rs, between its readme markers -->";

/// The README's fence holds at most this many lines.
const MAX_FENCE_LINES: usize = 30;

/// The text between `begin` and `end` lines of `src`, each found once.
fn between<'a>(src: &'a str, begin: &str, end: &str) -> &'a str {
    assert_eq!(src.matches(begin).count(), 1, "{begin:?} once");
    let start = src.find(begin).expect("counted") + begin.len();
    let len = src[start..]
        .find(end)
        .unwrap_or_else(|| panic!("{end:?} after {begin:?}"));
    &src[start..start + len]
}

#[test]
fn the_readme_fence_is_the_examples_marked_span() {
    let span = between(EXAMPLE, "// readme:begin\n", "// readme:end\n");
    let fence = between(README, &format!("{README_MARKER}\n```rust\n"), "```\n");
    assert_eq!(fence, span);
    let lines = fence.lines().count();
    assert!(
        (1..=MAX_FENCE_LINES).contains(&lines),
        "{lines} lines in the fence"
    );
}
