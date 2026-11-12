# ADR-0046: Static emissive light is baked per chunk when the chunk loads or changes, into a store of per-face cells over per-brick values

> **Status:** accepted (2026-11-09, Plan 0059's close), with an Outcome: the store is the brick level alone; a second Outcome at Plan 0067's close (2026-11-12): a partial reach is a setting
> **Date:** 2026-11-05
> **Related plan(s):** [Plan
> 0059](../plans/done/0059-loaded-light-a-per-chunk-store-baked-at-load-and-at-edit.md)
> (builds it; its close accepts this ADR, and the owner's verdicts in its Phases 3 and 13 can
> overturn it).
> Amended 2026-11-05 while proposed: the order, look first and cut later (the section below).
> Replaces, on the owner's verdict in that plan's Phase 13, the table, fill, preload and guard of
> [ADR-0043](0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md),
> and keeps its nearest read, its empty screen history and its lamp-bounce channel. Refines
> [ADR-0025](0025-light-candidates-from-a-power-over-distance-table-of-near-emitter-chunks.md)
> (the candidate table is built per bake, centred on the baked chunk) and
> [ADR-0045](0045-the-per-face-table-stores-irradiance-at-unit-emission-and-the-read-applies-the-scale-now.md)
> (a face keeps a dominant emitter material, not a kept emitter). First use of the stream-worker
> hook of [ADR-0017](0017-mirror-cpu-work-bounded-in-the-frame-loop-narrowing-on-stream-workers.md),
> whose one-owner rule stands. Leaves
> [ADR-0044](0044-direct-light-from-a-fixed-list-of-analytic-lights-one-shadow-ray-each.md) as it
> is.

## Context

The owner's request, from the window of 2026-11-05: when I walk into a place, the light is already
there. "preload doesnt really change anything - or it should be much much bigger"; "it feels like
this is wrong way"; and of the same table, "once light are loaded - all seems to be fine and not
flickering". ADR-0043's storage idea held: a still face read from a settled value does not flicker
("there is no flicker nether when i turn or go forward/backward"). What failed is the fill.

Plan 0057's readings (`docs/plans/logs/0057.md` on its branch; RTX 3080 Laptop unless named,
labelled
indicative where the machine was loaded):

- **A view-driven fill overruns any cache.** At the window's 843 x 459 lighting grid the level's
  view
  loaded the 2^20 table to 0.257, over the preload's 0.25 cap, so the preload claimed nothing; the
  hall read 993 304 cells and 194 079 missed pixels. The scripted path peaks at 789 657 cells in the
  hall.
- **The young faces are the "dark patches".** In the supplementary vein view at frame 1, 0.44 of the
  hits beyond 64 voxels read over 8 levels darker than their settled value, and still 0.018 at frame
  64 under FSR 2. A radius does not reach them: `R` 64 leaves "distant caves are too bad".
- **The density is modest.** Faces toward air within 64 and 96 voxels: 28 482 and 98 025 at the
  vein,
  19 467 and 47 242 at the patch, 7 243 and 61 066 at the hall. So about 6 900 faces a chunk at the
  vein, 3 300 to 4 600 at the patch, 1 700 to 4 300 at the hall, in about 187 surface bricks at
  about 34 faces a brick (the vein's bricks with faces, over the sphere's volume in chunks).
- **A sample is cheap.** One resampled emissive sample (8 candidates, one shadow ray) costs 4.3 to
  4.6
  ns on the RTX in the light pass's dispatch (0.0281 ms for 6 571, 0.0592 ms for 12 798) and about
  50 ns on the Cezanne (0.337 and 0.607 ms, assuming the same samples, which its row does not
  print).
