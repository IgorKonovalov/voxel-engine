# ADR-0005: Character collision as a vertical cylinder swept by integer conservative advancement

> **Status:** proposed
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0004](../plans/0004-cylinder-sweeps-character-controller-and-coarse-grid.md)

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

_(Added at acceptance if Plan 0004 falsifies something above.)_
