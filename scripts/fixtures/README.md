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

## `test-tiers/`: for `check-test-tiers.mjs`

```
node scripts/check-test-tiers.mjs scripts/fixtures/test-tiers
```

Expect **exit 1 and exactly seven findings**, then the count line `7 finding(s) across 9 test
binaries` (seven test files, the `still` example and the `app` binary). The tree is two packages (`gfx`, `app`) and a `.config/nextest.toml` whose tiers select GPU binaries by
`binary(/^gpu_/)` and keep readings out of soak and full by `test(/(^|::)reading::/)`:

| Case | Seeded as | Expected |
|------|-----------|----------|
| a GPU test file without the prefix | `apps/app/tests/missing.rs` opens `wgpu::Instance` and calls `request_adapter` | `apps/app/tests/missing.rs -> opens a GPU device but its test binary \`app::missing\` lacks the gpu_ prefix: ...` |
| a file that reaches a device only through `support` | `apps/app/tests/via_support.rs` declares `mod support;`, whose `mod.rs` names `software_device` | `apps/app/tests/via_support.rs -> opens a GPU device but ...` |
| an example marked `test = true` whose test module opens a device | `apps/app/examples/still.rs` | `apps/app/examples/still.rs -> its tests open a GPU device but the example \`app::example/still\` lacks the gpu_ prefix: ...` |
| a binary's module whose test module opens a device | `apps/app/src/draw.rs`, declared by `src/main.rs` | `apps/app/src/draw.rs -> its tests open a GPU device inside the binary \`app::bin/app\`, which cannot carry the gpu_ prefix: ...` |
| a profile that drifted | `[profile.ci-windows]`'s default-filter is `all()` | `.config/nextest.toml -> [profile.ci-windows] does not have default-filter 'not binary(/^gpu_/)'` |
| an `#[ignore]` neither soak nor a reading | `apps/app/tests/cpu.rs`'s `stray`, `#[ignore = "slow"]` | `apps/app/tests/cpu.rs:12 -> an #[ignore] that is neither "soak" nor inside \`mod reading\`: ...` |
| a file over the env-name cap | `apps/app/tests/env_five.rs` names five distinct `VXE_*` literals through a wrapper `env`, the first twice | `apps/app/tests/env_five.rs -> names 5 VXE_* environment variables, over the cap of 4: ... (VXE_FIVE, VXE_FOUR, VXE_ONE, VXE_THREE, VXE_TWO)` |
| a soak mark, and readings | `cpu.rs`'s `slow_one` (`#[ignore = "soak"]`) and its `mod reading`, one test with a reason and one bare, braces inside a string between them; `#[ignore]` in a block comment | not reported |
| a binary or example that opens a device outside its tests | `src/main.rs`'s and `still.rs`'s `main`, `examples/plain.rs` (no `test = true`) | not reported; only `#[cfg(test)]` items are read there |
| `--headless` in a binary's own test module, nothing spawned | `src/main.rs`'s `parses` | not reported; there it counts only beside `Command::new` |
| a GPU file with the prefix | `apps/app/tests/gpu_golden.rs`, `crates/gfx/tests/gpu_draw.rs` | not reported |
| a file at the env-name cap | `apps/app/tests/env_four.rs`: four distinct names, one twice, a fifth in a comment; `env_five.rs`'s comment names a sixth | not reported; a literal is counted once and comments are stripped |
| a CPU-only file | `apps/app/tests/cpu.rs` names every entry point in comments only, and a local `device` | not reported; comments are stripped before matching |

The other profiles are green here (`quick` and `gpu` are the prefix filter and its negation,
`gate` has none, `soak` and `full` leave out `reading::`), so the seven lines above are the whole
output.

## `file-size/`: for `check-file-size.mjs`

```
node scripts/check-file-size.mjs scripts/fixtures/file-size
```

Expect **exit 1 and exactly three findings**, then the count line `3 file(s) over their cap`. The
tree is one crate, `crates/seeded`, whose files are a seed comment padded to the line counts named:

| Case | Seeded as | Expected |
|------|-----------|----------|
| a source file over the source cap | `src/over.rs`, 1201 lines | `crates/seeded/src/over.rs -> 1201 lines, over the source cap of 1200` |
| a test file over the test cap | `tests/over.rs`, 2001 lines | `crates/seeded/tests/over.rs -> 2001 lines, over the test cap of 2000` |
| a `tests/support/` file over the test cap | `tests/support/mod.rs`, 2001 lines | `crates/seeded/tests/support/mod.rs -> 2001 lines, over the test cap of 2000` |
| a file exactly at the cap | `src/at_cap.rs`, 1200 lines | not reported; the cap is inclusive |

## `test-times/`: for `test-times.mjs`

```
node scripts/test-times.mjs scripts/fixtures/test-times/seeded.log
```

Expect **exit 1 and exactly one test over the line**, printed as
`12.0 flythrough::gpu_castle_golden castle_gate_golden -> over the 10 s line: ...`. The seeded
log is a nextest `--status-level pass` run of four tests: one CPU test at 0.012 s, one at 9.950 s
under the line, one at exactly 10.000 s (on the line, not over it) and the one at 12.000 s. With
`--over 60`, CI's line, it exits 0.

## No tree: `test-changed.mjs`

`node --test scripts/test-changed.test.mjs` reads the workspace's own `cargo metadata --no-deps`
rather than a fixture tree, since the package graph is what it checks: a change to
`crates/vxe-sim/src/body.rs` resolves to `-p flythrough -p vxe-sim -p vxe-tester` for the tests
and for `--clippy`, a root manifest change to `--workspace`, a docs and scripts change to
nothing, and `--binaries gpu_exposure` to the two packages that own a binary of that name.

## `plan-size/`: for `check-plan-size.mjs`

```
node scripts/check-plan-size.mjs scripts/fixtures/plan-size
```

Expect **exit 1 and exactly two findings**, then the count line `2 finding(s) across 2 plan(s)`:

| Case | Seeded as | Expected |
|------|-----------|----------|
| an Implementation log over the cap | `0099-seeded-log-dump.md`: a 140-bullet readings table under the log | `docs/plans/0099-seeded-log-dump.md -> Implementation log is N lines, cap 150: ...` |
| resume notes in the plan body | `0099-seeded-log-dump.md`: a `### Resume notes (...)` heading inside the log | `docs/plans/0099-seeded-log-dump.md:<line> -> resume notes in the plan body: ...` |
| a short log that names resume notes in prose | `0098-seeded-within-cap.md`: "Resume notes" in a bullet, never a heading | not reported |
| a plan numbered before 0055 | none seeded; the repository's own plans up to 0054 are the case | not scanned; their readings stay inline |

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
