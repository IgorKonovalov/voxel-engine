# ADR-0005: Character collision as a vertical cylinder swept by integer conservative advancement

> **Status:** accepted (2026-10-17, at Plan 0004's close; see Outcome)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0004](../plans/done/0004-cylinder-sweeps-character-controller-and-coarse-grid.md)

## Context

The first consumer's P0 list needs a character controller for the player and for monsters
against voxel terrain, with steps, slopes and climbing out of dug holes, plus shape sweeps for
tool reach and line of sight (`project-context.md`, Queries). The consumer's server owns AI, so
the controller runs server-side for monsters, and the roadmap's M5 split replays a server tick
from seed plus op log. A controller whose result depends on the platform's float rounding would
break that replay, so the sweep and the controller are held to the same rule as edits: integer
or fixed-point math, bit-identical on Linux and Windows.

The terrain is axis-aligned unit cubes in a brick map (ADR-0002), most of them inside `Uniform`
bricks that answer "solid or air" without a voxel read. There are no slopes in the geometry: a
slope is a staircase of voxel steps, and a controller that handles steps handles slopes.

The owner's choice (2026-10-16 interview) was a capsule with step, slope and ledge climb over an
axis-aligned box or sweeps only. Working the step case through showed that a hemispherical foot
cannot stand on a step's edge until its axis is past the edge: the down-sweep after a step-up
lands the hemisphere on the edge with a diagonal normal and the step is rejected. Every shipped
controller works around this with a flat-footed probe. This ADR records the body shape that
gives the rounded sides the capsule was chosen for and a flat foot that stands on edges.

## Decision

The body is a **vertical cylinder**: a foot point in fixed-point voxel units, a radius and a
height, flat at both ends, round in plan. Collision is a distance query: the squared distance
between the cylinder and an axis-aligned voxel box is the sum of a horizontal and a vertical
term, `max(0, d_xy - r)² + d_z²`, where `d_xy` is the distance from the axis to the box's
rectangle in plan (clamp, then integer square root) and `d_z` the gap between the two
z-intervals. The sum is exact for product sets under the Euclidean norm; the floor of the square
root makes the distance a lower bound, which is the safe side.

A **sweep** moves the body along a delta by conservative advancement: compute the minimum
distance to every solid voxel in the swept region (whole `Uniform` bricks as one box, voxels of
`Full` bricks one by one), advance by that gap along the delta with every component rounded
toward zero, repeat until the gap is zero, the delta is spent, or the advance rounds to zero on
every component. The body never penetrates; axis-aligned motion stops exactly in contact. The
contact normal is an integer direction built from the axis-to-box vector in plan and the sign
of the vertical separation; a contact is a floor or ceiling when the box is vertically separated
and its plan rectangle lies within the radius of the axis, otherwise a wall. There is no slope
parameter: a surface is walkable when it is a floor contact, and slopes are climbed as steps.

The **controller** composes sweeps in a fixed order per tick: horizontal move with step-up and
slide, then vertical move under gravity; a ledge climb is a multi-tick state entered when a
wall blocks and a standable position exists within the climb height. All parameters are fixed
units per tick; the caller owns the tick rate. The code lives in a new crate, `vxe-sim`, which
depends on `vxe-world` only. Raycasts stay in `vxe-world`, which owns all primitive queries.

## Consequences

### Positive

- Deterministic by construction: integers, a fixed iteration order, no clock. A course fixture
  hashes to the same constant on both CI arms, and the M5 replay can include characters.
- Exact and cheap: one distance formula, no quadratic solves, no epsilon tuning. A
  sweep costs a few iterations over the bricks the body spans; uniform bricks are one test.
- Flat feet stand on edges, so step-up works at the first contact, and the body does not slide
  off ledges while its axis is still over them.
- Round sides give capsule-like sliding along walls and around corners, which is what the owner
  asked the capsule for.

### Negative

- **A flat foot overhangs.** The body stays supported while any floor voxel lies within the
  radius of its axis, so it can stand with most of its base over a drop. Games ship with this;
  it is visible if the camera looks down.
- **No rounded top or bottom.** Ducking under a sloped ceiling or sliding off a sharp overhang
  feels blockier than with a capsule. The first consumer's caves have neither.
- **Conservative advancement can creep.** Oblique approach to a corner shrinks the gap
  geometrically; the loop stops when the advance rounds to zero, which leaves a gap below
  2/256 of a voxel. The iteration cap bounds the cost, not the gap.
- **Thin walls at speed.** The swept region is the body's box extended by the delta, so a
  one-voxel wall is never tunnelled through; but a large delta (a long teleport) scans a large
  region. The controller clamps velocities; the raw sweep does not.

## Alternatives considered

### Alternative A: Axis-aligned box swept one axis at a time

The classic voxel-game controller: resolve x, then y, then z against the voxels the box
overlaps. Exact, trivial and deterministic. Rejected because the box catches on every corner
when moving diagonally and slides along walls only on the axes, and because stepping needs the
same flat-foot logic anyway; the cylinder costs one square root more per contact and gives the
feel the owner asked for.

### Alternative B: A float capsule through a physics library (`rapier` or `parry`)

Ready-made capsule sweeps, contact manifolds and a controller. Rejected because the voxel world
would have to become collider shapes (a height field or a trimesh per chunk, rebuilt on every
edit, when the brick map already answers the query directly), because cross-platform
determinism needs the library's enhanced-determinism build and still excludes transcendental
calls we do not control, and because it is a large dependency for one primitive.

### Alternative C: Point-sampled collision

Test a fixed set of sample points on the body's surface against the voxel grid and push out
along the worst sample. Cheap and common in hobby voxel engines. Rejected because it tunnels
through thin walls at speed without substeps, because the contact normal is guessed rather than
derived, and because its behaviour depends on the sample layout, which is exactly the kind of
tuning a deterministic server tick should not carry.

## Outcome

Accepted 2026-10-17 at Plan 0004's close. The body shape, the distance formula, the integer
determinism, the crate split and the rejected alternatives stand as written. The walk-check fix
pass (the owner's windowed walk froze the body on open floor and beside walls) falsified three
sentences of the Decision; the shipped behaviour, which these paragraphs record, is:

- **The sweep.** `g` is taken only over the boxes the remaining delta **closes on** (the gap's
  rate along the delta is negative); a box the delta runs along or away from is left out, so a
  box under one unit beside the body no longer stops a fall or a move away from it. An advance
  that would overlap a box (checked on an iteration that left a box out) is not taken and that
  box is the contact. The step-length rule: the whole remainder is taken only when
  `g² >= |remaining|²`, else `g` along it scaled by the root of `|remaining|²` rounded **up**,
  every component rounded toward zero, so no advance is longer than `g`. With the root rounded
  down, a remainder up to one unit longer than `g` was taken whole and ended inside the box.
  At the iteration cap the contact is the nearest box the remaining delta closes on.
- **"The body never penetrates"** now rests on the overlap guard as well as on `g`: with boxes
  left out, `g` no longer bounds the gap to every box in the region. A 115 248-sweep test
  (`a_short_move_never_ends_inside_a_box`) and 4000-walk random runs on the seed-1 level
  assert no overlap after any sweep or tick.
- **The ledge climb** is entered when a wall blocks, and also, with `climb` held, when the
  wish runs into a wall within 60 degrees of head-on (`4 (w . n)² >= |w|² |n|²`) even if the
  slide moved the body: a wish at an angle to a ledge face slides along it and was never
  blocked. The probe's up sweep may stop at a ceiling once it has risen more than the step
  height, and the down sweep drops by what it rose. `climb_available` can be reported on a
  tick that is not blocked.
- The wall slide rounds each component of the projected wish toward the side the normal points
  to, not toward zero; rounding toward zero closed on the voxel edge being slid along.

One negative consequence is added:

- **No depenetration.** A body that starts a sweep overlapping solid (an edit fills the body)
  is not moved: `sweep` reports `started_overlapping` and the controller pushes nothing out.
  The fly-through refuses to place a body that overlaps; an edit can still embed one. Known
  gap, a followup of Plan 0004.

The creep consequence still holds: an oblique approach to a corner stops when the advance
rounds to zero, and the iteration cap bounds the cost. The worst case (a 64-iteration grazing
corner) is not benched. The measured costs are in `docs/nfr.md` (collision section): the F2
wall sweep reads about five times the 5 µs the plan proposed.
