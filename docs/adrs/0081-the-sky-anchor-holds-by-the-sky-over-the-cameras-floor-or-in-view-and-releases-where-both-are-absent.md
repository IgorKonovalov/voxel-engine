# ADR-0081: The sky anchor holds by the sky over the camera's floor (the baked record under the eye) or by the sky in view, and releases only where both are absent, by at most a lift over the daylight anchor

> **Status:** proposed (the lift added 2026-11-22, while proposed, at Plan 0097's window 97b; the file keeps its slug)
> **Date:** 2026-11-20
> **Related plan(s):** [Plan 0097](../plans/0097-the-exposures-anchor-by-the-sky-around-the-camera-the-floor-record-under-the-eye-a-time-constant-and-the-canopys-shade.md).
> Amends the exposure's ceiling rule of [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rule 5 (proposed; its clause of this date points here) and the sky-share meter Plan 0094
> Phase 7b built on [ADR-0033](0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md)'s model.

## Context

The auto exposure's ceiling (ADR-0033's `max`) is lowered outdoors by the **sky anchor**
(Plan 0086 Phase 12): `c_a = min(2^N key 2^compensation / (0.5 E_sky(+z) / pi), max)`, the
exposure that puts a sunlit horizontal face at the key opened `N` EV, so a view out of a cave
is not lifted to mid-grey and a lamp-off tunnel at noon does not read "an unnatural blue".
Plan 0094 Phase 7b made the anchor **conditional on the sky in view**: the composite's meter
tiles count the pixels that are sky, the exposure pass reads their weighted share `f_sky`, and
the ceiling's target is `c_a` at a share of `F` (1/256) and over, `max` at 0, log2-linear
between; the ceiling in force steps toward it under the rate cap (3.75 EV/s). Phase 8a clamped
`c_a` to `max` so a dark sky never opens the eye past it (the night at +2 EV). The owner's rule
behind both: **the exposure follows the surroundings, not the time of day** (window 94b).

Window W2 (2026-11-20, the forest at `--sun 10,180`, both arms, so Plan 0094's exposure) read
the measure's failure: "when I reach really shadowed spot - too bright". With 1 % of the view
sky the ceiling is anchored at -4.34 EV and the eye sits on it; one canopy closing over the
view (0 % sky, HUD at x 367.4 y 690.8 z 569.4) releases the ceiling toward `max`, +1.38 EV and
climbing to +2, the eye at -0.86 EV against a metered -1.8, and the forest floor washes to
pale mint. The sky **in view** is a measure of where the camera looks, not of where it is: a
forest floor in daylight shade is the same place whether a leaf hides the last sky pixel or
not, and about 5 EV of exposure hang on that pixel. The owner's pick (2026-11-20): anchor on
how much sky the camera's **position** sees, so a canopy with gaps keeps the daylight anchor
and a tunnel or cave still releases; the two 94c verdicts stand ("noon tunnel all good", the
night held at +2 EV).

`vxe-render` holds three sky-visibility sources already (ADR-0070): the **brick records** of
the baked store, which carry per cell and per face direction the fraction of 256 sky rays
from that face that reached the sky (`record_visibility`, a `u8` each, with `VIS_BAKED` and
`SKY_STALE` flags); the **chunk summaries**, one record per chunk whose visibilities are the
means of the chunk's faces per direction, kept on the CPU (`LightSummaries`) as well as in the
pool; and the **column map**, the highest opaque voxel per 2 x 2-voxel column over the window,
a sheet per chunk on the CPU and an `r32sint` texture. `E_sky` (`sky_irradiance.wgsl`) is the
unoccluded sky's irradiance per axis and carries no visibility; it sets the anchor's level,
not its hold. The store also keeps every block's words on the CPU (`Store::words`), so a
record is readable on the render thread without a readback.

## Decision

The anchor holds by the larger of two normalised shares, **the sky in view** (Phase 7b's
`f_sky / F`, unchanged) and **the sky over the camera's floor**: the `+z` sky visibility of
the first baked record with a `+z` face in the brick column under the eye (walking down from
the eye's cell at most `AROUND_REACH` cells, 8 cells of 8 voxels, a guess read in Plan 0097
Phase 1), that is the fraction of the hemisphere above the floor the camera stands over that
is open sky; where no such record is within reach the camera's own chunk summary's `+z`
visibility stands in, and where neither exists (not baked yet, no sky set) the term is
absent and the view's share alone decides, as today. A `SKY_STALE` record (a dig's ring)
keeps answering with its old value until its re-bake lands (ADR-0076 rule 6: the coarse
stable answer, the precise one as a fade). The measure is smoothed on the CPU with a time
constant `tau_around` (1 s, a guess read at the window) before it enters the exposure uniform,
so a cell or chunk boundary under a moving camera moves the target as a ramp and the ceiling
in force follows under the rate cap, never a step. The ceiling's target is
`log2 c = mix(log2 max, log2 c_a, min(max(f_sky / F, f_floor / F_floor), 1))`, `F_floor` an
`AutoExposure` field whose default Plan 0097 Phase 1 reads (a power-of-two fraction at which
the W2 shade holds and the deep noon tunnel releases); `F_floor` 0 is Phase 7b exactly, the
before for A/Bs. The night's clamp stands untouched: `c_a >= max` is `max` at every share.

