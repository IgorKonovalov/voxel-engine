# ADR-0084: A baked key ray that ends under a column's top is not lit; on a world face the key light's default stays the column map's horizon at the hit, and a cover source that may only add light (a ray near, the baked record at every distance) ships as a setting, off

> **Status:** proposed (2026-11-23, with Plan 0100). **Rewritten 2026-11-25, still proposed,**
> to what Plan 0100's window (Phase 8, 2026-11-25) and the owner's answers to its questions
> 8, 9 and 10 decided. As first written it made the key light's visibility one value a face
> (the horizon at the face's centre) with a ray near and the record far as the default under
> cover; the readings and the window refused both, and this text replaces that one. What
> changed is under "Rewrite (2026-11-25)". The file keeps its first name, so the links to it
> hold; the name is not the decision.
> **Date:** 2026-11-23
> **Related plan(s):** [Plan 0100](../plans/0100-a-sun-fixed-in-the-world-the-key-shadow-a-face-at-a-time-the-coarse-tier-held-still-and-sun-under-cover.md)
> (builds it; its readings are in `docs/plans/logs/0100.md`); [Plan 0098](../plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)
> (whose window W3 gave the requirement); [Plan 0096](../plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md)
> (whose Phase 10c read the bake's key ray lit at its cap under a roof)
> **Supersedes, at acceptance, in**
> [ADR-0079](0079-the-key-lights-visibility-is-read-with-no-screen-history-the-record-or-the-column-maps-horizon-at-the-far-a-deterministic-penumbra-near.md):
> in "The bake and the sun's motion" the words "a ray ending at the cap, an absent chunk or
> the window's side counting as lit"; and "The near: the closest-approach penumbra" as far as
> `key far column` goes (no key ray is cast at a cube face for the penumbra there). The rest
> of ADR-0079 stands: the horizon and its march, the record's bake and re-key, the band, the
> fade at a landing, and the near penumbra under the other `key far` modes.
> **Builds on:** [ADR-0082](0082-the-light-pass-keeps-no-screen-history-by-default-and-reads-smooth-sources-to-the-mid-range.md)
> (no screen history by default; its picture stays the default here),
> [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> (rules 2, 3 and 6),
> [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the column clause: under a column's top is not sky),
> [ADR-0055](0055-render-settings-are-one-struct-with-an-apply-class-per-field-and-profiles-are-named-values.md)
> (settings and profile rows)

## Context

Under ADR-0082's defaults (light history off, `key far column`, `key far mix step`) the key
light's visibility on every opaque cube face, at every distance, is the column map's horizon
evaluated at the pixel's hit. The owner accepted that picture as calm at Plan 0098's windows
and named what it does not do (W3, 2026-11-23; his words are in `docs/plans/logs/0098.md`):

- **The noon terraces wobble.** Plan 0098 Phase 9: 2.4 to 3.9 % of the step shadows' pixels
  move over 4 levels a pair at a still camera, 0.00 % with no upscaler and no jitter.
- **No sun under cover.** A heightfield answers 0 under any column's top: no sun patch under
  a canopy, through a roof slot, in a tunnel's mouth. With the screen history on the patches
  are there, and the owner rejected that way of getting them: "light only comes when I see
  it, even after turn looks completely unnatural".
- So the requirement: sun patches steady at a still and a moving camera, present at a
  distance the same as near, present before the surface comes into view and after a turn.

**The baked key record leaked at its cap.** ADR-0079's bake counted a key ray lit when it
ended at its cap in air; Plan 0096 Phase 10c read 64 of 64 floor faces lit under a closed
roof taller than the cap.

**What Plan 0100 read and showed** (each figure is in `docs/plans/logs/0100.md` under the
phase named; RTX is the RTX 3080 Laptop, RENOIR the RADV iGPU at Portable):

- **The horizon is already fixed in the world.** It is a function of the position alone.
  Reading it at the face's centre (`face`) moved 0.10 % of the display; no point it is read
  at (`face`, `face-smooth`, `centre`) closed half the distance to the no-jitter arm at a
  still camera at either pose (Phases 1 and 1b). The ceiling arm, every light term at the
  internal rate and unfiltered, left 51 and 92 % of the moved display pixels. What moves is
  a hit that lands on another face: the sun's term is hard (the cosine by the face's own
  normal, a visibility of 0 or 1) and is sampled at a hit the jitter and the camera move.
- **The coarse tier does not hold a wobble.** Held coarse moved 0.01 % of the near class a
  still pair, landed 0.00 %, both adapters (Phase 3); at the window: "no wobble on slop".
- **The horizon's "lit" is not exact as built.** Against 64 rays a face it read lit where
  the truth is shadow on more faces than its ramp holds (W1's pose 1 054 against 480, the
  slot 146 against 62; Phase 5), for two reasons: the march reads at most 21 of the 256 to
  362 cells on its line, and it asks about a point lifted 2.5 voxels along the normal, so a
  caster within the lift casts nothing. Walking every cell repairs the first; the second is
  the form of a 2-voxel height map (16.4 % of the terraces' key-lit pixels, Phase 5b).
- **The cover arms' cost and error** (Phase 6). `ray`: +0.55 to +0.78 ms of light pass on
  the RTX at Floor in the forests, +1.47 to +1.99 ms on the RENOIR at Portable, under
  `docs/nfr.md` section 2's 4.0 ms; patches only where the far normal's weight is under one
  half. `record`: no measured cost; a cell of 0.8 m; it lands 13 to 19 s after a fresh start
  on the rig and fades in; at the tunnel it lights about half of the truly shadowed pixels
  by over 16 levels.
- **The window** (Phase 8, the owner's words): the wobble is still there under the defaults
  and under every arm, "located only on sunned parts", and worse with no upscaler; `ray`:
  "less wobble, almost none on slopes, some on trees"; `record`: "sun spots looks nice, a bit
  squary but I guess fine for now"; `ray-record`: "not acceptable, big change in sun spots
  when I come near"; at the tunnel's slot "blinking on all 3. shadow missing. I guesss 4a was
  best" (`off`). Asked after it (2026-11-25): sun under cover as a default goes to a
  follow-on plan (question 8, (c)); the slot's blinking was the edges and dots of shadows
  shimmering, the wobble's kind (question 9, (b)); "the best" on the RENOIR was the terraces
  under `key horizon walk`, not `ray-record` (question 10, (b)).

## Decision

**In the bake, a key ray that ends at its cap, at an absent chunk or at the window's side is
lit only if its end point is at or above its column's top. In the frame, the default key
visibility on an opaque cube face stays ADR-0082's: the column map's horizon, marched, read
at the pixel's hit, with no cover source and no key ray cast. A cover source is a render
setting, `lighting.key_cover`, default `off` on every profile; where it is on it may only add
light to what the horizon shows: the shown value is the larger of the two. Nothing built by
Plan 0100 becomes a default; what the window did not refuse stays a setting, and the near
sun is decided in a follow-on plan.**

- **The bake's cap (default behaviour, the one change every build takes).** A floor under a
  closed roof bakes a key visibility of 0, as ADR-0080's zero record says it should (Plan
  0100 Phase 2: 0 on 64 of 64 where it read 255; the open floor still 255 on 64; a ray that
  leaves the window's side above every top stays lit). It changes record bytes wherever a
  key ray ends in air under a top. Under the defaults no pixel reads the key record, so no
  image moves.
- **A cover source only adds.** It is consulted only where the horizon reads under 1, and
  the shown value is the larger of the horizon and the source. A source's error can then
  brighten a true shadow and can never punch a hole in a face the horizon lights; and no
  cover source repairs a face the horizon lights wrongly.
- **`lighting.key_cover`: `off`, `ray`, `record`.** `ray`: where the far normal's weight is
  under one half, one ray from the face's centre toward the key light's disc centre, times
  its transmittance, cast in the light pass with no random number and no mean, so a face
  shows the same value on its first frame, its thousandth and after a turn; past the half,
  the horizon alone. `record`: at every distance, the store's baked key visibility at the
  face's own cell where that holds a fresh word, else the horizon alone; a landing and a
  re-key ride the key slot's fade (`FADE` frames). The record's history is the store; the
  ray keeps none.
- **`lighting.key_at`: `hit` (default), `face`, `face-smooth`;
  `lighting.key_horizon`: `march` (default), `walk`.** `walk` reads every map cell on the
  sun's line through coarse levels of largest tops, so its "lit" is the height map's own.
  It is the horizon's correct form and no profile takes it yet: it read +1.41 ms of temporal
  pass on the RENOIR at the terraces eye (Phase 7) and was not ranked over `march` by the
  eye.
- **Why these stay as settings.** An arm that is not picked and costs nothing when off stays
  a setting (owner, 2026-11-12), and each is a control the follow-on plan reads against:
  `ray` and `record` are the two candidates for sun under cover, `walk` the form any near
  shadow builds on, `face` and `face-smooth` the points a face-fixed source is read at.
  `walk`'s cost when off is the coarse rows under the column map and their upload (the
  bytes are in the plan's log, Phase 5b); it is accepted for that reason.
- **The penumbra under `key far column`.** No key ray is cast at a cube face for the
  penumbra. `lighting.penumbra approach` scores the cover ray where `key_cover ray` casts
  one and changes nothing else; under `key cover off` the penumbra setting is inert. This is
  what the menu says and what the owner picked at Plan 0098's W1 (one source at every
  distance over a true ray near). The other `key far` modes keep ADR-0079's near penumbra.
- **Out of this decision:** entities and field surfaces (they keep their ray), a penumbra of
  the disc's true width, the sky term.

**Where the history lives, and the fades (ADR-0076's binding clause).** The horizon and the
cover ray keep no history: each is a function of the world and the frame's key direction.
The record's history is the store. The switches under `record`, horizon to record at a
landing and record to record at a re-key, ride the key slot's fade. No switch in space is
left: the band where a ray gave way to the record is withdrawn with `ray-record`.

**Deferred to the follow-on plan** (`docs/backlog.md`, "The sun's term at small faces";
drafted by the architect from Plan 0100 Phase 8b's table):

- **The wobble on sunlit faces**, at rest and in the owner's move, the tunnel slot's shimmer
  with it (question 9, (b)).
- **The near shadow's hole:** a caster within the 2.5-voxel lift. A contact ray (a ray a few
  voxels long from a near face's centre that may darken what the horizon lights) is an arm
  there; it would be the one exception "only adds" takes, and is not part of this decision.
- **Sun under cover as a default**, on any profile, and `walk` as a default.

## Consequences

### Positive
- A closed cave no longer bakes sunlit; ADR-0080's zero record agrees with a job, and the
  record can be read (`key far baked`, `key_cover record`) without the leak at the cap.
- The default picture is ADR-0082's to the byte: no golden moves, no frame-budget row
  moves, no profile changes, and the light pass still casts no key ray at a cube face.
- Sun under cover is a flag away in two forms, each with its cost and error read, for
  whoever needs it before the follow-on plan picks.
- The rule a cover source must keep ("only adds") is fixed before a default is chosen, so
  the follow-on plan's arms are compared on one footing.

### Negative
- **The requirement that began this ADR is not met by default.** There is no sun under
  cover, and the sunlit wobble stays, until the follow-on plan.
- **The horizon's "lit" stays inexact by default**: the march's misses (a top between two of
  its distances) and the lift's hole (a rim or a step beside a sunlit face casts nothing;
  every shadow starts `2.5 / tan e` voxels late). Steady in the world, and in the picture
  the owner approved; `walk` closes the first where it is set.
- **Settings that are off are still code and rows**: three values of `key_cover`, three of
  `key_at`, two of `key_horizon`, their menu, banner and inert-table lines, and the coarse
  rows' bytes. A setting the follow-on plan does not pick is removed there, not kept twice.
- **`record` under "only adds" brightens true shadow** where a cell's mean crosses an edge
  (the tunnel: about half of the truly shadowed pixels by over 16 levels), and its squares
  are 0.8 m. `ray`'s patches exist only near, so they arrive as the camera comes up to
  them. Neither is a default for those reasons.
- **The column clause at the cap errs dark** for a ray that ends under a top and would have
  reached the sun past it (a low sun under a tall canopy with a short reach). The horizon
  from the end point is the refinement if a reading shows it.
- **The key bake stays on** under `key far column` though no default pixel reads it
  (`Frame::bake_key` hands the bake the key light whatever `key far`): `KEY_BAKE_RAYS` rays
  a brick direction and the re-key at a turning sun. Turning it off with no reader is a
  question for the follow-on plan, where `record` is picked or dropped.

## Alternatives considered

### Alternative A: One value a face, the horizon at the face's centre (this ADR as first written)
Make the visibility a function of the face, so two hits a jitter step apart read one value
and the shadow's edge falls on a face's edge. The horizon was already fixed in the world;
`face` moved 0.10 % of the display, `face-smooth` and `centre` likewise closed under half the
distance at a still camera, and `centre` moved 70 and 93 % more display pixels than `hit` on
the plan's own bar. Decisive: the fault is a hit that changes face, which no point the
horizon is read at removes. `face` and `face-smooth` stay settings; `centre` is withdrawn.

### Alternative B: The ray near and the record far, switched across a band (`ray-record`)
The first text's default under cover: exact patches near, patches at every distance far.
The two sources disagree where they meet (a face's centre against a 0.8 m cell's mean; a
cell spans 48 internal pixels at the band's half). The owner at the window: "not acceptable,
big change in sun spots when I come near". Withdrawn: the value, its band blend and its
rows leave in Plan 0100 Phase 9.

### Alternative C: `ray` or `record` as the default now
`ray` on the fast profiles (question 8, (a)) or `record` on every profile ((b)). No arm
held at every subject of the window: at the tunnel `off` was the owner's guess for the best,
`ray`'s patches arrive with the camera (the complaint the requirement began with), `record`
lights shadow. The follow-on plan reworks the near sun, so a default picked now is blessed
twice. Decisive: the owner's answer, (c).

### Alternative D: The coarse tier held still (`lighting.coarse_hold`)
A pooled or held value while a face shows its coarse record, for the near trunks' wobble
after a stop. The reading did not hold the fault (0.01 % held coarse against 0.00 % landed;
what moves is the landings, Plan 0099's subject) and the owner saw none at the window. Not
built.

### Alternative E: A second layer in the column map and a slanted walk under the cover
Store the underside of the topmost solid run and let the horizon pass under it. One more
layer answers one overhang; a canopy of sparse leaves, a slot over a ledge, a cave under a
cave are not two heights a column. The map doubles and every lit pixel's march reads twice.
Decisive: it buys the simple overhang and not the forest floor.

### Alternative F: A key value baked per face in the store
Exact at read and free in the frame. The key light moves: every face in the sun's reach
re-bakes each `key_step`, and a face not yet baked has no value, which is the wait the owner
refuses (`docs/backlog.md`, "Per-face light cells"). Decisive: a moving light is the wrong
thing to bake at the face's size.

### Alternative G: The screen history where the horizon reads shadow
`light history on`, confined to shadowed faces. It is the arm Plan 0098's W3 showed: grain
at the patches' edges, the wobble back, light that arrives as one looks. Rejected by the
owner, and by ADR-0076 rule 2.

### Alternative H: The near ray decides wherever it reaches, the horizon only beyond
Drop "only adds" near: one ray from every near face's centre, lit or not. Exact near, the
lift's hole and the march's misses gone there. It is the ray Plan 0098 Phase 13 stopped
casting: the RENOIR's light pass read 6.0 to 11.3 ms with it against 1.6 to 2.1 without, at
Floor, and on a stepped slope it is one binary ray a face, the edges that wobble. Decisive
here: the cost, for a fault that is steady. Its bounded form, the contact ray, is the
follow-on plan's arm.

### Alternative I: A smaller lift
Ask the horizon about a point nearer the face, so a rim within 2.5 voxels shadows it. A
face whose air-side point then shares a 2-voxel map cell with a higher voxel reads 0 by the
own-cell rule: stripes a voxel wide on walls and stepped slopes, which is why Plan 0086 set
the lift past the cell; both lift rows read new dark at the terraces (Plan 0100 Phase 5b).
Decisive: it trades a missing shadow for a striped one.

### Alternative J: The approach ray kept near under `key far column`, the menu corrected
Leave the per-pixel approach ray at near faces under `penumbra approach` and call the
setting live near. It is a key ray at the hit, on every near face (5.2 ms of light pass at
the canopy eye on the RTX), against the one-source picture the owner picked. Rejected.

## Rewrite (2026-11-25)

Against the first text and its amendment of the same day. **Stands:** the bake's column
clause at the cap (landed, Plan 0100 Phase 2); "a cover source only adds"; the walk as a
form of the horizon; the lift's limit named. **Falls:** the face's centre as the horizon's
point (Alternative A); the ray near with the record far (Alternative B); any cover source
as a default (Alternative C); "if the window picks the walked horizon, this ADR supersedes
the march's form in ADR-0079": it did not, and `COLUMN_KEY_STEPS`' march stays the default.
**Added:** the penumbra's ruling under `key far column`, and with it the narrower
supersession of ADR-0079's near clause; the list of what is deferred.

## Outcome

_(Added at acceptance, at Plan 0100's close: what Phase 9 shipped against this text, and
Phase 8b's line on where the sunlit class's reversals come from.)_
