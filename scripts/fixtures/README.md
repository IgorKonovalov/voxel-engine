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

## `index-rows/`: for `check-index-rows.mjs`

```
node scripts/check-index-rows.mjs scripts/fixtures/index-rows
```

Expect **exit 0**: this tree passes rather than fails, and it is half the instrument. A byte cap is
trivially red on a tree with a fat row in it, so the assertion here is the reverse: the gate
measures the rows it should and stays silent about everything else. Green means all four hold:

| Case | Seeded in `roster.md` as | Expected |
|------|--------------------------|----------|
| a row inside a region, at width | a 258-byte table row | measured, not reported |
| a second region in the same file | a bullet region below the table region | **3** regions counted in the file, proving the count is a count |
| a row outside every region | a 1,100-odd-byte unmarked table row | not measured at all |
| a fenced row inside a region | an over-cap row inside a ```` ```markdown ```` block | not measured; a document describing a roster is not one |

The third case is the gate's documented hole: a row moved past a marker escapes silently. The
per-file region count printed on success is the mitigation, so a deleted marker shows up as a
region that vanished.

## `index-rows-red/`: the same gate's other half

```
node scripts/check-index-rows.mjs scripts/fixtures/index-rows-red
```

Expect **exit 1 and exactly three breaks**, one over cap and two of the wrong shape:

| Case in `roster.md` | Expected |
|---------------------|----------|
| a 437-byte table row inside a `roster:begin cap=320` region | reported as `roster.md:25  437 bytes (cap 320)` |
| a correctly sized row in the same region | measured, not reported |
| a bullet row in a table region, well under cap, above the table header | reported as `roster.md:47  a bullet row in a table region (expected a table row; ...)` |
| the byte-identical bullet in a bullet region | not reported; the check is the row's form against its region's majority |
| a region holding one row of each form | reported once, at the region's opening line 74, naming both counts |

The two trees are siblings, each its own root, so neither run reaches the other's rows.

## `--self-test`: the counts are asserted, not printed

```
node scripts/check-index-rows.mjs --self-test    # expects exit 0, 9 of 9
```

Exit 0 from the green tree and from the repository is not on its own an assertion: a detector whose
row regexes stopped matching reports `0 rows, 0 over cap` and exits 0 from both. The self-test
asserts the green tree's exact 3 regions and 4 rows with nothing reported; the red tree's exact 4
regions, 8 rows, 1 over cap and 2 misshaped, with the over-cap line equal to the one above and each
shape report in its printed form; and a floor of 5 rows in each of `docs/adrs/README.md` and
`docs/plans/README.md` and 15 across the repository, which a dead detector takes to zero.
