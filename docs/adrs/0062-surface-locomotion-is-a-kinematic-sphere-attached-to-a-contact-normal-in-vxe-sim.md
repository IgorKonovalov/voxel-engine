# ADR-0062: Surface locomotion is a kinematic sphere attached to a contact normal in `vxe-sim`; ground locomotion keeps the cylinder controller

> **Status:** accepted (2026-11-18, at Plan 0079's close; see Outcome)
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0079](../plans/done/0079-entities-aware-of-floor-and-walls-a-surface-mover-the-smoothed-normal-and-a-wandering-demo.md)

## Context

The owner wants entities that know where the floor and the walls are, "so for example we can
have walkers walking and spiders crawling" (2026-11-16). The first consumer's creatures move
through dug tunnels on the server: ADR-0005's context names "a character controller for the
player and for monsters", and the roadmap's M5 replays a server tick from seed plus op log, so
whatever moves an entity must be integer arithmetic, bit-identical on Linux and Windows, and
must read the world as it is after every edit, because the floor under a creature can be dug
away at any tick.

What the tree gives (at `plan-0063` c5c655ad): `vxe-sim` sweeps a **vertical cylinder** by
integer conservative advancement against the brick map and drives a controller with step,
slide, climb, fall and depenetration (ADR-0005, Plan 0004, Plan 0016); a contact carries an
integer normal and reads as floor, wall or ceiling by its orientation to the cylinder's axis.
The controller's course hashes to a committed constant on both CI arms and a tick reads 1.8 to
2.6 us (`docs/nfr.md` section 8). `vxe-entity` poses a model with a `Fixed3` root and a Q14
quaternion (ADR-0050) and samples clips by tick (ADR-0051). `vxe-world` answers rays, point
materials and box counts in integers, and keeps a per-brick walkable map (0.8 m cells).

The cylinder cannot crawl. Its axis is the world's vertical, gravity is a signed vertical speed,
and a contact's kind is read against that axis: a body on a wall or a ceiling has no meaning
in it. A crawler needs a body whose "up" is the surface it stands on, a rule for carrying that
up across a convex edge (over a ledge onto the wall below) and a concave one (into the corner
where the floor meets a wall), and a fall when the surface goes.

A second force is the look. Voxel faces give axis normals: a crawler aligned to them on a
stepped slope flips its body ninety degrees at every voxel step. Plan 0054's field look
(ADR-0041, ADR-0042) smooths the shading normal from the 27-count field, but that is a GPU
path and not a thing a server tick can read. The body's alignment feeds the pose, and the pose
feeds the server's hit test (ADR-0050), so the smoothed normal must be an integer query too.

ADR-0008 bars a rigid-body engine from the authoritative world and keeps rigid bodies to a
cosmetic client layer that nothing deterministic reads. A mover that is kinematic (it is moved,
it does not integrate forces beyond a fall) is on the right side of that line.

## Decision

`vxe-sim` gains a **surface mover** beside the character controller: a **sphere** (a centre in
fixed point and a radius) swept by the same conservative advancement against the same solid
boxes, with the sphere-to-box gap `max(0, d - r)` where `d` is the floor of the root of the
summed per-axis squared gaps. Its state is the centre, an integer **contact normal** `n` and an
integer **forward** `f` in the tangent plane, and whether it is attached. A tick does three
things in a fixed order: it turns `f` about `n` by the input's heading step and moves along
`f`'s projection onto the tangent plane by the input's speed; a contact that stops the move
makes that contact's normal the new `n` and carries `f` across by the rotation that takes the
old normal to the new one (the **concave** transition); it then sweeps along `-n` by an attach
reach and snaps to the contact, taking its normal, which rounds the body over a **convex**
edge without a special case, because a sphere past a ledge finds the ledge's side within the
reach. When the reach finds nothing the body **detaches** and falls under world gravity as a
sphere until any contact re-attaches it, reporting the landing speed. Every quantity is an
integer; the same course gives the same states on every platform, and the course hashes to a
committed constant beside the controller's.

**The ground walker keeps the cylinder controller.** It already steps, slides, climbs and
falls, its course is pinned, and a biped on a floor has a vertical axis. The two movers share
a heading convention (a turn step about the body's up and a speed) so one steering behaviour
drives both.

**The smoothed normal is a world query.** `vxe-world` gains the **solid-count gradient** over
an odd cube of voxels around a point: per axis, the solid voxels on the minus side less those
on the plus side, read through the bricks' masks. It points from solid toward air and is the
surface's smoothed normal at the body's scale (the cube's half-width is the body's radius). On a
flat face it is the face normal; on a one-voxel staircase it is the slope. The mover reports it
beside the contact normal, and the consumer aligns the body to it. The body's alignment comes
from an integer **frame to quaternion** in `vxe-entity` (a forward and an up, the forward made
orthogonal to the up, normalised with the integer square root), so no sine table and no float
sits between the sim state and the pose.

Steering is the consumer's. The engine answers probes ahead (a wall within reach, the floor
ahead or a drop) through `World::raycast`, and the consumer, or the fly-through's demo, decides
where to go. Pathfinding is not part of this decision.

## Consequences

### Positive
- One sweep loop, two bodies: the sphere adds a distance formula and a contact rule, not a new
  collision system. Uniform bricks stay one box each.
