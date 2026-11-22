# ADR-0082: The light pass keeps no screen history by default and reads smooth sources to the mid range

> **Status:** proposed
> **Date:** 2026-11-21
> **Related plan(s):** [Plan 0098](../plans/0098-a-calm-light-by-default-no-screen-history-smooth-sources-to-the-mid-range-and-the-fsr-2-turn.md)

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
2026-11-22, below). The history stays a render setting (`light history on`), off by default. The
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
  grain" outdoors with the approach penumbra; indoors under lamps is unseen.
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

_(Added at acceptance: the band's values, the penumbra's form and the FSR 2 mechanism picked
at Plan 0098's window.)_
