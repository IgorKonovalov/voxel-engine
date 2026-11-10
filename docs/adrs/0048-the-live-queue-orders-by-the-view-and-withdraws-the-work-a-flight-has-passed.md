# ADR-0048: The live queue orders by the view, nearest first, and withdraws the work a flight has passed

> **Status:** proposed
> **Date:** 2026-11-10
> **Related plan(s):** [Plan 0060](../plans/0060-the-live-queue-by-the-view-and-withdrawing-passed-work.md)
> (builds it; its close accepts this ADR, and the owner's window in its Phase 4 can overturn it).
> Amends the queue clause of
> [ADR-0046](0046-static-emissive-light-is-baked-per-chunk-when-a-chunk-loads-or-changes.md)'s
> Outcome ("nearest by chunk distance, ties to the chunk ahead") and keeps the rest of it: the
> store, the slice, the coarse pass, the digs and their rings, and the order-independent result.

## Context

ADR-0046's live queue (`bake::BakeQueue` in `crates/vxe-render/src/bake.rs`) opens a chunk by the
key (class, squared chunk distance, view cosine): every chunk with no block (class 0) before every
coarse block to refine (class 1), then the nearest, and the view only breaks ties. The owner's
window of 2026-11-09 (Plan 0059 Phase 13, `docs/plans/logs/0059.md`) named two faults of that key:

- **At rest, far light in the view waits.** At the patch: "in long distance, light is not
  recalculated until I come closer, it would be better to recalculate all in the field of view,
  starting from the nearest chunks". A coarse chunk in view at distance 6 waits for every chunk at
  distance 5 or less, behind the camera included. Plan 0059 Phase 8c read the vein's level all at
  4096 after 157 s; at about 1.2 x 10^6 samples a refinement and the 1.5 ms slice, the queue
  refines a few chunks a second.
- **In a fast flight, the work goes where the camera was.** In the streamed cave: "we are starting
  to calculate places that camera is already passed. We should be able to cancel it when camera
  passed". The mirror holds about 17 000 full chunks around the camera at `r_full` 16 (Plan 0059
  Phase 8c), so class 0 never empties in a flight, and the nearest unbaked chunks are as likely
  behind the camera as ahead. A job, once open, runs to its end even when its chunk is behind.

What the key cannot change: the queue's throughput (the slice), the result of a chunk's bake (its
seeds are keyed by chunk, side and sample index, so a block is bit-identical whatever the order,
ADR-0046), and the near shell, which must stay lit in every direction so a turn shows no unlit
face. How much of the flight's fault is the order and how much is work in flight on passed chunks
has no reading yet; Plan 0060's first phase reads it before the withdrawal is built.

## Decision

The queue sorts its candidates by **(tier, class, squared distance, view cosine)**, and **withdraws
an open or pending job whose chunk the camera has passed**.

- **Tiers.** Tier 0 is the near shell: the chunks within `near` (Chebyshev) of the camera's chunk,
  in every direction. Tier 1 is the view: a chunk whose box meets the camera's frustum widened by a
  margin (a guess of 15 degrees on each side, read in Plan 0060). Tier 2 is the rest. Within a
  tier, a chunk with no block comes before a coarse one, and then the nearest. Digs (rings 1 and
  2) stay ahead of every tier, and ring 3 stays with the refinements of its tier.
- **Withdrawal.** At an opening, a job that is not a dig and not ring 3, whose chunk is outside the
  near shell and outside a keep cone (the view's margin doubled, so a chunk at the edge does not
  flip each frame), is withdrawn when a tier 0 or tier 1 candidate is ready to take its place. A
  pending job is dropped; an open job leaves the loop with its partial sums discarded and its
  place released. The chunk keeps whatever block it had (none, or coarse) and goes back to the
  candidates, in tier 2.
- **The result does not move.** A withdrawn chunk re-bakes from the start, so its block is still
  bit-identical to a one-shot bake on the same adapter; only the time it lands changes.

## Consequences

### Positive
- The light the owner looks at is baked first, at rest and in flight; the near shell keeps the
  surroundings lit for a turn.
- The slice stops spending on chunks behind a flight, without changing the slice, the store or
  the counts.
- The rule is a sort key and a predicate, both testable on a six-chunk world without a window.

### Negative
- **A turn costs.** What was behind the camera is baked last: turning around shows coarse light
  (or, beyond the near shell, no value) until the queue catches up, then a 16-frame fade. The near
  shell bounds it to faces beyond `near` chunks.
- **Withdrawn work is lost.** A refinement withdrawn at 90 % re-draws all its samples later. The
  keep cone's margin and the "a better candidate is ready" condition bound the churn; Plan 0060
  reads the samples withdrawn.
- **The queue needs the frustum.** `QueueView` gains the camera's basis and tangents; a VR view
  needs the union of its eyes' frusta (the roadmap's asymmetric-view rule).
- **ADR-0046's consequence "a place is lit before it is seen, whatever the view" weakens** to
  "within the near shell, or once the view's work is done".

## Alternatives considered

### Alternative A: Distance first, view as a tie (ADR-0046 as built)
Kept nothing the owner asked for: both complaints in Plan 0059 Phase 13 are this key.

### Alternative B: One continuous score, distance scaled by the angle off the view
A key like `d^2 x (1 + k (1 - cos))` orders smoothly and needs no tiers. It lost because no `k`
guarantees "all of the view before what is behind", the owner's words, and a tier is a claim a test
can assert; a far chunk in view and a near chunk behind trade places at a `k` nobody can see.

### Alternative C: Order by a predicted eye (position plus velocity times a lead)
It would bake ahead of a fast flight before the chunks enter the view. It lost for now: the queue
would read the controller's velocity, the lead is a second knob, and at rest (the patch complaint)
it equals the plain view order. A later plan can move the frustum's apex along the velocity
without changing this ADR's tiers.

### Alternative D: Demote only, never withdraw
Re-sorting alone stops new work behind the camera but lets open jobs run to their end. It is
cheaper and loses no samples; it lost on the owner's words ("cancel it when camera passed") and
because a refinement at 4096 (about 1.2 x 10^6 samples) holds tens of frames of slice. If Plan 0060's first reading shows the
work in flight on passed chunks is a small share, the plan falls back to this alternative and this
ADR records it in an Outcome.
