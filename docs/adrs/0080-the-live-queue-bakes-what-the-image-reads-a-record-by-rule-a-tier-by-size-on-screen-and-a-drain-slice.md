# ADR-0080: The live queue bakes what the image reads: a record by rule where no ray is needed, a sample tier by the chunk's size on screen, and a slice that drains what is in view

> **Status:** accepted 2026-11-23 at Plan 0096's close (v0.71.0), with an Outcome. **Amended 2026-11-20 (owner)**, while proposed, at Plan 0095's window
> W1: the far tiers gain a reach clause (rule 4), since under the default reach rule no chunk
> past about 11.2 chunks ever bakes and the far tier would have had nothing to bake; see
> "Context, amended" and Alternatives E to G
> **Date:** 2026-11-20
> **Related plan(s):** [Plan 0096](../plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md)
> **Builds on:** [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)
> (the per-chunk store and its queue), [ADR-0048](0048-the-live-queue-orders-by-the-view-and-withdraws-the-work-a-flight-has-passed.md)
> (the order by the view, the withdrawal), [ADR-0052](0052-the-live-queues-bake-budget-is-time-per-second-not-a-slice-per-frame.md)
> (the budget's unit), [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the sky channel and the column-map clause), [ADR-0076](0076-light-stability-rules-world-history-per-source-a-fade-per-transition-and-a-ratcheted-pop-counter.md)
> rule 6 (the undrained image is the judged image; the drain is a budget), and ADR-0079 (on
branch `plan-0095` until Plan 0095 merges: the key light's far estimate is the column map's
horizon, which the record refines; it hands the far tiers' reach rule to this ADR)

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

### Context, amended 2026-11-20: the reach rule leaves the far tier nothing to bake

Plan 0095's dev read the queue at rest after a drain (its readings log, "The idle queue after
W1", `b7ad63f7` on branch `plan-0095`): a chunk bakes only when every chunk in its light reach is
mirrored at full detail (ADR-0046's identity rule; `in_reach` at the default light radius 4
reaches offsets up to 5.39 chunks), and the streamer's full radius is 16 chunks (12 in the
Portable profile, `lod.r_full`), so a still camera never fills the reach of a chunk farther than
about 11.2 chunks (about 72 m at 6.4 m a chunk; about 42 m in the Portable profile): under
`ReachMode::Full` nothing beyond that line ever bakes, and the LOD-only ring (16 to 24 chunks)
never does. On the hills at noon the drain "landed" with 0 jobs while 2 964 of 3 561 in-view
chunks waiting were reach-bound; the exact test (`reach_crosses`) marks 11 814 of the full
sphere's chunks as crossing, the HUD's `len + lr > r_full` count 9 924 (Plan 0095 Phase 6 fixes
the count). Rule 2's far band begins where a voxel projects under `lo` (2 px: about 38.5 m at
1080p and the fly-through's focal length, about 26 m at 720p), so nearly every far-band chunk
and no near-band chunk lies past the reach line: **a tier by size on screen whose far tier can
never bake is no tier.** `ReachMode::Partial` (ADR-0046's Outcome of 2026-11-12, a setting off
by default) already bakes such a chunk from the chunks resident, flags it partial in the queue's
maps and re-queues it when a chunk in its reach arrives full; the owner saw it globally at Plan
0067's window ("not much difference really") and kept `Full`. Plan 0095's owner answers of
2026-11-20 route the rule to this ADR and Plan 0096: the far's calm image is the column map's
horizon (ADR-0079) and the bake refines it where a record lands.

## Decision

**The live queue bakes what the image reads, and the time to land what is in view is a budget in
`docs/nfr.md`.** Three rules, in the order they pay, and a fourth (amended 2026-11-20) that lets
the far tiers bake at all:

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
4. **The reach rule by tier** (amended 2026-11-20, owner). The near tier keeps ADR-0046's rule:
   a near-band chunk bakes once every chunk in its light reach is mirrored full. A mid- or
   far-band chunk whose reach crosses the streamer's full radius (`reach_crosses`) bakes **from
   the chunks resident** once nothing in its reach waits to upload (the `Partial` reach of
   ADR-0046's Outcome, now by band instead of global): the full-detail chunks inside the sphere
   and the LOD-only chunks beyond it as the bake's shadow walk finds them (it walks the mirror
   as the frame does, from the job's chunk; the plan confirms a LOD-only chunk casts from its
   mip, not as air), the record flagged partial in the queue's maps as today. **A partial far
   record is refined by rule 2's promotion, not by the reach filling:** the camera that fills a
   far chunk's reach is on its way to the mid band, where the re-bake happens anyway at the
   full reach, so the existing re-queue on arrival is off for mid and far records and one fade
   is saved per approached chunk. The dig's ring keeps the near count and the full reach; a
   ring whose reach crosses bakes partial at the near count and the re-queue on arrival refines
   it. A chunk that cannot hold a record (LOD-only, no bricks) is not listed; its faces read the
   column map (the key light's horizon, ADR-0079; the sky rule, ADR-0070) for good.

The queue therefore reaches a **landed** state (every in-view chunk that can hold a record holds
its tier's record; a LOD-ring face served by the column map has nothing to wait for and counts
as landed), and the budget is the time from the first world frame to that state, per adapter and
profile, beside the time to drain the whole residency.

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
- (Amended 2026-11-20) The baked far reaches the full sphere's edge (16 chunks, about 102 m at
  the defaults) instead of about 72 m, with no change to the residency or its memory; the near
  tier's records are ADR-0046's, bit for bit.

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
- (Amended 2026-11-20) **A partial record's bytes depend on what was resident at the bake.** At
  a still camera that set is the full sphere (the job waits until nothing in reach is pending),
  so the record is the same on every run from the same start; in motion it is whatever had
  arrived, and the camera that stops at 60 m keeps a far record cast partly from mips until it
  comes nearer. The plan's hash test pins the still case; the window judges the far record's
  look, mips and all, against the full-reach bake. The `partial` flag stays in the queue's
  maps, not in the record head.
- (Amended 2026-11-20) The LOD ring never holds a record, so a far face there reads the column
  map for good: the horizon's hard heightfield shadow at the key light and no leaf porosity.
  That is ADR-0079's call and its window's; this ADR only names the line where the record ends.

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

### Alternative E: A wider full residency (added 2026-11-20)
Raise `lod.r_full` so the reach line moves out (20 puts it at about 97 m). The resident set
grows as the cube (about 0.87 GB live at 16 by `docs/nfr.md` section 3; about 1.7 GB at 20, 2.9
at 24), the queue's depth with it, the LOD ring stays unbaked and the Portable profile, at 12,
cannot pay it at all. ADR-0079's Alternative J, rejected there for the same arithmetic.

### Alternative F: `--reach partial` as the global default (added 2026-11-20)
Flip ADR-0046's Outcome: every chunk whose reach crosses bakes partial, the near ones too, as
Plan 0067's arm did. The owner saw it and kept `Full`; it changes the near records' bytes (the
ones ADR-0046's identity test and the dig's ring are built on) for a gain that only the far
needs, and Plan 0095 could not flip it on its branch without owning the queue. Rejected: the
reach is a rule by tier, rule 4, so the near keeps the identity rule and only the bands that
cannot see the difference depart from it.

### Alternative G: Refine a partial far record when its reach fills (added 2026-11-20)
Keep the existing re-queue on arrival for the far tiers. It costs one more job and one more fade
for every far chunk the camera approaches, at about 72 m, where a voxel is 1 px and the
difference between a mip caster and a full one at the reach's rim is under the eye's reach; the
promotion to the mid band at about 38 m re-bakes it at the full reach anyway. Rejected as the
trigger; kept for the dig's ring, whose near count wants the full reach.

## Outcome

**2026-11-23, at Plan 0096's close.** The four rules are built as written. The readings are in
`docs/nfr.md` section 2 ("The bake queue's drain and step") and `docs/plans/logs/0096.md`;
what the implementation falsified or left open:

- **Falsified: "three rules, in the order they pay", for rule 1.** The zero record answered
  236 of the 3 540 buried chunks of the RTX's forest and 1 to 12 of 1 019 on the RENOIR,
  because the opening applies it only to a first bake with a full reach under final columns;
  the others take GPU jobs, some twice (a provisional bake and its catch-up). Applied at every
  opening it would have answered 10.1 to 20.0 % of the jobs opened before the in-view landing
  and 5.1 to 8.9 % of their samples (Plan 0096 Phase 10a). The no-face half of the rule moved
  to the stream workers and holds. Widening the zero record is a plan of its own
  (`docs/backlog.md`, "The record by rule at every opening"); **the sentence "where the column
  map is not final for the footprint the chunk is baked, not ruled" is what that plan's ADR
  supersedes** (what exactness needs is that every change of a top over the footprint is a
  mirror change the re-list reads, which holds).
- **The rule and a GPU bake disagree under a closed cave taller than the job's radius, and the
  rule is the side that is right.** The bake's key ray counts lit where it reaches the radius
  in air (64 of 64 floor faces on Plan 0096 Phase 10c's rig, the rule's record 0). That is
  ADR-0079's bake, not this rule; its fix takes an ADR superseding that clause.
- **Falsified: the in-view budget on the RTX.** The landed state exists and is reached (before
  the plan the forest never drained in 30 000 frames), but the RTX at Floor lands its view in
  36.5 s (forest) and 12.2 s (hills) against 10; the RENOIR at Portable in 27.1 and 16.7 s
  against 30. The RTX's bound is not the slice but the queue's CPU per job opened on the
  render thread (the step 15.5 / 27.9 ms p50 / p99 while baking, about 30 fps in the owner's
  window). ADR-0083 (Plan 0099) takes it; the lines stay as proposed, disclosed as missed.
- **Rule 3's drain value was not picked by eye.** The owner declined ("wherever best
  performance on synthetic") and set a rule instead: frame rate first, an arm whose in-view
  drain is over twice the shortest is out. It picked 1.5 ms at Floor and 6.0 at Portable.
  Floor's 1.5 equals the default before this ADR, so on Floor rule 3 changes nothing while
  in-view work waits and lowers the slice to 1.0 once it has landed. The RENOIR's 6.0 rests
  on the median of three quiet runs, a reading defined after one stalled run (37.6 s) had
  been seen.
- **The far tier's look is accepted.** Floor 4 096 / 1 024 / 256 and Portable 1 024 / 256 /
  256; the owner, shown the profile's tiers against 4 096 in every band: "shadows are fine"
  (window W2, 2026-11-23). The "Negative" bullet's fallback (the mid count as the far tier) is
  not taken.
- **Open: the near tier under Portable.** A dig's ring bakes at the profile's near tier
  (1 024 at Portable), as rule 2 says. At W2 the owner read the fresh hole on the RENOIR as
  bright spots and, landed, as grainy. Not attributed: that picture is marched at 853 x 480
  and stretched to the window, on a binary without Plan 0098's calm picks. If a reading names
  the count, the answer is the Portable row's near count, not a dig exception.
- **Rule 4 as built:** partial far records land and are refined by the band promotion; the
  owner's windows raised nothing on the seam at the sphere or on a partial record's look.