- **The grain is the estimator's.** A face of `n` samples sits about `82 / sqrt(n)` display levels
  rms off its truth (Plan 0057's settle derivation), independently per face. One level needs about
  6 700 samples. A 3 x 3 coplanar tent multiplies this by 0.375. **That holds in the hall and not
  beside emitters:** at 4096 samples a face, `G_noise` p50 / p90 reads 0.91 / 2.92 in the hall and
  3.18 / 13.78 at the vein (Plan 0057's Phase 7 "before" rows, native), about six times the hall's
  p90. By `1 / sqrt(n)` the vein's p90 reaches 1 level at about 780 000 samples.

Plan 0057 rejected "a fill at chunk upload" on 2026-11-04 for four reasons. This design answers
each: the work is GPU work in the frame (a bounded slice, and it removes the light pass's emissive
sampling); `light_radius` holds about 1.6 million faces, over the table's size (the store is not a
hash, and most of them are read at the brick level); a face's light depends on chunks uploaded after
it (a chunk bakes only once its reach is resident); and an upload fires once, not when the camera
comes near (quality rises by distance from a queue, and the upload only starts it).

`wgpu` exposes one queue, so "off the frame" cannot mean a separate compute queue. It means a slice
with a budget inside each frame.

## The order: look first, cut later

The owner, after Plan 0057's second window: "what if we go differently - no matter performance, but
build something that would look really polished, then we will see where we can cut performance?"; on
its scope: "agreed, only light. antializing next".

**The first thing built is a reference with no budget**, and the owner judges it before any budget
work starts. It is this ADR's store at its limit: a face cell for every face of every chunk in a bounded
piece of the world around each of the window's poses (no brick level, no rings, no queue; Plan 0059's census
put a whole level at about 15 h on the RTX, so the owner, 2026-11-05: "Bounded piece, bars kept"), baked before the first frame with independent seeds and no
filter, each face sampled until its own standard error through the display transform is under half a
level (at least 4096 samples, at most 2^20), read at native resolution with full-resolution lighting
and no upscaler. A dig re-bakes the box and the bricks within 32 voxels, in the frame, at a hitch nobody
budgets; ring 3 is deliberately out of that first look and says so on screen.

**Everything else in the Decision below is the expected destination, not the commitment.** The brick level,
the rings and their samples, the shared seeds and the filter, the slice, the fade and the fresh start's
seconds are cuts from the reference: each is read against it, shown beside it, and kept or refused by the
owner, one at a time. A cut the owner refuses changes this ADR's Decision before it is accepted, and
"a guess" below means a guess. Antialiasing comes between the reference's verdict and the first cut, in
its own plan, and the resolution cut needs its answer.

## Decision

**Each resident chunk within a bake radius holds its static emissive irradiance in a store the
renderer owns, computed for every face of the chunk by a budgeted GPU job when the chunk becomes
bakeable and when an edit changes it, and a pixel reads its face's value from that store. There is
no view-driven fill, no per-frame emissive sampling for a cube face and no screen history.** The
values below marked "a guess" are read in Plan 0059's cut phases, against the reference.

- **Two levels, in one pool.** A surface brick holds six direction values, each the mean of that
  direction's faces: the **brick level**. A chunk near the camera also holds one 8-byte **face
  cell** a face: `rgb9e5` irradiance at unit emission, the 12-bit key in the brick (9 of voxel, 3 of
  side), four flag bits, and the 16-bit dominant emitter material. Runs are sorted by key and a
  512-entry directory per chunk finds a brick's run, so nothing is hashed and nothing can miss.
  A brick over `BRICK_FACES_MAX` faces (512, a guess) stays at the brick level, so a pathological
  chunk cannot exhaust the pool. The pool is one storage buffer, 128 MiB by default.
- **The bake.** A job takes a chunk or a set of its bricks, draws `N` samples per listed face (the
  existing resample and shadow walk, at unit emission, ignoring the lamp and the emission scales)
  into a transient buffer of Q17.15 sums, filters, writes `rgb9e5`, and fades the new values in
  over `FADE` frames so a tier change does not pop. A brick-level job draws per brick direction
  from points on that direction's faces. Tiers re-bake from scratch and replace; no raw sums stay
  resident. The candidate table is built per job, centred on the job's chunk, from every emitter
  chunk within `light_radius` of it, and each sample carries a window that falls to zero at
  `light_radius` from the face, so a table's edge is no seam.
