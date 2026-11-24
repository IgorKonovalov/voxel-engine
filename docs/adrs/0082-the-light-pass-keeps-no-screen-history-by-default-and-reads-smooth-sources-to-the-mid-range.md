# ADR-0082: The light pass keeps no screen history by default and reads smooth sources to the mid range

> **Status:** accepted (2026-11-24, at Plan 0098's close, v0.72.0; with an Outcome)
> **Date:** 2026-11-21
> **Related plan(s):** [Plan 0098](../plans/done/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)

## Context

Plans 0094 and 0095 each calmed one estimator of the outdoor light (the visibility pair's
means, then the key light at the far) and each passed its readings; at both windows the owner
saw no real change ("image before and after are roughly the same"). Plan 0095's own soak table
agrees: its far path answers about 3 % of the far pixels, because the estimate applies only at
the far normal's weight 1 and the band `[2, 6]` pixels a voxel still casts the ray.

The diagnosis window of 2026-11-21 (the owner by eye, RTX 3080 Laptop, Linux, the debug page's
switches; his words are in `docs/plans/logs/0098.md`) split the complaint into two sources:

- **The slopes' wobble and the grain are the light passes' screen history.** With the far
  smooth (`key far column`, `key far mix step`, `far normal 8,12`) and `light history off`:
  "now things got better, nothing on slopes", "no grain"; with the history back on: "yes, its
  light history - with it wobble worsens". The exposure is not a source (auto: "stays calm").
- **The far trees' wobble after a turn is FSR 2.** Native: "no wobble on turn, slight when move
  on the distance"; our own upscaler "wobbles too"; and "without [FSR 2] picture is not right",
  so the upscaler stays.
- **The bake queue's landings are not the visible wobble.** `queue freeze` on: "freeze makes
  things worse". The architect's reading-based guess (records landing after a turn) was wrong
  by the eye.
- **The approach penumbra follows the camera.** At the default far with the history off: "when
  I move shadow moves with me which is not right"; with `penumbra disc`: "shadow does not
  move".

This is a decision because the history is what every light plan since Plan 0057 has tuned, and
dropping it gives up the cheap way to average a noisy per-frame sample.

## Decision

The light passes take no screen-space history by default: every lighting pixel is computed
from this frame's hit, the world's records and the sun, and what a term cannot answer without
averaging over frames it answers from a smooth source instead (the store's record, the column
map's horizon, the 27-count gradient normal). The band in which a voxel counts as far for the
shading normal and for the key light's far estimate widens from `[2, 6]` pixels a voxel toward
the mid range; its values, and the near penumbra's form (approach made world-fixed, the disc,
or the disc's centre ray alone), are the owner's picks at Plan 0098's window, recorded in this
ADR's Outcome. **A change in a world record (a bake landing, a refinement) fades in the store,
every word the image reads, since no screen history is left to soften it** (amended
2026-11-22, below). **A landing that changes no word but switches a pixel's source (its own
ray or the column's answer before the block, the block's record after) fades in the light
pass by the frame the block records for its landing: a world record's clock, no screen
history** (amended 2026-11-23, still proposed: Plan 0098's Phase 6 read the store's fade
moving no pop, because no word changed). The history stays a render setting (`light history on`), off by default. The
upscaler keeps its own history; FSR 2 stays the default upscaler, and its behaviour after a
turn is treated inside its path, by a reading first.

## Consequences

### Positive
- The image right after a move is the image at rest, by construction, for everything the
  screen history used to carry: there is no mean to restart. **Amended 2026-11-22** (still
  proposed; Plan 0098's readings): the first draft said ADR-0076's rule 6 then holds "without
  a fade", which is wrong. With no history a bake landing shows in one frame (15.61 % of the
  near pixels at one landing after a stop, the exposure pinned), so the fade rule 6 asks for
  moves into the store.
- One fewer source of state: the history test, its rejection rules and the stair rules stop
  deciding what the default image looks like.
- The light pass can get cheaper where the history's passes can be skipped (unread; Plan 0098
  reads it on both adapters).

### Negative
- **Far geometry finer than a pixel's jitter step loses the average that hid it.** Sparse
  leaves at 15 to 60 m change hit between jitter phases (17.46 % of the far hits at rest), and
  the two hits' light differs; the history averaged that, and the upscaler's own history does
  not. No mechanism is known yet (added 2026-11-22; Plan 0098 Phase 5 is its spike).
- **A term that draws one random sample a frame has nothing to average it.** The disc
  penumbra, a sky ray where no record is fresh and the fired lights' visibility each show
  per-frame noise unless they become deterministic or read a smooth source. The owner saw "no
  grain" outdoors with the approach penumbra; indoors under lamps is unseen. **Amended
  2026-11-23** (still proposed; Plan 0098's Phase 4 findings, read in the shaders): under the
  picks the disc's sample is never shown on a world face. A pixel with no history shows the
  mean's seed, the far estimate, so the sun's shadow is the column map's horizon at every
  distance: no penumbra but the horizon's ramp, and no sun under any cover at any sun angle.
  The per-frame sample is shown only at an entity or a field surface.
- **Shadow detail at the mid range goes.** The column map's horizon is a heightfield shadow: a
  canopy reads as a soft block, and the owner saw "bright places around the trees when moving"
  in one run (cause unread).
- The widened band moves ADR-0042's table and ADR-0079's far gate; goldens re-bless.
- The work of Plans 0094 and 0095 on the history (the seeded mean, the age rules, the fades of
  a restarted pixel) runs only when the setting is on.

## Alternatives considered

### Alternative A: Keep the history and improve its rejection and restart
Plan 0094's and Plan 0095's line. It lost because two plans of it passed their bars and
changed nothing the owner could see, and because the window shows the history itself is the
slopes' wobble.

### Alternative B: Stop refining what is in view (freeze the landings)
The architect's proposal before the window, from the readings (the far's post-turn drift 61 %
with the queue against 12.5 % frozen). It lost at the window: "freeze makes things worse".

