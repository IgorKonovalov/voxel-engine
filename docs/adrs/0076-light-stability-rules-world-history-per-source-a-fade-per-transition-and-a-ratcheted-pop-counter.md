# ADR-0076: A still world gives a still image: five light-stability rules, a world-space history per source, a fade per transition, and a pop counter that only ratchets down

> **Status:** proposed
> **Date:** 2026-11-18
> **Related plan(s):** [Plan 0090](../plans/0090-the-pop-counter-scenes-on-lavapipe-count-the-pixels-that-break-the-light-rules-a-ratchet.md)
> (builds the counter and takes the baseline); [Plan 0086](../plans/done/0086-still-outdoor-light-baked-sky-visibility-a-long-history-solid-canopy-shadows-and-stair-history.md)
> (whose Phase 16 attributed the pops this ADR names; the baseline is taken after it merges)
> **Builds on:** [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the baked store, its queue and its `FADE`),
> [ADR-0053](0053-a-light-summary-per-block-and-a-two-level-parent-table-light-a-brick-with-no-value.md)
> (the fallback chain for a brick with no value),
> [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky's baked visibility and the long-history ray),
> [ADR-0027](0027-the-sun-shadow-is-one-per-pixel-ray-in-the-light-pass.md) (the sun's jittered ray),
> [ADR-0044](0044-direct-light-from-a-fixed-list-of-analytic-lights-one-shadow-ray-each.md)
> (the light list), [ADR-0033](0033-eye-adaptation-as-an-asymmetric-two-stage-exposure-on-the-gpu.md)
> (the exposure), [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> (the screen-space temporal pass the rays still use)

## Context

The owner's standing rule since 2026-11-03 is a stable image first: "a flame may flicker,
nothing else may" (2026-11-18). After Plan 0086's second window the pops still show "here and
there". They are not one defect. The frame's light is a sum of sources, and each source keeps
its own estimate and its own history: the baked emissive store with its queue (ADR-0046), the
parent-table fallback for a brick with no value (ADR-0053), the sky as baked visibility plus a
long-history ray where the bake is stale (ADR-0070), the sun as one jittered ray per pixel on
a screen-space blend (ADR-0027, ADR-0018), the analytic and fired lights (ADR-0044, Plan 0058),
and the exposure that scales the sum (ADR-0033). A pixel's value is a switch over which of
these answered, and every switch is a place a pop can come from.

Plan 0086's Phase 16 read the pops the window showed (`docs/plans/logs/0086.md`, 2026-11-18)
and they fall into three kinds. **A source handover or a cache-state switch in one frame:**
the dig flash is the stored part dropping the sky term the instant the stale bit sets while the
direct part takes it at its blend weight (with the sky alone, half the region moves by over 16
display levels for two frames); the seam between the baked sky term and the ray's is the same
switch at rest; a brick going unbaked to fallback to baked, or a ring re-baked after an edit,
lands as a brick-sized patch. **A stochastic sample whose history lives in screen space and
resets:** the dig's remaining dots are young sky-ray samples (about 26 of 50) and the key
light's reset pixels (about 24); the forest's grain after a turn is the direct part's history
restarting at age 0 on every disoccluded pixel; the fired lights' grain is the list's direct
sample taken as it is while the list changes. **Exposure amplifying both:** the anchor and the
auto exposure scale whatever stepped. The history agrees: Plan 0057's move of the emissive
light into a world-space store ended the turn flicker that two screen-space plans had not, Plan
0047 fixed the emissive history when 83 % of the flicker came from the probes, and ADR-0070's
context shows why a longer screen history cannot fix a dim tunnel end (a Bernoulli at hit rate
0.03 needs about 3200 frames for ten percent).

What the repo has against this is two things, neither a rule. Owner windows find pops and say
which look is right, at about an hour each, on one build, and they misattribute (Plan 0047).
Per-test bars (`fsr2_adds_little_flicker_on_a_still_vein`, the patch bars of
`gpu_emissive_stillness`, Plan 0086's "as still as the lamp" p99 bars) defend one mechanism
each with a statistic: a 99th-percentile change of 5 levels passes a few hundred popping pixels,
which is exactly "here and there". Nothing says what a light source owes the image, nothing
counts the pixels that break it, and nothing stops the next source from reintroducing the
class. Meanwhile seven more light plans are approved (0081, 0033 to 0037, 0084's reveal).

The gate's wall budget is 19 min (`docs/nfr.md` section 7, raised 2026-11-18) and the latest
gate read 1107 s, so a new gate test has about 30 s of headroom unless it replaces one.

## Decision

**Light stability is a rule with a mechanical gate, not a look.** Five rules say what every
light source owes the image; a counter measures the pixels that break each one on fixed scenes;
a committed baseline per scene and rule may fall and never rise; and every plan that adds or
changes a light source states where its history lives and how it fades.

**The five rules.** Each is stated with the measurement that decides it. The visibility
thresholds (`T_still`, `T_return`, `T_fade`, the fade's frame count `F`, the exposure rate
`R`) are calibrated once by the owner's eye on graded pops (Plan 0090 Phase 5) and then fixed
in the counter; until then they are the guesses Plan 0090 names.

1. **Still camera on a still world: a still image after settle.** After a scene's settle time
   every display pixel changes by at most `T_still` levels of 255 between consecutive frames,
   except pixels whose hit material, or whose face cell's dominant emitter material (ADR-0046's
   record), carries the material flag `FLICKERS`. A flame flickers by declaring it; the engine
   does not decide which materials may.
2. **View independence.** A settled pose, a 90-degree turn away, a turn back: the frame at the
   return matches the settled image at the pose within `T_return` levels per pixel, at once.
   The estimate that answers a pixel lives in the world (a store, a record, a bake), so looking
   away loses nothing. A screen-space accumulation may carry a world-space estimate's
   reprojection, never be the estimate.
3. **Every change is one monotonic fade.** Across an event (an edit, a light on or off, a
   source or cache-state switch: unbaked to fallback to baked, stale to re-baked, ray to bake, a
   tier change) each pixel's trace goes from its old value to its new one within `F` frames and
   stays within `[min(old, new) - T_fade, max(old, new) + T_fade]` throughout: no flash above the
   new value, no overshoot, no black dip below the old. `F` is one constant for every source
   (ADR-0046's `FADE`, 16 frames as built, is the candidate), so two sources fading at once
   fade together.
4. **Locality.** An event has a light-influence region: the edit's box dilated by the bake's
   reach, or a light's reach. Outside it no pixel changes by more than `T_still` across the
   event's frames. A re-bake, a re-window or a queue reorder that moves light elsewhere breaks
   this rule.
5. **Exposure moves at a bounded rate and only on real change.** The adapted log2 luminance
   (ADR-0033's `a`) steps by at most `R` EV a frame, monotonically toward its target, and in a
   still scene after settle it does not move (within 1/64 EV). Rule 5 is read on the exposure
   state, not per pixel, and the other rules are read on the display with the exposure's share
   attributed, so an exposure step that amplifies a pop is counted once, as exposure.

**The counter.** A headless harness on lavapipe (the software adapter CI has; `docs/nfr.md`
section 6) runs fixed scenes with fixed seeds at a low resolution, reads back the display and
the per-source targets every frame, and counts, per scene and rule, the pixels that break the
rule. Every counted pixel is attributed to the source whose own change is largest (baked store,
fallback chain, sky bake, sky ray, sun ray, list light, exposure) and to the transition it was
in (a cache-state switch of its hit's record between the two frames, a young history, a reset,
an exposure step), building on Plan 0086's `--dump-black` and `tests/support/defects.rs`. The
counts are a committed baseline, one per scene and rule; the source split is printed beside
each, not ratcheted, since the attribution is a heuristic and must not flip a verdict. **The
ratchet:** a count at or under its baseline passes; a count over it fails by scene and rule,
naming the sources; a bless lowers a baseline and never raises one;
raising a baseline is a hand edit with a dated line in the file that the review reads. Cheap
scenes run in the gate tier within the headroom Plan 0090 reads, long ones in soak, which the
close review runs for the crates a plan changed. The counter adds nothing to the frame: its
read-backs run only under the harness.

**The binding clause for plans.** Every plan that adds or changes a light source, a cache level
or fallback, an exposure stage, or a term of the composite states two things: where the source's
history lives (a world-space store, never a screen-space accumulation alone) and which fade
carries every switch of its source or cache state (rule 3). The architect review checks both
statements and runs the counter at the close; a plan whose counts rise does not close.

## Consequences

### Positive
- "Stable" has a definition. A pop is a counted pixel with a named source and transition, so a
  fix is a number that fell, not an owner's "better".
- The ratchet turns every light plan into a step down. The next source cannot reintroduce the
  class unseen, which the seven approved light plans need.
- The attribution replaces the hour-long window as the first instrument; the window keeps the
  look calls and the calibration.
- Rules 2 and 3 put the design pressure where Plan 0057 showed it pays: histories in the world
  and fades at every switch, instead of longer screen histories.

### Negative
- The counter is a test harness of some size (scenes, read-backs, a classifier, a baseline
  file) that pays nothing on screen until a source is fixed against it.
- The gate has about 30 s of headroom; the counter's gate share either fits it or retires the
  per-mechanism stillness tests it subsumes, and the long scenes run only at closes and weekly.
- The thresholds are an eye's, read once at one resolution and display; a pop under `T_still`
  passes however many pixels it covers, and the counter counts pixels, not the eye's grouping.
- A ratchet never rising means a plan that trades a large pop for many small ones under the
  threshold passes; the attribution table in the log is what the review reads against that.
- Rule 1's `FLICKERS` flag is a material-table bit the engine reserves before any material uses
  it; `vxe-world` gains a flag for a renderer's rule.
- Rule 2 is strict ("at once"), and today's rays will break it on every disoccluded pixel; the
  baseline records that, and the ratchet asks only that it not get worse.

## Alternatives considered

### Alternative A: Owner windows only
What we do: a windowed check at each look plan's end, the owner naming pops by eye. It finds
pops but cannot prevent their return, reads one build at a time, costs the owner about an hour,
and misattributes (Plan 0047's history, 83 % from the probes). It stays as the calibration and
the look call; it lost as the gate because it is not one.

### Alternative B: Fix each pop ad hoc, plan by plan
Plan 0086's Phases 16 to 23: attribute the window's defects, fix each from a short list. It
works for the pops in hand and says nothing about the next source; each plan writes its own
statistic and bar, and no bar says what a light source owes the image. It lost because the
defect is structural (many estimators, many histories, many switches) and a per-defect fix
leaves the structure.

### Alternative C: A hard zero gate from day one
Every scene reads non-zero today (Plan 0086 Phase 1's tables: p99 changes of 5 to 15 levels at
the defaults, the dig's dots, the forest's grain after a turn). A zero gate blocks every merge
or is disabled in a week. The ratchet lost nothing of the rule and keeps the gate green from the
first commit.

### Alternative D: Statistical bars (p99, mean) per test instead of per-pixel counts
The repo's current form. A 99th percentile passes hundreds of popping pixels on a 320 x 180
grid, which is "here and there". Counts are what the eye sees; the percentile is kept as a
printed reading beside them.

### Alternative E: One unified world-space light cache, now
Fold every source into one estimate with one history and one fade, superseding ADR-0027,
ADR-0044, ADR-0046 and ADR-0070 together. It may be the end state, but no reading says which
sources need it, and it is a rewrite of the lighting base in flight. The five rules are what
such a design would have to satisfy; the counter says whether and where the ratchet stalls,
and that reading decides it later.

## Outcome

_(Added at acceptance if Plan 0090 falsifies something above.)_
