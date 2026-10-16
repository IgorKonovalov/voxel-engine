# scripts/fixtures: the trees the repository gates bite on

The gates in `scripts/` take an optional `root` argument so they can be run against a tree other
than this repository. Each subdirectory here is such a tree, and most files in it are
**deliberately wrong in a named way**, so that "the gate still catches things" is a command anyone
can run rather than a property nobody has re-tested since the day it was written.

The gates skip this directory **by path** (`scripts/fixtures`, named once in each script) on an
ordinary run over the repository, and scan a subdirectory when it **is** the root, which is the
only way its seeded breaks are reachable. Both gates enumerate files with `git ls-files`, so a
fixture file is read only once it is tracked.

The trees are kept per gate, because a shared fixture that drifts into serving one gate stops
being an instrument for the other. The gates and their trees are copied from Ritmolux (the owner's
sibling repository, commit `ad375596`); ADR and backlog numbers written inside them with a
`Ritmolux` prefix are that repository's.

## `comment-hygiene/`: for `check-comment-hygiene.mjs`

```
node scripts/check-comment-hygiene.mjs scripts/fixtures/comment-hygiene
```

Expect **exit 1 and exactly thirteen findings across five files**, one per line as
`file:line -> reason`, and nothing else:

| File | Findings | Seeded as |
|------|---------:|-----------|
| `seeded.rs` | 2 | line 10, a relative link definition; line 26, the phrase `this plan` |
| `seeded-elapsed.rs` | 5 | lines 17 to 21, one per elapsed-time preposition in front of a numbered citation: `before` / `since` / `until` / `after` / `pre-` |
| `seeded-residue.rs` | 1 | line 14, the phrase `any more`, in a sentence explaining why something is absent |
| `seeded-literal.rs` | 3 | lines 13, 19 and 30, a string literal carrying a run of 12 or more spaces mid-sentence: one rejoined onto a single line, one at the width a continuation indent produces, and one still unrejoined |
| `seeded.wgsl` | 2 | line 11, a relative link in a `//` comment; line 16, a relative link after the inner close of a nested `/* */` comment |

One rejected form per line, and the counts are the instrument: a seed that stopped biting shows up
as a number that moved rather than as a silence nobody noticed. `seeded-elapsed.rs` seeds five
lines because the pattern is an alternation, and a dropped branch is the regression a single line
cannot see.

`seeded.wgsl` is what proves WGSL is read, and that it is read through the Rust lexer's rules. Its
second finding sits after the inner `*/` of a nested block comment: a lexer that closed the
comment at the first `*/` would read that line as code and report twelve.

The silences matter as much as the findings, because a gate that cries wolf gets escaped rather
than obeyed:

| Case | Seeded in | Expected |
|------|-----------|----------|
| a rustdoc intra-doc link | `seeded.rs`: `[Seeded::render]`, `[the renderer](crate::render::Renderer)` | not reported; `rustc` resolves these, so they cannot rot silently |
| a bare-number citation | `seeded.rs`, `seeded-elapsed.rs`, `seeded.wgsl` | not reported; this is the form links and datings are replaced *with* |
| the citation as a sentence's subject | `seeded.rs`: `the Plan 0045 Phase 4b defect` | not reported; `the plan` is exempt in front of a number |
| a preposition with no citation behind it | `seeded-elapsed.rs`: `after the warp and before the blur chain` | not reported; describing pass order stays legal |
| the two words adjacent across a phrase boundary | `seeded-residue.rs`: `any input with more stops` | not reported |
| an escaped false positive | `seeded.rs` line 34, `seeded-elapsed.rs` line 45, `seeded.wgsl` line 27: `hygiene-allow: <reason>` | not reported; the escape covers its own line and the next, so the narration on the line below each marker is silent |
| a comment marker inside a string | `seeded.rs`, `seeded-elapsed.rs`: a URL, a raw string, an escaped `"` | not reported; the gate lexes source rather than grepping lines |
| a correct `\` continuation | `seeded-literal.rs`: a literal wrapped across two lines with the escape present | not reported; the escape removes the newline and the next line's indent |
| a formatted block | `seeded-literal.rs`: a column table carrying a `\n` | not reported; a table's rows start at a column, so no line break in it is followed by a 12-space run |
| hand-typed alignment under twelve | `seeded-literal.rs`: `note     : ...` | not reported; nine spaces is under the width rule |

An escape with **no reason after the colon** is itself a finding. That is not seeded, since it
would make the counts above wrong, but it is asserted by the gate's own header.
