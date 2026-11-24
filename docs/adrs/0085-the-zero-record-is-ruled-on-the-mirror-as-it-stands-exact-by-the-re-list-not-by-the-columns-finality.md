# ADR-0085: The zero record is ruled on the mirror as it stands: exact by the re-list, not by the columns' finality

> **Status:** proposed (with Plan 0101). At acceptance it supersedes one sentence of rule 1 of
> [ADR-0080](0080-the-live-queue-bakes-what-the-image-reads-a-record-by-rule-a-tier-by-size-on-screen-and-a-drain-slice.md)
> ("Where the column map is not final for the footprint the chunk is baked, not ruled") and
> the clause of its Negative bullet that repeats it ("the rule is off where the map is
> provisional"). The rest of ADR-0080 stands.
> **Date:** 2026-11-24
> **Related plan(s):** [Plan 0101](../plans/0101-the-record-by-rule-at-every-opening-an-audit-on-the-real-scenes-first-then-the-three-exclusions-lifted.md);
> the readings are [Plan 0096](../plans/done/0096-the-bake-queues-drain-tiers-a-drain-slice-the-step-and-the-slow-adapters-light-pass.md)'s
> Phase 10a.
> **Builds on:** ADR-0080 (rule 1, and rule 4's partial reach),
> [ADR-0070](0070-sky-light-is-baked-visibility-coloured-at-read-with-a-long-history-ray-where-stale.md)
> (the column clause),
> [ADR-0084](0084-the-key-lights-visibility-on-a-world-face-is-fixed-per-face-the-columns-horizon-where-lit-a-cover-source-where-shadowed.md)
> (the same clause at the bake's key ray, without which the rule and a job disagree under a
> tall closed cave).

## Context

ADR-0080's rule 1 gives a chunk buried under the column map, with no emitter in reach, a zero
record written on the CPU and no GPU job. It tied the rule's exactness to the column map
being final over the footprint, and the queue as built adds two more conditions: the reach
must be full, and the chunk must hold no block yet. Plan 0096 read what that leaves: the rule
answered 266, 241, 3 and 6 chunks on its four runs, and the jobs opened for chunks it would
have answered were 10.1 to 20.0 % of the jobs before the in-view landing and 5.1 to 8.9 % of
their samples. Some of those chunks bake twice, once provisional and once at the catch-up.

The finality condition was an argument from "a column's tops may still rise or fall while the
world streams in". Read against the code, a top that rises keeps a buried chunk buried, and a
top can fall on an arrival only when a full chunk replaces its LOD version (the mip's 2-voxel
cell counts as solid up to its top voxel). What the queue already has is stronger than
finality: every change of a column sheet is a chunk's upload or drop, each is in the mirror's
change list, and the queue re-reads the rule on every zeroed chunk near a changed column each
step (`relist_zeroed`). The same holds for an emitter arriving in reach, and a job on a
partial reach samples only the emitters resident, which is what the rule reads.

It is a decision because it could reasonably stay as it is: the gain is bounded (about 1.4 s
of 27.3 on the RADV RENOIR's forest, at most about 4 s of 36.5 on the RTX 3080 Laptop, no
budget line changing its standing), and the cost of a mistake is a cave that stays dark.

## Decision

The queue reads rule 1's zero record at every opening, on the column tops and the emitters as
the mirror holds them at that step, whatever the chunk's class, whether its reach is full, and
whether the columns over it are final. A ruled record is final: it is never flagged
provisional or partial and no catch-up or arrival re-queues it. What makes it exact is one
invariant: **every change that can stop the rule holding (a top over the footprint moving, an
emitter entering the reach, a dig) is a mirror change, and the step that reads a mirror change
re-reads the rule on every zeroed chunk within the light radius of its column and re-lists the
ones it no longer answers, with a job and one fade.** A mechanism that changes a column top
or an emitter list without a mirror change breaks the rule and must not be added. The rule is
switched on per exclusion only where an audit on the real scenes (every block a job bakes for
a chunk the rule would have answered, compared with the rule's record) reads 0 blocks that
differ.

## Consequences

### Positive
- The rule answers the chunks it describes: no GPU job, no provisional bake and catch-up, one
  landing fewer for each buried chunk under a column not final.
- A ruled record's bytes do not depend on which chunks were resident when a partial job would
  have cast, nor on the adapter: it is written on the CPU from the listing.
- The correctness argument is one invariant on the mirror's change list, testable case by
  case, in place of a condition (finality) that excluded nearly every chunk while the world
  streams in.

### Negative
- **The rule's reach grows about fifteen times, and a wrong answer is black.** A ruled chunk
  that should hold light shows none until a mirror change near it re-lists it, which may be
  never at a still camera. The audit and the four re-list tests are the defence; whether the
  scenes audited hold a tall closed cave or an emitter underground is not read, so those
  shapes may rest on tests alone.
- **The invariant binds later work.** Anything that edits a column sheet or a chunk's light
  list must go through the mirror's change list. Plan 0099's bake thread reads a shadow of
  the mirror one message behind; it must answer the column tops and the emitters, and a rule
  read on a stale shadow is corrected one message later.
- **The re-list does more.** The zeroed set grows from hundreds to thousands of chunks, each
  read against every step's changed columns; while the world streams in that is CPU on the
  thread the step runs on. It may need an index by column.
- A zero record under a column that was provisional is no longer distinguishable from one
  under final columns in the queue's maps; the reading flag is the only place the split is
  seen.

## Alternatives considered

### Alternative A: Keep the finality condition (ADR-0080 as accepted)
Rule 1 stays almost inert and its Outcome stays the disclosure. It lost because the condition
guards against a case (a top falling unseen) that the change list already covers, at the cost
of 10 to 20 % of the jobs opened before the view lands; and because the audit makes the
wider rule cheap to prove before it is on.

### Alternative B: Wait for the columns instead of baking provisional
Hold a buried chunk back until every column over its reach is final (`--column-wait on`, for
those chunks only), then rule it. It removes the double bake and keeps ADR-0080's sentence. It
lost because it does nothing for the partial reach, which is most of the jobs counted (2 974
of 4 039 on the RTX's forest), and because a chunk held back is a chunk with no record at
all, where the rule can give it its final one at once.

### Alternative C: Skip the dark bricks inside the job's shader
ADR-0080's Alternative C, for these chunks: the job is still listed, opened and submitted, so
it keeps the per-job CPU that bounds the RTX, and the samples it saves are the same 5 to 9 %.
It lost for the reason it lost there.
