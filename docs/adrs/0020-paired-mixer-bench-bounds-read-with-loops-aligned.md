# ADR-0020: A paired mixer-bench bound is read with both builds' loops aligned to 64 bytes

> **Status:** accepted
> **Date:** 2026-10-21
> **Related plan(s):** [Plan 0020](../plans/done/0020-cavern-reverb-directional-echoes-and-gliding-taps.md) Phase 13 (the bound that raised it)

## Context

A plan phase that touches `vxe-audio`'s mixer bounds its cost against its parent: "`bench
mixer` medians of three alternating runs against the parent's, within N percent in the same
session". The bound defends a claim about *work added*, such as "the room curve is evaluated
per command, not per block".

Plan 0020 Phase 13 failed that bound with no new work in the block. On the dev box (Ryzen 9
5900HS, release, exclusive perf lock, load 9.28 to 4.60), the code of c85bac5 read static
0.6897 ms against the parent 83cbf6f's 0.6519 ms (+5.80 percent, bound 5), turning +3.73
percent, the two direct-delay cases +0.50 and +0.24 percent. `render` is the same size in both
builds, and the parent with only Phase 13's `reverb.rs` keeps the parent's figure. The hot FIR
loop `convolve` starts at a different offset within its cache line in the two builds (`...d00`
against `...530`). Rebuilt with `-C llvm-args=-align-loops=64`, both trees read the same:
static 650.8 and 657.9 µs against 651.1 and 655.1 µs. The gap is where the linker put the loop,
which any edit anywhere in the crate can move. It is not work the phase added.

A relative bound that a code-placement shift of about 6 percent can fail, or pass, does not
defend its claim. It will fail again on unrelated edits, and it can hide a real regression of
the same size when the placement happens to move the other way.

## Decision

A **relative (paired) bound** on `cargo bench -p vxe-audio --bench mixer` is read with **both
builds compiled with `RUSTFLAGS="-C llvm-args=-align-loops=64"`**, in a separate target
directory (`CARGO_TARGET_DIR=target/aligned`) so the default build stays warm. Each build gets
three alternating runs inside one exclusive perf-lock hold. The bound is met when the median of
each case's three criterion medians is within the stated percentage of the parent's. The
default-flag pair may be logged beside it as information, but it does not decide the bound.

An **absolute bound** (the `docs/nfr.md` section 9 mixer figures: 0.6 ms static, 1.0 ms turning,
1.0 ms on the baseline CPU) is read on the **default build**, the code that ships, under section
9's quiet-box protocol. Placement is part of what ships, so it stays in that figure.

This applies to the mixer bench only. Another bench adopts it through its plan's amendment,
after the same evidence: a paired gap that vanishes with loops aligned.

## Consequences

### Positive
- A paired bound measures work added. A layout shift from an unrelated edit no longer fails it.
- The mechanism is one flag, available on the pinned stable toolchain, with no new dependency
  and no code change.
- Absolute figures still carry the shipped build's placement, so a real-world slowdown from
  layout still shows where it matters: against the budget.

### Negative
- Aligned builds are not the shipped binary. A change that is slower *only* through placement
  passes its paired bound and shows only in the absolute figure.
- A second release build per tree (`target/aligned`): roughly another build's time and disk.
- `-align-loops` is an LLVM option, not a stable `rustc` flag. A toolchain bump that drops or
  renames it breaks the protocol, and the bump's plan must replace it.
- Aligning every loop to 64 bytes pads the code, so the aligned absolute figure differs from the
  default one. Only the ratio is read from it.

## Alternatives considered

### Alternative A: Accept the reading as measured and note it at close

Record the +5.80 percent, explain it as placement, and let the close review accept it. This
loses because it makes the bound advisory. Every later phase with the same kind of gap would
need the same exception, and a real 6 percent regression would read exactly like this one.

### Alternative B: Nudge the code's placement until the default build passes

Have `dev` add `#[inline]` attributes, reorder functions or pad the code until `convolve` lands
where the parent's did. This loses because it tunes the code to the benchmark. The fix is
fragile, since the next edit moves it again. It is invisible in review, and it spends dev time
on something no listener hears.

### Alternative C: Bound instructions retired instead of time

Read `perf stat -e instructions` over the bench. The count does not depend on placement or
load. This loses because it ignores the cache and branch effects that a mixer change can really
cause. It also needs `perf` access on the box and on CI runners, and the budget it would gate
is stated in milliseconds.

## Outcome

_(none yet)_
