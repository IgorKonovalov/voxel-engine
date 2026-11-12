# ADR-0052: The live queue's bake budget is milliseconds per second, not a slice per frame

> **Status:** accepted (2026-11-11, owner: the direction is a standing rule before Plan 0067's
> go; the values, the default `ms_per_s` and the cap, stay the pick of its window), with an Outcome at Plan 0067's close (2026-11-12): the default
> stays `frame:1.5`
> **Date:** 2026-11-11
> **Related plan(s):** [Plan 0067](../plans/done/0067-the-frames-cpu-a-cheap-queue-step-a-bake-budget-by-time-and-no-black-for-missing-light.md)

## Context

ADR-0046's live queue bakes a chunk's static light in slices on the render thread, one slice a
frame, `DEFAULT_BAKE_SLICE_MS` 1.5. ADR-0046 guessed a larger slice for the slow adapter ("4 ms on
the Cezanne") and left it to a reading. Plan 0060's window read the result (`docs/plans/logs/0060.md`,
Phase 4): on the RTX 3080 Laptop at 100 to 140 fps the slice is 150 to 210 ms of bake a second;
on the Radeon (RADV RENOIR) at 15 fps it is about 22 ms a second, and each sample costs 92 ns
there against 6 to 10 on the RTX. After 137 s at the window's size the store held 526 bricks with
36 932 missing, and the owner saw black distant caves. The owner asked (2026-11-11, `docs/backlog.md`)
for "a bake budget per second rather than per frame".

A budget stated per frame ties the bake's throughput to the frame rate: the slower the GPU, the
less light it bakes a second, which is the opposite of what a slow adapter needs. A budget stated
per second of wall time holds the throughput and lets the frame pay for it: on a 66 ms frame,
150 ms a second is 9.9 ms a frame, 15 % of the frame.

## Decision

The live queue's bake budget is **milliseconds of GPU time per second of wall time**, with a cap
on the share of any one frame. Each step's slice is `ms_per_s x the last frame's interval`,
clamped to `cap x interval` and to the slice's floor. The per-frame slice stays as a named mode
for comparison. The values (the default `ms_per_s`, the cap) are the owner's pick in Plan 0067's
window, read on both adapters; until then the default stays the per-frame 1.5 ms. The budget is
a `vxe-render` setting the consumer sets; the fly-through exposes it as a flag and a key.

## Consequences

### Positive
- The far light arrives at the same rate a second on every adapter, so a slow adapter's caves fill
  at the rate the owner tuned on the fast one, not at a tenth of it.
- The budget is a figure `docs/nfr.md` can carry as a line, with the frame's share as its cost.
- The slice's size follows a hitch down: a long frame gets a long slice once, capped.

### Negative
- A slow adapter's frame gets longer by up to the cap's share (16 ms of a 66 ms frame at 0.25);
  the owner trades frame time for light there, in the open.
- The Radeon still bakes a tenth of the RTX's samples in the same milliseconds; the budget gives
  it the RTX's time, not its light. Plan 0067's fallback covers the wait: a brick with no value
  takes its neighbour chunks' or its parent cell's baked light (owner, 2026-11-11), and stays
  black where neither has any.
- The slice now depends on a wall-clock interval the shell measures; a headless run must pass a
  fixed interval to stay repeatable, as the tests do.

## Alternatives considered

### Alternative A: Keep the slice per frame, larger on a slow adapter

ADR-0046's guess. It needs a per-adapter table nobody maintains, and it still halves the bake a
second whenever the frame rate halves for any other reason (a bigger window, a heavier scene).

### Alternative B: A fixed share of each frame's GPU time

Adapts to the adapter without a clock, but is self-referential (a longer slice lengthens the frame
that sizes the next slice) and gives the fast adapter less bake exactly when it has headroom. The
cap in the decision keeps the one property worth having from it.

### Alternative C: The bake on a second GPU queue

`wgpu` exposes one queue; there is no async compute to run the bake beside the frame. Rejected
for now, not forever.

## Outcome

Plan 0067 closed 2026-11-12. **The window kept the per-frame slice as the default.** The unit is
built as the Decision says, `vxe_render::bake::Budget::PerSecond { ms_per_s, cap }` beside
`Budget::PerFrame { ms }`, each step's slice `ms_per_s x the interval` clamped to `cap x interval`
and to the floor, and the fly-through exposes it (`--bake-budget frame:MS|second:MS,CAP`, key `5`,
named in the banner and the HUD). The owner's window of 2026-11-12, both adapters, cycling
`frame:1.5`, `second:150,0.25` and a higher value: "keep frame:1.5". The default `Budget` is
`PerFrame { ms: 1.5 }`; the per-second mode stays a render setting a profile may pick (Plan 0069),
by the owner's rule of the same day that a setting that costs nothing when off stays a setting.
So the Decision's direction stands as an available mode, and the default it expected to replace
stays. The readings (`docs/plans/logs/0067.md`, Phases 3 and 7):

- **The Radeon's frame under the cap** grew within the cap's share (Phase 3's bar, met).
- **The same bake a second on both adapters** did not hold: under `second:150,0.25` on the streamed
  glide the RADV RENOIR read 148 ms a wall second and the RTX 3080 Laptop 91 to 92, its submits
  drawing about 5.3 ms of an about 8 ms slice. The cause is unread (`docs/backlog.md`, "The RTX's
  submits draw less than their per-second slice").
- **At the default `frame:1.5`** the glide reads about 40 ms of bake a wall second on the RTX and
  30 on the RENOIR; the slow adapter's far light is what ADR-0053's missing-light chain covers.