### Alternative C: Drop the upscaler, or switch to our own
Native removes the trees' wobble after a turn. It lost because the owner finds the image
"not right" without FSR 2 and our own upscaler "wobbles too".

## Outcome

**Accepted 2026-11-24 at Plan 0098's close (v0.72.0).** The decision stands as the default on
every profile; three of its sentences are narrowed by what was built and seen. The owner's
words are in `docs/plans/logs/0098.md`; the windows are W1, W2, W3, 98d and 98e.

**The picks (owner, W2; confirmed on both adapters at W3).** Light history off in every scene
kind, lamps included ("no flicker, all good"); `key far column`; `key far mix step`; the far
normal's band `4..8` pixels a voxel; the penumbra setting at `disc`. The arms not picked stay
settings, off: light history on, `light history far`, the approach penumbra, `key jitter off`,
`key far baked`, `landing fade`, `source fade`, `far light wide`.

**What the picks show, named as it is (read in the shaders at Plan 0098's Phase 4).** With no
history a pixel shows the key mean's seed, which is the far estimate, so on a world face the
sun's shadow is the column map's horizon at every distance: no penumbra but the horizon's
ramp, and no sun under any cover at any sun angle. The disc's ray was never shown on a world
face and is no longer cast there (Phase 13: the RADV RENOIR's light pass at 1080p falls from
11.3, 6.2 and 6.0 ms to 1.6, 1.6 and 2.1 ms in the forest at 15:00, the forest at a low sun
and the hills at noon, under the 4.0 ms line; it read 33.9, 27.7 and 11.2 ms under the
defaults before this ADR). The owner picked one source at every distance over a true ray near
(the approach penumbra seams against the column where the two disagree); the word "disc" names
that pick by accident. `key jitter` and `penumbra disc` are inert on world faces under the
defaults and the docs say so.

**Light history on is rejected as an alternative** (owner, W3): "wobble comes back, and light
only visible when I close"; "light only comes when I see it, even after turn looks completely
unnatural".

**FSR 2: no lever is picked.** Scale 1.5 with FSR 2's own phases stands (W1). The far trees'
wobble is sparse geometry under the jitter made visible by the light (17.46 % of the far hits
change between two jitter phases at a still camera); no arm calms it on any model, `far light
wide` is "not calmer", and under the picks the owner calls it "not too bad". It stays open in
the backlog ("Distant trees that do not wobble") and with Plan 0033's re-read.

**The landing's step is accepted by eye, not faded by default.** The Decision's two bold
sentences were built as written (`landing fade`: the store fades every word the image reads;
`source fade`: the switch of source fades by the block's landing frame, a new head word) and
both are settings, off: at W2 the owner saw "no difference" at the landings between the two
fades on and off, and called the image without them "all looks correct". The reading that
goes with that acceptance: a first block landing over the fallback chain moves 37.3 % of the
near pixels over 4 display levels in its frame under the picks, a mean of 17.7 levels (RTX
3080 Laptop, the forest, the exposure pinned); `source fade` cuts the mean 3.4 to 5.5 times
and leaves the share over 4 levels where it was, a linear-light fade from black. No bar reads
a landing; the image at rest with the queue stopped is asserted instead.

**The dig's rule, as Plan 0098's Phases 16 and 17 built it (accepted at window 98e on both
adapters: "so much better, not ideal but acceptable", a slight colour change at a landing).**
ADR-0076's two statements for it: the history is the store's record, and one fade carries the
switch.

- The bricks a dig's box dilated by one voxel touches (ring 1) are a job of their own that
  opens in the edit's step ahead of every other job; a stale job in a ring chunk is withdrawn,
  not awaited. Ring 1 lands one or two frames after the dig on the RTX 3080 Laptop and on the
  RADV RENOIR at Portable, on a waiting queue too.
- From the edit to that landing each touched brick holds a record made on the CPU against the
  world (the sky visibility of the surface the dig cut through, times a near openness, at
  least a floor above black), which the light pass reads as any record: no ray, no screen
  mean. The landing is one linear fade in the store over 4 frames (`DIG_FADE`); the chunk's
  second job waits for that fade to end.
- **Two switches carry no fade, and are disclosed:** a stale ring record steps to its
  re-bake's in one frame (read under 4 display levels in Plan 0098's Phase 8), and a new dig
  into a chunk whose dig fade still runs ends that fade with a step of at most the fade's
  remainder. The slight colour change the owner names at a landing is the held estimate's
  distance from the baked record.
- **The delay of a dig until its light is ready** (the owner's idea at window 98d) needs
  staged mirror records and is parked in the backlog. Rejected with their reasons in Plan
  0098's "Amendment at window 98d": holding the edit's upload back (the bake has nothing to
  trace until the upload), a second whole mirror, freezing the screen's pixels in the dig's
  box (a screen history), blocking the frame on the dig's readback, a full bake of the dug
  faces on the CPU, and the coarse count for a dig.

**The limits the picks leave.** With no store bound, or before a bake lands, a face under
cover has no sun and no sky light (`outdoor_forest_afternoon.png` is that picture). Under the
defaults no sun reaches under a canopy or through an opening; the noon terraces' one-voxel
step shadows move under the jitter; near trunks and leaves wobble while a face holds its
coarse record. Those three are Plan 0100's, under ADR-0084. At an entity or a field surface
the raw disc sample of one ray a frame is shown (inferred from the code, unread).