- The crawler rounds convex edges and climbs concave ones from the same two sweeps a tick,
  with no edge-case probes; a dug-away surface is noticed the next tick with no notification
  path, because the attach sweep reads the world as it is.
- Deterministic by construction, so the consumer's server can move its creatures and replay
  them (M5), and a crawler's pose is the same on every client.
- The smoothed normal is one integer query at the body's scale, shared by any consumer that
  wants a surface frame (a decal, a footstep sound's direction, a camera on a wall).

### Negative
- **A sphere has no height.** A tall body on a floor is modelled by its clearance radius
  alone: a spider's legs clip into a wall on a stepped surface, and a crawler cannot be made
  to duck. The parts are not collided (ADR-0050's scope), so this is a look, not a hit-test
  error.
- **The cylinder and the sphere disagree in kind.** A consumer that wants one creature to both
  walk upright and climb walls switches movers at a moment of its choosing; the plan hands it
  the state to do so (the sphere's centre and normal, the cylinder's foot) but no automatic
  hand-over.
- **Rounding a convex edge takes several ticks at a tilted normal**, during which the contact
  normal is diagonal and the forward is transported through rounded integer frames. The
  forward is renormalised each tick so it does not shrink, but a long crawl accumulates a
  heading drift of a fraction of a degree per edge. The course test pins the behaviour; it
  does not promise a straight line.
- **The gradient's cube is the body's radius**, so a crawler of radius 6 voxels reads about
  2200 voxels a tick (13 cubed), through at most 27 bricks' masks. Cheap by the arithmetic;
  the plan measures it before anyone argues.
- **Gravity while detached is the world's `-z`.** A crawler that lets go of a ceiling falls to
  the floor; there is no "gravity toward the nearest surface". That is the first consumer's
  caves and the owner's words.

## Alternatives considered

### Alternative A: one generalised controller with a gravity vector
Make the cylinder controller's "up" a parameter and run walkers and crawlers through it. It
lost because every rule in the controller (step-up, the slide's horizontal projection, the
ledge climb, the floor probe, depenetration's candidate order) is written against the vertical
axis and pinned by a course hash; rewriting it as a frame-relative body is a rewrite of Plan
0004 with the walker as collateral. A sphere beside it costs a few hundred lines and leaves
the controller as it is.

### Alternative B: locomotion in the consumer or the fly-through, in float
Let the app move entities with `glam` and the world ray query, as the fly-through's scripted
circle does today. It lost because the first consumer's creatures move on its server and are
replayed from a log; a float path is not bit-identical across platforms, and `vxe-world` and
`vxe-sim` exist to keep that math integer. The demo's steering stays in the app; the motion
does not.

### Alternative C: a rigid-body library for the creatures
Simulate each creature as a body with friction and a sticking force. It lost before it was
weighed: ADR-0008 keeps rigid bodies out of the authoritative world, and a sticking force is
not how a spider crawls in any case.

### Alternative D: move on the walkable map's cells
Drive entities over the per-brick walkable map (M3) with no sweep. It lost because the cells
are 0.8 m and know only floors: a crawler on a wall or a ceiling has no cell, and a body of
0.4 m would move in 0.8 m steps. The map remains the right data for pathfinding, which is a
later plan.

## Outcome

Added 2026-11-18 at Plan 0079's close. The decision held: `vxe-sim`'s `SurfaceMover` is the
sphere described, swept by the same conservative advancement over the same collected boxes
(`sweep_sphere` beside the cylinder's sweep, the cylinder's course hash unchanged), the
gradient is `World::solid_gradient` read through the brick masks, and the frame is
`Quat::from_frame` in `vxe-entity`; the surface course hashes to `0x0a07_c86c_51f9_8d3c` and
the fly-through's room course (sixteen bodies, 1500 ticks) to `0xe57c_02c6_8ac4_b2d8`, both
asserted on CI's Linux and Windows arms. Three details differ from the body above, each stated
in the code's doc and pinned by the course hash: the forward is carried by the shortest arc at
a **convex** transition too, not only at the concave one (the body says the concave case; on
landing it is projected onto the new surface only); a move that meets a contact whose normal is
the body's own glides on, swept once more lifted a little along the normal, rather than
stopping; and the half turn for antiparallel directions is about the lowest-index axis made
perpendicular to the direction, not that axis itself. `vxe-sim` keeps `vxe-world` as its only
dependency: the arc is a crate-local copy of `vxe-entity`'s, held equal by one committed vector
of 64 direction pairs hashed in both crates' tests. The measured costs (`docs/nfr.md` section 8,
loaded, indicative): one attached tick 0.43 µs against the 20 µs proposal, `solid_gradient` at
`half` 6 0.92 µs against 5 µs; 64 spiders' movers cost the fly-through 1.4 ms a tick in a room
and 2.8 ms on the showcase, which is +3.07 ms of frame CPU against the plan's +1.5 ms guess,
a profile owed in `docs/backlog.md`. The negative consequence "a sphere has no height" showed
as planned (a radius-512 sphere on the one-voxel staircase rests on the step corners, so the
staircase test reads a radius-128 body); the convex-edge dwell showed as a spider lingering up to
220 ticks at a pillar's vertical edge, within the Negative list's description.
