// Fixture source for scripts/check-comment-hygiene.mjs. Not compiled — no
// pipeline includes this file, and the fixture tree is skipped on a repo walk.
//
// Two seeded findings and two silences, which prove WGSL is read through the
// Rust lexer: a `//` comment, a `/* */` comment that nests, a bare citation and
// the escape. Enumerated in ../README.md.

// SEEDED — a relative link in a WGSL comment. Broken on purpose, since a link
// that resolves rots exactly the same way.
//
// [the march]: ../../../docs/adrs/0001-not-a-real-path.md
const SEEDED_STEP: f32 = 0.5;

/* SEEDED — a block comment that nests, as WGSL's do. The /* inner */ pair
   closes first and the comment carries on to the close below, so the
   relative link [after the inner close](../not/a/real/path.md) is still in a
   comment and is reported. A lexer that stopped at the first close would read
   this line as code and stay silent, which is what this finding pins. */
const SEEDED_NESTED: f32 = 1.0;

// SILENT — a bare-number citation, the form the links are replaced with.
// The step is the brick edge over eight (ADR-0001, Plan 0001 Phase 3).
const SILENT_CITATION: f32 = 2.0;

// SILENT — an escaped false positive, covering its own line and the next.
//
// hygiene-allow: quoting the vocabulary the gate rejects
// The shadow ray used to start at the hit, which this plan moved.
const SILENT_ESCAPED: f32 = 3.0;
