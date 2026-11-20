# ADR-0080: The live queue bakes what the image reads: a record by rule where no ray is needed, a sample tier by the chunk's size on screen, and a slice that drains what is in view

> **Status:** proposed
> **Date:** 2026-11-20
> **Related plan(s):** [Plan 0096](../plans/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md)
> **Builds on:** [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the per-chunk store and its queue), [ADR-0048](0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the order by the view, the withdrawal), [ADR-0052](0052-the-live-queues-bake-budget-is-time-per-second-not-a-slice-per-frame.md)
> (the budget's unit), [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky channel and the column-map clause), [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rule 6 (the undrained image is the judged image; the drain is a budget)

## Context

ADR-0046 bakes every chunk in the mirror's reach at one count, 4096 samples a brick direction,
after a coarse pass at 256, nearest first by ADR-0048's score, inside ADR-0052's slice. Indoors
that drained; the owner's cave sessions saw a queue that rests. Outdoors it does not: at the
owner's window of 2026-11-20 (Plan 0094's readings log, Phase 7) the forest held 12 752 to
16 150 jobs of a 17 077-chunk residency and landed 9 to 38 chunks a second, so the precise light
of a scene arrives 7 to 30 minutes after the load, and the wobble spike read two thirds of the
far pixels still changing after a stop as faces waiting for a record. The owner's rule, now
ADR-0076's rule 6: "if we have to wait for a minute for light to settle, something is wrong with
the light and we need approximate things, it is not acceptable for user".

The arithmetic says no slice fixes it. From Plan 0067's readings a full chunk draws about 1.2e6
samples, 6.5 ms of GPU on the RTX 3080 Laptop at 5.1 ns a sample and 60 ms on the RADV RENOIR
at 47.3 ns; 16 000 chunks are 104 s and 960 s of GPU before any slice, against a 16.6 ms frame
whose bake line is 1.0 ms. Meanwhile most of those chunks hold nothing the image reads: a
uniform chunk has no exposed face, a chunk buried under the column map with no emitter within
the bake's reach can only ever read zero under ADR-0070's own clause (a ray ending under the
column top is not sky), and a chunk whose voxels project under two pixels is read through the
far-normal band's filtered normal and the half-resolution lighting, where 4096 samples a brick
direction and 256 give the same display value within the owner's eye (a guess the plan's window
reads). What the image reads is a record at the precision the view can see; what the queue bakes
today is every record at the precision the nearest view could see.

## Decision

**The live queue bakes what the image reads, and the time to land what is in view is a budget in
`docs/nfr.md`.** Three rules, in the order they pay:

1. **A record by rule where no ray is needed.** A listed chunk with no exposed face is marked
   baked-empty and holds no record; a listed chunk whose reach box (the chunk dilated by the
   light radius) lies under the column map's lowest top over that footprint, with no emitter in
   reach, gets a zero record written on the CPU with the baked bits set and the sky-stale bit
   clear, and no GPU job. What undoes a bake undoes a mark: a dig in the chunk or its reach, a
   column top that drops over the box, an emitter arriving in reach. Where the column map is
   not final for the footprint the chunk is baked, not ruled.
2. **A sample tier by the chunk's size on screen.** A chunk's band is its nearest corner's
   projected voxel size against the far-normal band `[lo, hi]` (pixels a voxel) at the output's
   focal length: over `hi` the full count, between a mid count, under `lo` the coarse count **as
   its final record**. The tier is written in the record's head word. When the camera's chunk
   changes, a chunk now in a nearer band than its record is re-listed as a refinement and lands
   as one `FADE` (rule 3 of ADR-0076); a listed job whose band fell is withdrawn if not yet
   submitted (ADR-0048). A dig's ring bakes at the near tier whatever its band. The counts are a
   settings row per profile.
3. **A slice that drains what is in view.** The budget (ADR-0052's unit) has two values: a
   drain value while any chunk in the view cone waits for its tier's record, and the line's
   value once none does. Both are settings rows per profile; the drain value is the owner's pick
   at the plan's window, against the frame rate they feel.

The queue therefore reaches a **landed** state (every in-view chunk at its tier), and the budget
is the time from the first world frame to that state, per adapter and profile, beside the time to
drain the whole residency.

## Consequences

### Positive
- The queue's work scales with the surface the view can see, not with the residency's volume;
  the step's candidate pool shrinks with it before any step optimisation.
- Rule 6 gets its budget: a landed state exists, so "one fade when it lands" has a landing, and
  the drain's seconds are a number on the HUD and in `run.json`.
- The slow adapter has a path that ends: the Portable profile's tiers and drain value are its
  own, not the RTX's divided by nine.
- Nothing changes for an indoor scene with emitters: no chunk there is buried without an emitter
  in reach, and the near band is where the lamp is.

### Negative
- **The store is camera-dependent.** ADR-0046's bit-identity "whatever the slicing" becomes
  bit-identity per tier; a far ridge approached slowly re-bakes through two tiers (two fades),
  and the `--bake-file` hash and the mirror oracle read the tier beside the bytes.
- **The far tier is a look call.** 256 samples a brick direction has only ever been a first
  pass; as a final record on a far canopy its shadow is the coarse pass's. If the eye rejects it
  the mid count becomes the far tier and the drain roughly doubles.
- **The rule's correctness rides on the column map's finality and its invalidation.** A dig that
  opens a buried chunk to the sky must re-list every chunk the rule covered, or a cave stays
  dark under a new hole; the plan tests it, and the rule is off where the map is provisional.
- Two more settings rows (the tiers, the drain value) and a tier field in the record head, read
  by the shader's store read and the tests.
- A drain slice over the bake line while the owner looks at a coarse image is GPU taken from the
  frame when it is most visible; the Floor profile's 16.6 ms holds it only because the RTX frame
  outdoors is about 6.7 ms at 1080p.

## Alternatives considered

### Alternative A: A larger slice, one count
Keep ADR-0046's single count and raise the slice (or the per-second budget) until the queue
drains. On the RTX that is minutes of GPU at any slice the frame can give; on the RENOIR, hours.
It lost on the arithmetic.

### Alternative B: A progressive queue that refines every record to the full count when idle
Bake every chunk coarse first, then refine all of them as the slice allows, nearest first. It
has no landed state: the step never rests, every refinement is a landing with a fade somewhere
in view, and rule 6's "one fade when the record lands" has no single landing. It lost because
the owner's complaint is exactly the image that keeps changing.

### Alternative C: A per-brick skip inside the GPU job
Let the job's shader return early for bricks under the column top or with no emitter in reach,
keeping every chunk listed. The GPU cost falls, but the chunk is still listed, scored, sorted
and submitted every step, which is the main-thread cost the plan chases (6.3 ms of step at a
depth over 14 000), and the record still occupies the store. It lost as the only mechanism; the
shader may keep its own early-outs.

### Alternative D: The bake on the stream workers' CPUs
Rejected in ADR-0046 and not reopened: at 5 ns a sample the GPU is the fast path, and rule 1
already gives the CPU the records that need no sample.

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