- **Rings, with cost flat by construction.** Samples fall with the square of distance, so each ring
  costs the same at a given speed:

  | Ring (chunks from the camera) | Level | Samples | Cost per chunk at 6 900 faces, RTX / Cezanne |
  |---|---|---|---|
  | 12 | brick | 64 per brick direction, about 30 000 a chunk (a guess: 2.5 directions) | 0.14 / 1.5 ms |
  | 4 | face | 32 | 1.0 / 11 ms |
  | 2 | face | 128 | 4.1 / 45 ms |
  | 1 | face | 512 | 16.5 / 180 ms |

  The bake radius is at most `r_full - light_radius` (12 at the defaults), the largest for which a
  chunk's whole reach can be resident. Beyond it a hit carries no emissive part, as beyond the light
  radius today, at three times the distance.
- **The scheduler is one queue on the render thread** (ADR-0017), nearest first, edits ahead of
  loads, with a slice set from the previous frames' timestamp readings: **1.5 ms on the RTX, 4 ms
  on the Cezanne with `N` and the brick samples a quarter** (guesses, read in Phase 4). A chunk
  enters only when every chunk within `light_radius` of it is mirrored or known empty, so a later
  upload cannot change its light. Faces are listed on the stream workers from the masks of the chunk
  and its six neighbours, a pure function through ADR-0017's hook, and the render thread alone
  allocates and writes. A bake outlives its ring: values stay in the pool until pressure evicts the
  farthest, so walking back costs nothing.
- **What a pixel shows.** Its face cell if one exists; otherwise its brick's direction value;
  otherwise, for a chunk not baked yet, no emissive part. A dark face is therefore possible only
  where the brick level has not arrived, and the pre-roll below and the ring's lead over the camera
  exist to keep that out of sight. Unlike the table's young cell, a value on screen is never a mean
  of a few samples of one face: its lowest forms are the brick level and a tier through the filter.
- **The fresh start bakes before its first frame.** `Frame::bake_until` runs large slices with
  nothing on screen until the face rings and the brick level within 6 chunks are done (about 66 M
  samples at the vein's density, about 0.3 s on the RTX, and on the Cezanne 3.3 s or about 0.8 s at
  its
  quarter of the samples, a guess); the queue takes
  the brick level out to 12 in slices after that. A teleport calls it again. The consumer owns the
  wait.
- **The grain: shared seeds, a filter, then the count.** Faces of one side in a chunk draw from one
  random sequence, so their errors correlate and read as a slow gradient, not as salt and pepper,
  and a 3 x 3 coplanar tent (ADR-0043's same-orientation rule: a light never crosses a crease; the
  stepped neighbour at half weight) runs once in the job and is stored, with no per-frame pass and
  no read-time cost. Independent seeds through the filter read 1.4 levels rms at 512 samples, by
  arithmetic; **shared seeds have no reading yet, and their cost is a seam at chunk borders**, where
  two different random sequences meet. Plan 0059's Phase 4 reads the seed arms against the reference.