The reason the two shares combine by `max` and not by the floor alone: the sky in view says
"the open sky is on screen, do not lift it past daylight plus `N` EV" (the anchor's original
reason, 0086 question 1); the sky over the floor says "you stand in daylight, do not open the
eye past daylight plus `N` EV". Either is a reason to hold. Release needs both absent: a
tunnel past its mouth, a cave, a sealed room. Every scene Phase 7b anchored in full stays
anchored bit for bit, so the open-sky goldens do not move.

**Amended 2026-11-22 (owner, window 97b): the release is bounded by a lift `L`.** The owner
in the noon tunnel at x 262.1 y 4.6 z 489.0, the floor's sky at 5 % and none in view, the
ceiling released 5.5 EV over the daylight anchor to -1.29 EV: "there is blue light and it is
very bright. such thing will never happen in real life. Should be much darker in this case";
Plan 0097 Phase 3a's highlight cap was not binding (nothing clipped), so the complaint is the
release itself. A place by day that the daylight reaches only through a crack is dark against
the daylight it is cut from, and an eye that opens 5 EV in seconds shows it as a lit room. So
the release's top is **`c_top = min(max, c_a 2^L)`** instead of `max`: `log2 c = mix(log2
c_top, log2 c_a, min(s, 1))`, `s` the larger normalised share as above, `L` an `AutoExposure`
field in EV (`sky_anchor_lift`; 4 the default, a guess read at the window; `None` the before,
the top `max`). By day the eye opens at most `L` EV over the daylight anchor wherever the
camera stands; the shares still decide where between `c_a` and `c_top` the target sits; the
ceiling in force steps under the rate cap as before; the night (`c_a >= max`) is `max` at every
share and every `L`; a sky share at `F` and over, or a floor share at `F_floor` and over, is
`c_a` exactly, so nothing anchored in full moves. The reason for 4: the pupil's 2 to 8 mm is
16 times, the fast part of adaptation; the rods' twenty minutes are not a game's daytime eye.
At the readings' noon (`c_a` -6.75 EV, `max` +2) the tunnel's ceilings become -4.35 (floor
5 %), -3.71 (3 %), -3.25 (1.6 %) and -3.13 (1.2 %) against -1.50, -0.10, +0.90 and +1.17,
and a sealed cave by day -2.75 against +2.00, so the deep lamp tunnel by day is about 5 EV
darker than window 94c's +2 (the window-97 before at +2.00 read "even worse", so that verdict
is not the bar; the owner's eye at Plan 0097's Phase 3c is).

## Consequences

### Positive
- A forest floor in daylight shade keeps the daylight ceiling with or without a sky pixel in
  view; the one-canopy 5 EV swing of window W2 is gone by construction where the floor's record
  reads over `F_floor`.
- No new ray, pass, binding or texture: the record exists (ADR-0070), is on the CPU already,
  and a walk of at most 8 cells a frame is microseconds; the uniform's spare word carries the
  share. The exposure pass stays a pass that binds no world.
- The measure is a hemisphere, not a vertical line: a gap 20 m away at 30 degrees counts, as
  the owner's "how much sky the position sees" asks.
- The night is untouched (the clamp), the noon tunnel deep in is untouched (the tunnel floor's
  record reads 0), and the open-sky scenes are byte-identical (the `max`).
- (2026-11-22) Dark places by day stay dark: the eye never shows a tunnel or a sealed room at
  noon as a lit room, whatever its floor reads; the bound follows the daylight (`c_a` moves
  with the sun's height), not the clock, and is one `min` and one multiply in the pass.

### Negative
- The floor's hemisphere is the sky over the floor, not over the eye: under a low overhang the
  floor at the camera's feet may see less than the eye does; in a tall room with a skylight the
  floor may see more. The in-view term and the window judge these; a measure at the eye itself
  needs rays from the camera (Alternative E, the fallback arm).
- Until the floor's record is baked the term is absent and the view's share alone decides, so a
  cold start under a canopy, or a freshly stamped structure, shows Phase 7b's release for the
  seconds Plan 0096's drain takes, then re-anchors as one fade. Disclosed, not hidden.
- `tau_around` and `F_floor` are two more numbers on the exposure model with readings behind
  them, not derivations; a camera that crosses a record boundary every frame still moves the
  target, bounded by `tau_around` and the rate cap, which the walk reading of Plan 0097 bars.
- A flying camera far above the ground (over `AROUND_REACH`) reads its chunk's summary, a
  coarser answer, or none.
- (2026-11-22) A lamp-lit tunnel or cave by day under an outdoor sky is dim: its ceiling is
  `c_a 2^L`, about 5 EV under `max` at noon, so the lamp's near walls that clipped at +2 show
  dark grey, and the same place at night opens to `max`. A consumer whose caves are its game
  sets no sky (no anchor, no lift) or sets `L` higher. `L` is one more number on the exposure
  model with a window's word behind it, not a derivation.

## Alternatives considered

### Alternative A: the sky in view alone (Plan 0094 Phase 7b as built)
Measures the view, not the place: one leaf over the last sky pixel swings the exposure by
about 5 EV (window W2). Kept as one of the two terms, since the open sky on screen is the
anchor's first reason, and `F_floor` 0 restores it exactly.

### Alternative B: the hits' baked visibility on the lighting grid (7b's rejected (c))
The composite would sum `vis_in.a` of the visible surfaces into the meter tile's spare word.
Existing data and a free word, but it is as view-dependent as A (the trunk's shaded side
against the floor), its value at the W2 spots is the no-record prior's where the floor's record
has not landed, and it needs the same `F` reading. It answers "what do the surfaces I look at
see", not "where am I".

### Alternative C: the chunk summaries' mean about the camera (the 3 x 3 x 3 chunks)
Chunk-stable and on the CPU, but it averages faces the camera cannot see through solid rock: a
tunnel one chunk under a hillside takes the surface's `+z` faces (visibility about 1) into its
mean and reads open, the 94b tunnel complaint again, and a chunk boundary swaps 9 of 27 terms
at once. The camera's own chunk summary survives only as the fallback where no floor record is
within reach.

### Alternative D: the column map's footprint (the share of open columns about the camera)
On the CPU already and cheap, but it is a vertical measure: a dense crown with no gap inside a
6.4 m footprint reads 0 where the floor's hemisphere sees 10 % of the sky through gaps 20 m
away, and a cliff overhang reads 0 with open sky to every side. The hemisphere is what the owner
asked for.

### Alternative E: rays from the camera each frame, on a long history
Exact at the eye and independent of the bake (no cold-start gap), about 64 rays a frame is
nothing. Rejected first because it adds a ray source with a history of its own to a pass that
binds no world, where a baked hemisphere measure of the same thing is in hand. **It is the
fallback arm:** Plan 0097 Phase 1 stops for the architect if the W2 shade's floor record cannot
be told from the tunnel's (both at 0), or the window reads the floor's hemisphere as the wrong
place.

### Alternative F: a hysteresis pair of thresholds instead of a time constant
Two bars (hold over `F_hi`, release under `F_lo`) stop the flutter at one boundary, but each
crossing is still a step of the target, and the rules want fades (ADR-0076 rules 3 and 5). The
time constant gives a ramp at every change and the rate cap bounds its speed; a pair of bars
can be added over it if the walk reading shows the ramp is not enough.

### Alternative G: a slope in the floor's share instead of a lift (added 2026-11-22)
The ceiling lifted by `k` EV for every EV the floor's sky is under `F_floor` (`k` 1 follows
the local daylight exactly: every sky-lit floor shows at one brightness; under 1 in part), the
full release to `max` kept for a floor that reads 0. Physically the right shape, and it keeps
the deep lamp tunnel bright, but the owner's eye read the deep pose at +0.93 as overexposed
(window 97, floor 1 to 2 %), so "the deeper, the brighter" is the wrong order on screen; and
the release to `max` sits at the record's last quantum (1/255 to 0), a step of 6 EV or more
where one ray decides. The lift bounds every place by the same daylight and has no such edge.

### Alternative H: a slow release rate (added 2026-11-22)
The ceiling released toward `max` at a fraction of the rate cap (the rods' minutes scaled to
tens of seconds) and re-anchored at the full rate. The look then depends on how long the owner
stood in the tunnel, which no still and no reading can judge, and a camera parked in the
middle ends as bright as today. Rejected for a bound that holds at rest.

### Alternative I: a lower `N`, or a lower `max` by day (added 2026-11-22)
Both darken the W2 shade and the forest at 15:00, which the owner kept ("deep shade better",
"fine"), and a day `max` as a constant does not follow the sun (the tunnel at 17:45 should open
nearly to `max`, as `c_a 2^L` does at +1.92). The owner declined "a lower daytime ceiling" at
window 97 already.

## Outcome

_(Added at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
