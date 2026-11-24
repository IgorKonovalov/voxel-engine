# ADR-0084: The key light's visibility on a world face is fixed per face in the world: the column map's horizon where it reads lit, a cover source where it reads shadow (a ray from the face's centre near, the baked record far); a baked key ray that ends under a column's top is not lit

> **Status:** proposed (2026-11-23, with Plan 0100; the mechanism under cover is the owner's
> pick at that plan's window, so the Decision names the rule and the recommended arm, and the
> Outcome at acceptance names what was picked)
> **Date:** 2026-11-23
> **Related plan(s):** [Plan 0100](../plans/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md)
> (builds it); [Plan 0098](../plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)
> (whose window W3 gave the requirement); [Plan 0096](../plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md)
> (whose Phase 10c read the bake's key ray lit at its cap under a roof)
> **Supersedes, at acceptance, two clauses of**
> [ADR-0079](0079-the-key-lights-visibility-is-read-with-no-screen-history-the-record-or-the-column-maps-horizon-at-the-far-a-deterministic-penumbra-near.md):
> "The near: the closest-approach penumbra" (the near key source), and in "The bake and the
> sun's motion" the words "a ray ending at the cap, an absent chunk or the window's side
> counting as lit". The rest of ADR-0079 stands (the horizon, the record's bake and re-key, the
> band, the fade at a landing).
> **Builds on:** [ADR-0082](0082-the-light-pass-keeps-no-screen-history-by-default-and-reads-smooth-sources-to-the-mid-range.md)
> (no screen history by default; this ADR lifts its limit under cover),
> [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> (rules 2, 3 and 6),
> [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the column clause: under a column's top is not sky),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the per-pixel
> sun ray this keeps, moved to the face's centre)

## Context

Under ADR-0082's defaults (light history off, `key far column`, `key far mix step`) the key
light's visibility on every opaque cube face, at every distance, is the column map's horizon
evaluated at the pixel's hit (Plan 0098, "Amendment at Phase 4's findings", read in
`temporal.wgsl` and `pair.wgsl`). The owner accepted that picture as calm at windows W2 and W3
and named three things it does not do (W3, 2026-11-23; his words are in
`docs/plans/logs/0098.md`):

- **The noon terraces wobble.** The horizon's edge is hard, one map cell wide, and it is
  evaluated at a hit that moves with the jitter. Plan 0098 Phase 9 read it: 2.4 to 3.9 % of
  the step shadows' pixels move over 4 levels a pair at a still camera, 0.00 % with no
  upscaler and no jitter; 22 to 32 % of the moved pixels show the key value itself changing
  at the jittered hit.
- **No sun under cover.** A heightfield answers 0 under any column's top at any sun angle: no
  sun patch under a canopy, through a roof slot, in a tunnel's mouth. With the screen history
  on the patches are there and the light is "comparable to the trees", and the owner rejected
  that way of getting them: "light only comes when I see it, even after turn looks completely
  unnatural", "wobble comes back, and light only visible when I close".
- So the requirement: sun patches that are steady at a still and a moving camera, present at
  a distance the same as near, and present before the surface comes into view and after a
  turn. A screen history gives none of the three.

Two facts bound the choice. **The column map's "lit" is exact and its "shadow" is not:** a
point whose ray clears every column top on the way to the sun is lit, whatever is below those
tops; a point the horizon shadows may be lit by a ray that passes under the top (an overhang,
a canopy's underside, a slot). So the horizon errs only dark, and only where it reads shadow.
**The baked key record leaks:** ADR-0079's bake counts a key ray lit when it ends at its cap
in air, and Plan 0096 Phase 10c read 64 of 64 floor faces lit under a closed roof taller than
the cap; the record's blend between brick-direction cells put 12 225 px bright where the truth
is shadow in the tunnel-leak diagnosis (`docs/backlog.md`, "The baked key record's cell blend
carries light across a shadow edge"), and at the leak window the owner saw the seam: "its lit
now but when I come closer it goes shadow". A record is one value per brick direction, a cell
of 0.8 m; a step's shadow is 0.1 m.

Cost is the third force. Plan 0098 Phase 13 stopped casting the key ray that was never shown:
the light pass fell from 0.68 to 1.64 ms to 0.22 to 0.42 ms on the RTX 3080 Laptop and from
6.0 to 11.3 ms to 1.6 to 2.1 ms on the RADV RENOIR (Floor, 1080p output; `docs/plans/logs/0098.md`).
A ray on every near face is that cost back; `docs/nfr.md` section 2's lighting line is 4.0 ms.

## Decision

**On an opaque cube face the key light's visibility is a function of the face, not of the
pixel, and of nothing kept on screen. It is the column map's horizon evaluated at the face's
centre; where that reads fully lit it is the answer. Where it reads under 1 a cover source may
only add light: near (the far normal's weight under one half) one ray from the face's centre
toward the key light's disc centre, times its transmittance, cast every frame with no mean;
far, the store's baked key visibility where its cell holds a fresh one. The shown value is the
larger of the horizon and the cover source. In the bake, a key ray that ends at its cap, at an
absent chunk or at the window's side is lit only if its end point is at or above its column's
top.**

- **One value a face.** The face is the one the hit record names, at the level it was hit;
  its centre is the voxel's centre moved half a voxel along the face normal. Two hits on one
  face a jitter step apart read the same value, so the shadow's edge falls on a face's edge,
  which the upscaler resolves as it resolves an albedo edge. Whether the eye takes a shadow
  edge stepped by faces, or wants the four nearest face-centre values blended across the
  face's plane, is read and picked in Plan 0100 (its Phase 1 arms `face` and `face-smooth`).
- **The cover source near is a ray, because only a ray knows a slot.** It is cast only where
  the horizon reads under 1, from a point fixed in the world, toward a direction fixed for the
  frame: no random number, no history, the same value on the first frame a face is seen as on
  the thousandth, and after a turn. With `lighting.penumbra approach` the ray scores its
  closest approach (ADR-0079's probes, now from a world-fixed origin); with `disc` it is the
  centre ray alone, binary but for the leaves' transmittance.
- **The cover source far is the record, because a ray per far pixel is the far trees'
  wobble** (a face under a pixel has no stable centre) **and the far has no budget for it.**
  A brick direction's value over 0.8 m is under the band's pixel size there. Until a far
  face's record lands it shows the horizon alone, which is rule 6's coarse, stable answer;
  the landing is the one fade ADR-0079 gave the key slot.
- **The cover source only adds.** The horizon's lit is exact, so nothing may darken it; a
  record's leak can then brighten a true shadow but never punch a hole in a lit face.
- **The bake's cap.** With the column clause at the cap a floor under a closed roof bakes 0,
  as ADR-0080's zero record already says it should. It changes record bytes wherever a key
  ray ends in air under a top.
- **Settings, not a fork.** `lighting.key_at` (`hit`, `face`, `face-smooth`) and
  `lighting.key_cover` (`off`, `ray`, `record`, `ray-record`) are render settings with a row
  per profile (ADR-0055); `hit` and `off` are ADR-0082's picture to the byte and stay
  reachable. A slow adapter's profile may take `record` or `off` where the ray does not fit
  its lighting line.
- **Out of this decision:** entities and field surfaces (they keep the ray as today), a
  penumbra of the disc's true width, and the sky term.

**Where the history lives, and the fades (ADR-0076's binding clause).** The horizon and the
ray keep no history: each is a function of the world and the frame's key direction. The
record's history is the store. The switches: horizon to record at a far landing, and record
to record at a re-key, ride the key slot's fade (`FADE` frames); the ray to the record across
the band is a seam in space, blended by the far normal's weight if the plan's reading shows
the two sources apart there.

## Consequences

### Positive
- Sun under canopies, through slots and into openings, from the first frame a face is
  visible and after any turn, near; at a distance once the chunk's record lands, and kept
  while the chunk is resident, seen or not.
- The shadow's edge no longer moves inside a face under the jitter, at any sun angle; the
  noon terraces' dashes become whole faces.
- A closed cave no longer bakes sunlit, so the record can be read again without the leak
  ADR-0079's Outcome and the leak window carry, and ADR-0080's zero record agrees with a job.
- Lit faces pay nothing new: the ray is cast only where the horizon reads shadow.

### Negative
- **The ray is back in the light pass** on every near face the horizon shadows: in a forest
  that is most of the floor. Plan 0100 reads it on both adapters before the window; the
  bound known today is Phase 13's "before" column above. The Portable row may not afford it.
- **A shadow edge stepped by faces** is a 10 cm staircase along any edge not parallel to the
  grid, and with `disc` it has no penumbra. `face-smooth` and `approach` soften it at a cost
  (four horizon marches a lit pixel; the approach's probes read 17.4 ms on the RENOIR's
  forest at Portable under Plan 0096).
- **Near and far can disagree until the far lands, and by resolution after.** A patch that is
  there near and absent far for the seconds the bake takes is the seam the owner named at
  W1 ("light rays are gone when I am distant"), now bounded by the queue's drain (Plan 0099's
  line) instead of permanent. A patch smaller than a brick reads as a faint 0.8 m blot far.
- **The record leaks where it is read**: the cell blend carries light across an edge, and
  with "only adds" a leak brightens a true far shadow. Plan 0100 reads the leak before
  choosing the read (the face's own cell, or the blend).
- **The key bake stays on by default** wherever `key_cover` reads the record: `KEY_BAKE_RAYS`
  rays a brick direction, the re-key at a turning sun (ADR-0079's Outcome: 2.3 to 49.5 s
  after a step).
- **The column clause at the cap errs dark** for a ray that ends under a top and would have
  reached the sun past it: a low sun under a tall canopy with a short reach. The horizon
  from the end point is the refinement if a reading shows it.
- Every sunlit golden moves at the pick, and tests of the record's bytes move with the cap.

## Alternatives considered

### Alternative A: The baked key record as the source under cover at every distance
No ray in the frame, the store's words as they are (with the cap fixed), sun under cover
before the face is seen. A value per brick direction cannot hold a shadow narrower than
0.8 m: read near with "only adds", every hard shadow's edge washes out to the brick's mean,
and the cell blend's leak is in front of the eye (the leak window: lit far, shadow near, was
this source against a true one). Decisive: near, the eye sees the cell. Kept as the `record`
value of `key_cover`: the arm a slow adapter may take, and a control in the window.

### Alternative B: A second layer in the column map and a slanted walk under the cover
Store the underside of the topmost solid run beside the top and let the horizon march pass
under it. One more layer answers one overhang; a canopy of sparse leaves, a slot in a roof
over a ledge, a cave under a cave are not two heights a column. The map doubles, every lit
pixel's march reads twice, and the answer is still an approximation with its own seams.
Decisive: it buys the simple overhang and not the forest floor the owner asked for.

### Alternative C: A key value baked per face in the store
The same idea done in the bake: a face cell per near face (Plan 0059's face path, deleted at
its Phase 14), exact at read and free in the frame. The key light moves: every face in the
sun's reach re-bakes each `key_step`, where a ray follows the sun every frame for nothing
kept; `docs/backlog.md` ("Per-face light cells") read about 1.9 x 10^5 near faces adding
1.3 s to a fresh start for the emissive light alone, and a face not yet baked has no value,
which is the wait the owner refuses. Decisive: a moving light is the wrong thing to bake at
the face's size.

### Alternative D: The screen history where the horizon reads shadow
`light history on`, confined to shadowed faces. It is the arm W3 showed: grain at the
patches' edges, the wobble back, light that arrives as one looks. Rejected by the owner, and
by ADR-0076 rule 2.

## Outcome

_(Added at acceptance: the picks of Plan 0100's window, each profile's row, and what the
readings falsified above.)_
