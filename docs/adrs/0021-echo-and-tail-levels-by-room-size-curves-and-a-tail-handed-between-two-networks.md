# ADR-0021: Echo and tail levels follow listening-fitted curves of the room's mean free path, and a room change hands the tail to a second network

> **Status:** proposed
> **Date:** 2026-10-21
> **Related plan(s):** [Plan 0020](../plans/0020-cavern-reverb-directional-echoes-and-gliding-taps.md) Phases 7 to 14
> **Supersedes in part:** ADR-0015's level clauses (the tail's energy as Kuttruff's room
> constant against a unit direct impulse, the echoes at `keep / path` alone) and its
> room-change clause (one network whose lines glide). ADR-0015's structure stands: one shared
> feedback delay network, the fan at the listener, the skin-depth rule, the direction buses.

## Context

ADR-0015 set both levels from physics: each early tap at its image source's `keep / path`, and
the tail's energy at `16 pi (1 - alpha) / (S alpha)` against a unit-energy direct impulse. A
change of room glided the tail's line lengths. The owner's listening (Plan 0020 Phase 7,
headset) failed both. With the tail on, a source's direction was lost, because the critical
distance in the test rooms is 0.5 to 5 m (1.6 m in the 12 m cube). The echoes were too quiet
against the tail. Flying back into a chamber from outside the world gave a loud burst.

Phase 8 measured the room change headlessly. A rock room between two chamber rooms overshot the
chamber's steady tail by +19.11 dB with gliding lines and +2.62 dB with a handover between two
networks, against a +1.96 dB switch-on. Under a room that moved every 0.33 s, the energy left
within 3 Hz of a 440 Hz hum was 0.0864 with gliding lines and 0.9221 with the handover: a
recirculating signal read through a moving line is repitched on every pass.

Phase 12 had the owner pick the levels by ear, one setting per cube, from its centre. The echo
gain wanted fell as the room grew: +30 dB at mean free path 2.67 m, +15 at 8.0, +12 at 24.0,
+6 at 69.0. Only the 4 m room wanted the tail lowered (-15 dB). No single setting served all
rooms. Phase 14 confirmed the fitted curve by ear at its defaults: the 4 m room and a walk
through the seed-1 cave held with the trims at 0, and the 12 m room held but sounded better
with the echo trim at -3 dB.

## Decision

**The physical levels stay as the base, and a curve of the room's size scales each.** A
`RoomCurve` holds up to 8 knots `(mean free path m, dB)`. It is piecewise linear in
`log(mfp)` between knots, holds its end values outside them, and reads its last knot (the
largest room) for a mean free path that is not finite or not above 0. `MixSettings.echo_by_room`
multiplies the direction buses' input. `ReverbSettings.level_by_room` multiplies the tail's
input gain where `level` enters it, at the mean free path of the room that network takes. The
input is the mean free path the mixer already receives in `Command::SetRoom`. The mixer
evaluates the curve on `SetRoom` and `SetMix`, never per block, and the echoes move to it
through the 50 ms one-pole. `echo_gain` and `ReverbSettings.level` remain as trims on top. The
engine's default knots are the Phase 12 picks: echoes `(2.67, +30)`, `(8.0, +15)`, `(24.0, +12)`,
`(69.0, +6)`, and the tail `(2.67, -15)`, `(8.0, 0)`. A consumer passes its own curves, and
`MixSettings::flat()` restores the physical levels.

**A room change hands the tail to the other of two networks.** When the new mean free path
differs from the fed network's by more than `TAIL_HANDOVER_RATIO` (1.15) either way, the idle
network is cleared and takes the new lengths at once. The input crossfades to it over 50 ms with
equal power, and the old network rings out at its own decay with no input. A room within the
ratio changes the fed network's decay and energy in place, and its lengths do not move. No line
that holds signal changes length. Gliding lines remain as `TailLines::Glide`, an option for
comparison.

**A room the fan cannot measure gets no tail.** A fan with no hit or no surface reports energy
0, and a fan due while the listener's voxel is solid is discarded before any ray.

## Consequences

### Positive

- The levels are the ones the owner heard as right, in every cube, and a walk through the cave's
  chambers and tunnels showed no level jump at room changes (Phase 14).
- No per-block cost. Under ADR-0020's aligned protocol, Phase 13 read within 0.5 percent of its
  parent in all four mixer cases.
- The handover never repitches a stored tail, and the re-entry overshoot drops from +19 dB to
  within 0.7 dB of the tail's own switch-on.
- Physics still sets everything the curve does not: decay time, onset spacing, tap delays and
  directions, and the per-material keep factor.

### Negative

- **The defaults are one listener's picks, made with one headset in cube rooms**, at a 3 dB step
  resolution. They vary between sessions: the 12 m knot was picked at +15 dB and confirmed a day
  later at about +12 dB. Rooms between the knots, and tunnels, are interpolations that only the
  Phase 14 cave walk has checked.
- **The levels are no longer physical.** In the 4 m cube at the defaults, the early energy is
  about 34 dB over the direct path's. A headless figure such as the balance table reads that as
  intended, not as an error, and a consumer that wants physical levels must ask for `flat()`.
- **Louder echoes make the wobble while moving louder** (pinned in Plan 0020 as a known issue).
  The owner heard it as "almost non existent" at the defaults, but it is a candidate cause
  whenever the echo curve is raised.
- **Two networks double the tail's memory**, to 3 616 768 B with Phase 11b's 28 000-frame cap.
  A block that clears a network costs about 0.1 ms more on the dev box.
- **The 4 m knot sat at the top of the trim range** when it was picked, so the true preference
  there may be higher. The curve holds +30 dB below 2.67 m.

## Alternatives considered

### Alternative A: Keep the physical levels

Kuttruff's room constant for the tail and `keep / path` for the taps. This needs no tuning, and
"bigger cavern, bigger echo" follows from physics. It lost to the ear: with the tail on, a
source's direction was lost in every test room, and the echoes were buried (Phase 7, checks 2,
3 and 5).

### Alternative B: One fixed echo gain and tail level

This is one number per part, simple to reason about. It lost because the owner's picks span
24 dB of echo gain across the cubes, and no single setting kept the knock's direction and the
large room's late return together (Phase 12).

### Alternative C: A least-squares line in `log10(mfp)`

This is smooth and has two parameters. It lost because over all four picks it is 4.5 dB under
the 12 m pick, and over the three picks below the range's top it reads +20 dB at 4 m, 10 dB
under what the owner heard. Knots reproduce every pick.

### Alternative D: Key the curve on `T60` or the room's volume

`T60` is clamped at 8 s from the 36 m cube up, and it moves with absorption, which the picks did
not vary. The volume is not in `RoomTargets`, and it ranks the cubes the same as the mean free
path. Both lost to a key the mixer already receives.

### Alternative E: Gliding tail lines (ADR-0015 as written)

This uses one network and half the memory. It lost on Phase 8's measurements: a +19 dB overshoot
on re-entry and a stored hum repitched on every pass.

## Outcome

_(none yet)_
