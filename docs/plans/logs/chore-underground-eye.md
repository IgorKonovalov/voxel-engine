# chore-underground-eye: readings

### The surface reference of `the_underground_eye_reads_no_sky` (2026-11-25)

The weekly soak's `gpu_outdoor::the_underground_eye_reads_no_sky` failed at its surface
reference: `surface stored 157.49023807374985` against 149.7 within 5 %. The reference was the
Plan 0086 diagnosis's reading at `2fe4598c` (the Plan 0086 log, "Whole world mirrored, baked":
r 7, 149.7 / 1345), written into the bar at Phase 9 (`d821e3e2`, 149.68 / 1345.5).

**Rig.** `reading::the_underground_eye` (`apps/flythrough/tests/gpu_outdoor.rs`, `#[ignore]`d,
by name, `SVET_UNDER_R` or before the rename `VXE_UNDER_R`), which prints every part the soak
test asserts on and asserts nothing, so one run reads the reference and the underground parts.
Lavapipe (`force_fallback_adapter`), exclusive perf lock (`svet-perf.lock`), one run a hold,
`CARGO_BUILD_JOBS=4`, `MemoryMax=6G`. Old commits in one scratch worktree under `live/`,
removed after. Three lanes open, so every reading is loaded (indicative for time; the values
are deterministic). The surface view's hit pixels are 13 851 in every r 7 row.

**Probes** (surface reference, `sky_rays` 1; `sky_rays` 0 reads the same stored value in
every row):

| commit | what | r | surface stored | surface direct | run | load start / end |
|---|---|---|---|---|---|---|
| `db9c170a` | main, v0.74.0 | 7 | 157.49 | 1396.5 | 248.8 s | 4.8 / 16.6 |
| `fac2c751` | Plan 0086's merge | 3 | 159.35 | 1456.4 | 71.7 s | 15.7 / 6.5 |
| `fac2c751` | Plan 0086's merge | 7 | 157.49 | 1345.4 | 170.9 s | 6.1 / 11.2 |
| `f39e7dd3` | Plan 0086 Phase 16, parent of Phase 17 | 7 | 149.68 | 1345.5 | 167.4 s | 7.7 / 10.8 |
| `ceb9dd0e` | Plan 0086 Phase 17 defaults | 7 | 157.49 | 1345.5 | 178.9 s | 9.2 / 12.1 |
| `db9c170a` | main, `sky_normal: SkyNormal::Face` set by hand in the reading (scratch, reverted) | 7 | 149.68 | 1396.5 | 213.5 s | 11.2 / 18.8 |

The r 3 rig was tried as the cheap probe and dropped: its surface view sees the extent's edge
(12 282 hits) and its value is not the r 7 one (the diagnosis read 152.8 there), so the
remaining probes ran at r 7. No bisection over the merges was needed after the Plan 0086
merge read the failing value: the stored part is 157.49 from Phase 17 to main, unchanged.

**What moved it.** `ceb9dd0e` (Plan 0086 Phase 17, "the window's picks as defaults") made
`Lighting::sky_normal` default to `SkyNormal::Smoothed`
(`crates/svet-render/src/frame/settings.rs`, the `Default for Lighting` row `sky_normal:
SkyNormal::Smoothed`, line 1010 on main). Under it the baked sky term of a cube face
(`crates/svet-render/src/shaders/temporal.wgsl`, `sky_baked_term`, lines 424 to 447) reads
the mix over the directions the 27-count gradient's normal leans toward of each direction's
baked visibility times the sky irradiance about it, instead of the face's own visibility about
its own normal: on the hills' stepped surface the risers and the treads read the slope's
blend, which puts more of the open sky on the risers. The mechanism working as the plan
intended (Phase 13 built it, Phase 17 made it the default on the owner's window picks); the
Phase 17 done-when re-blessed the outdoor goldens but did not re-read this soak test's
reference. The face normal on main gives back 149.68 to the digit, so nothing else since
Plan 0086 moved the stored part.

**The direct part moved too, after Plan 0086, and still passes.** 1345.4 at Plan 0086's merge,
1396.5 on main (+3.8 %, inside the bar's 5 %), with the same 13 851 hits and under either sky
normal. Not localized by this chore (the bar holds); a later drift past 5 % would trip the test
on it.

**The underground parts on main** (the claim the test exists for; the reading at r 7 on
`db9c170a`, read off the same run as the first row): near-eye baked visibility mean 0.0000,
max 0.000 in every face (177 to 326 faces each, 0 without a value); the stored and direct
parts 0.0000 in all six views (+x, -x, +y, -y, up, down) at `sky_rays` 1 and 0; the sky ray's
hit pixels 0 in every underground view. All pass their bars (under 0.001, under 0.02).

**The re-derivation** (lane rule 2a): the claim and the 5 % bar stay; the stored reference
moves from 149.7 to 157.49, the value the quantity reads deterministically under the default
from Phase 17 to main (five r 7 readings above at three commits, on CI's lavapipe in the
weekly run of 2026-11-23 too). The direct reference stays 1345. The test read in both forms
(the tip changes only the test, so the base's and the tip's engine are the same code):

| tree | form | result | time | load start / end |
|---|---|---|---|---|
| base `db9c170a` | old (149.7) | failed, `surface stored 157.49023807374985` | 180.7 s | 17.3 / 11.1 |
| base `db9c170a` | new (157.49) | the tip's run below (same engine, same binary inputs) | | |
| tip | old (149.7) | the base's run above (same engine) | | |
| tip (uncommitted test change on `db9c170a`) | new (157.49) | passed | 169.0 s | 9.2 / 8.8 |

The test's time on lavapipe after the clock cap was lifted: 169 to 181 s under load 9 to 17
(242.8 s at Plan 0086 Phase 9).