- **Invalidation by brick, in three rings.**
  1. **The box** (the edit's box dilated by one voxel): relisted and baked at 32 samples in the
     frame
     the mirror receives the edit, ahead of the frame's read, then refined to the ring's tier. A new
     face whose job the frame's cap postponed reads its brick's old direction value, so a hole is
     never dark.
  2. **Near** (bricks within 32 voxels of the box): queued at the ring's tier at edit priority.
  3. **Reach** (bricks within `light_radius` of the box): queued at low priority, and only if the
     box's faces read light before or after the edit, or the edit changed an emitter. A carve whose
     faces no emitter reaches cannot have opened a path for light.

  Old values show until their replacement fades in, and a brick dirtied again is deduplicated and
  restarted. A collapse's `drop_region` or `remove_region` is an edit like any other (ADR-0008), one
  ring-1 box a tick. A runtime emission scale moves no bake: the read multiplies by the dominant
  material's scale now, so a flame flickers at full amplitude and reopens nothing. A material swap
  that adds or removes an emitter is an edit that opens ring 3.
- **The lamp, the list and the bounce are untouched.** The lamp's exact part keeps its short screen
  history and a-trous, ADR-0044's list follows it, and the lamp-bounce probes gather as ADR-0043
  built them. The composite sums `store value x scale x exposure`, the lamp's part and the gather.
  The
  bake never sees them. A hit with no face keeps ADR-0043's fallback.
- **Determinism.** Baked light is derived and render-side: it enters no `vxe-world` state, op log or
  hash, and no gameplay query may read it. A bake's seeds are keyed by chunk, side and sample index
  and its sums are integer, so its result is **bit-identical on one adapter whatever the slicing,
  the camera's path or the job order**, and that is asserted. Across adapters the shading's float
  rounding differs: the difference is a reading (a target of under 1 level), not a bar, and goldens
  stay blessed on lavapipe as they are today.

**The table goes only on the owner's verdict.** `--light baked|table` holds both until Plan 0059's
Phase 13. On a yes, the table, its fill, preload, guard, sweep and age-out are deleted, and nothing
replaces them as a cache. On a no, the store comes out and this ADR is superseded.

## Consequences

### Positive
- A place is lit before it is seen, at any range within the bake radius and whatever the view, which
  is what the owner asked for and what no radius reached.
- A still image does nothing at all once the queue is empty: no sample is drawn and no value moves.
  The cost at rest is one lookup a lighting pixel.
- No load, no misses, no cap. Memory is about 49 MB at the vein's density against the table's
  201 326 592 B at 2^22 (452 near chunks at 62 KB plus 3 167 far at 6.5 KB, surface share 0.5 a
  guess; Plan 0059's census read 0.990 on the level and 0.378 in the showcase, so this is re-read).
- The dug hole has a value in its first frame, and the far faces no longer wait on their footprint.
- A bake is hit-centred: a face far from the camera samples its own emitters, which removes
  ADR-0025's
  named weakness in a tunnel without its rejected Alternative B's rebuild cost, since the table is
  per job.
- Scaled emission gets simpler than ADR-0045: bakes run at unit emission, so a material at scale 0
  still bakes, and no divide is needed.

### Negative
- **It is new machinery:** a pool, a directory, a queue, a listing hook, a bake pipeline and rings,
  where
  the table was one buffer and a hash. Plan 0057's Phases 2 to 5 are largely undone if it wins.
- **The brick level is soft.** Beyond 4 chunks a wall reads at 0.8 m: about 30 display pixels a
  brick
  at 256 voxels, with shadow edges blurred or lost and light leaking across a thin wall inside a
  brick.
  The "Lego look" risk is the owner's to judge in the window; the ring and interpolation can move.
- **The filter softens by one face**, as Plan 0057's Phase 7 Part B does: a hard shadow edge becomes
  a
  one-face ramp, and a pit's bottom takes part of its rim's light.
- **Shared seeds may trade grain for seams,** and a low-frequency error of a few levels may cover a
  whole wall at the 32-sample tier. Plan 0059's Phase 4 can send this back to independent seeds at twice the
  cost.
- **Staleness at reach.** After a dig, a far face lit through the new opening keeps its old value
  for
  seconds, and ring 3's test is a heuristic: a block placed in lit air with no faces near it reads
  as lit only through its own new faces. Accepted against re-baking about 270 chunks per dig.
- **A wait at the fresh start,** about 0.8 s on the slow adapter at its quarter of the samples (a
  guess), and a
  grainier bake there for it.
- **Emitter samples for a larger radius.** Ranges must exist within bake radius plus light radius
  (16
  chunks), not 4. ADR-0025 read 997 740 of 1 048 576 samples at the streamed window under its
  uniform pool, so it fits narrowly and extraction grows; Plan 0059's Phase 1 reads it.
- **Dynamic geometry casts no emissive shadow.** A future voxel entity does not shadow baked light,
  as it does not shadow the table's settled cells. M6 is the place to say otherwise.
- **Plan 0058 needs a refresh:** its emission-scale phase reads a cell's kept emitter, which is
  gone.

## Alternatives considered

### Alternative A: The view-driven table alone (ADR-0043 as built)
A face gains samples when a pixel touches it. The owner saw its stability and its fill: "when I move
through caves, I see dark patches everywhere - it settles slowly". Its store fills with what the
view
touched, so its size follows the path rather than the world, and Plan 0057's Phase 7 buys room at
201 MB with an age-out. It lost because a face's light depends on when someone looked.

### Alternative B: The radius preload, larger (Plan 0057 Phase 4 Part 2b)
A cursor sweeps bricks within `R` of the eye and fills them. The owner: "preload doesnt really
change
anything - or it should be much much bigger". Cost grows with `R` cubed in cells and it is still a
radius that fills while the player looks, with a hash in front of it and a budget a frame paid by
cells that the view then shares. Baking is that idea made bigger by moving where the work is
decided:
per chunk, once, with a store sized for it.

### Alternative C: Screen-space history
ADR-0018 and ADR-0036, already rejected in ADR-0043: a pixel's history is lost on every turn, and
"no
good", "still settles for a long time".

### Alternative D: Faces only, no brick level
This is the reference's form, with no budget, and rejected here as the destination only; the reach cut
(Plan 0059's Phases 6 and 7) tests the rejection against the owner's eye.
Every chunk within 12 holds face cells: about 220 MB at the vein's density and the sharpest far
field.
A chunk not yet baked has nothing to show, which is the young cell again, far faces under a pixel
alias, and 32 samples a face at radius 12 costs 9 times ring 4's at any speed. Rejected for the
coarse level's cost and its role as the value that is always there.

### Alternative E: Per-brick irradiance with per-face occlusion from the masks
About 10 KB a chunk and very cheap. It leaks across thin walls inside a brick and has no real shadow
edges up close, where the owner stands. Plan 0057's Phase 7 rejected coarser storage for the same
reason.

### Alternative F: The bake on the stream workers' CPU
A second sampler in Rust, off the frame and bit-identical everywhere. At a guessed 500 ns a sample
the
walking load of 11 M samples a second is about 5.5 cores, over the floor's 4, and a dig's light
arrives
frames late. It also doubles the sampler to keep equal with WGSL's. Plan 0059's Phase 4 reads the real figure
once,
and a reading under about 100 ns a sample reopens this. **Both** (CPU far, GPU near) pays for both
samplers
and adds a seam between them.

### Alternative G: The sample count alone, no filter or shared seeds
The reference is this at no budget, with the count set per face by its spread; rejected here as the
destination only.
One level needs about 6 700 samples a face: about 210 ms a chunk at 6 900 faces on the RTX. It is
out of
reach within any budget above. A brute count of 1024 through the filter (0.96 levels, arithmetic)
takes the
walking cost to about 1.5 ms a frame, the whole slice, and stays the fallback if Plan 0059's Phase 4
refuses shared seeds.

### Alternative H: Emitter-out rays
Rejected in Plan 0057 for the preload and for the same reason here: a ray from an emitter finds a
face with odds
in proportion to the solid angle it fills, so the dim corner lit through a gap is found last.

### Alternative I: Whole chunks in reach on every edit, or the guard kept beside the near rings
Whole chunks means about 270 chunks per dig, so a player digging never settles and near refinement
starves.
ADR-0043's guard re-shades in view at about 0.09 ms every frame and needs a kept emitter in each
cell (about 16 B a
face), which brings back a part that depends on the view.

### Alternative J: The table kept as the not-ready cache
Two stores and two read paths for good, plus the table's memory, and the young-cell look comes back
exactly where
the bake is behind. The brick level does that job in 24 B a brick.

### Alternative K: Fixed-point shading for bit-identical light on every adapter
A second march in integer WGSL at a speed cost, for a property nothing consumes: goldens are blessed
on one adapter and
no gameplay reads light.

### Alternative L: The budgets first, the owner's look on the way (this ADR's first order)
The plan as first approved: a spike with a stop rule, a still bake at the budgeted tiers shown at its third
phase, then the queue and the rings. Its first look would be grain and softening from counts picked for a
frame budget, so a fault could be the design or the cut, and Plan 0057 spent eight phases fitting a design to
budgets before the owner called it the wrong way. Rejected by the owner's direction.

### Alternative M: Cut down from the table
Raise the table's count, radius and table size until it looks right, with no new store. It keeps the view-driven
fill the owner rejected: however deep, a face is young the first time it is seen, and "dark patches when moving
fast" is that fault.

### Alternative N: A still reference offline, and no window
Path-trace reference images headless, and compare strips. It is the cheapest reference, and it cannot show what the
owner is judging: moving fast, turning and digging. Plan 0057's strips met several bars the window did not agree with.
The owner also said of this kind of evidence that the judgement is in the app (Plan 0055).

## Outcome

Accepted 2026-11-09 at Plan 0059's close. The direction held, and the owner picked it over the table in Phase 13 ("of
course backet, table as a fallback maybe"). The cuts the owner made by eye falsified most of the Decision's shape. The
body above is kept as written; this section is what was built. The readings are in `docs/plans/logs/0059.md`.

- **One level, no face cells.** The owner, Phase 7b: "Bricks everywhere". A surface brick holds one record a cell: an
  8-voxel cell (the brick), six `rgb9e5` direction values, each the mean of that direction's faces, and `material |
  direction mask << 16`. A face reads the 2 x 2 cells around it in its plane, bilinear, over the cells that have its
  direction. The face cells, `BRICK_FACES_MAX`, the rings of the table above, the shared seeds, the filter and the
  per-face stop are gone (Phase 14). Alternative E's leak inside a brick is the accepted price. Per-face cells near the
  camera are parked in `docs/backlog.md`.
- **Samples.** 4096 a brick direction, spread over its faces, independent seeds, no filter (Phase 7b: "8/4096 is
  fine, others are too noisy"). Shared seeds missed their seam bar in Phase 4 and were dropped.
- **The queue.** One queue on the render thread (ADR-0017), nearest by chunk distance, ties to the chunk ahead. The
  face listing runs on the stream workers through `Streamer::set_prep`. A chunk waits until its reach is mirrored.
  Before the first frame the chunks within 2 (Chebyshev) bake at 4096, then a coarse pass at 256 covers what is
  mirrored, for at most 5 s. The live slice refines coarse chunks nearest first and fades each over 16 frames. The
  slice is sized from its own GPU timestamps, with two submits in flight. The pool is one 256 MiB binding
  (`STORE_BINDING_BYTES`). A refused allocation leaves the old block in place, and the queue goes on.
- **Digs.** Rings 1 and 2 re-bake at the queue's settings ahead of every other job, with the slice raised while they
  wait, and fade over 4 frames from a provisional block written at the dig. Ring 3 (the light radius) opens when a
  touched brick was lit. On the RTX at native, a vein dig settles in 7 frames, none over 33 ms.
- **Readings against the guesses above** (RTX 3080 Laptop unless named). The first frame reads 2.2 to 11.6 s at the
  owner's window, not the 0.3 s guessed: Phase 1 read a sample at 36 to 64 ns, not 4.6. The temporal pass now carries
  the read, 2.97 to 3.11 ms native at 2530 x 1378 against the table's 0.87 to 0.93, while the lighting line falls
  below the table's. Across adapters, the vein piece's records read equal words 0.9998 and at most 0.204 levels
  apart between the RTX and the Cezanne on RADV, under the 1-level target.
- **Glass** (ADR-0047): a translucent voxel's faces join their brick's records, so a pane's entry face reads its
  brick's value.
- **Still open,** in `docs/backlog.md`: the queue's view order, and cancelling chunks that a fast flight has passed.

## Outcome (2026-11-12, Plan 0067)

**The reach rule holds by default; a partial reach is a setting.** Plan 0067 Phase 5 built
`vxe_render::bake::ReachMode::Partial` (`BakeQueue::set_reach_mode`, the fly-through's `--reach partial`,
key `8`): a chunk whose light reach crosses the streamer's full radius bakes from the chunks of its
reach that are resident, is marked partial in the queue's maps (not in the store), and re-queues as
a refinement when its reach arrives. The owner's window of 2026-11-12: "not much difference really
(full kept with chain)". `ReachMode::Full`, this ADR's rule, stays the default, and under it the
reach walk stops at the first chunk not resident full, as before. `Partial` departs from the rule
only when a consumer picks it; it stays as a render setting by the owner's rule that a setting that
costs nothing when off stays (`docs/plans/logs/0067.md`, Phases 5 and 6). The chunks the rule
leaves waiting are lit meanwhile by ADR-0053's missing-light chain. The live queue's near shell
still takes full jobs within Chebyshev 2 (`full_first`, owner "keep 2"); `off`, a coarse pass first
everywhere, is a setting.
