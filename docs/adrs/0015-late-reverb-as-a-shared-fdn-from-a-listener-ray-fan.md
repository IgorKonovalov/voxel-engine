# ADR-0015: Late reverb is one listener-shared feedback delay network driven by a ray fan from the listener; early echoes render from their wall's direction

> **Status:** proposed
> **Date:** 2026-10-17
> **Related plan(s):** [Plan 0020](../plans/0020-cavern-reverb-directional-echoes-and-gliding-taps.md)

## Context

Plan 0006 shipped positional audio with occlusion and sixteen early-reflection taps per voice
and no late tail. The owner's verdict on `main` (7e8b3d2) was that the echoes need work: a
bigger cavern should sound bigger, and nothing today grows with the room. The consumer's P1
requirement is "early reflections and reverb from ray probes" (`project-context.md`, Audio).

The world is dug every frame, so nothing about a room can be baked ahead of time. The probes
run on the game thread through the integer DDA at about 40 ns per voxel step, and the owner
accepted about 1.0 ms per `Acoustics::update` as the budget (D2b, 2026-10-17). The mixer runs
on the device callback at 0.36 ms per 5.33 ms block with 32 voices (quiet-box reading, dev
box), with 1.0 ms on the baseline CPU as the proposal, and allocates nothing after
construction. `vxe-audio` is not deterministic by contract (`nfr.md` section 6).

The statistical room model needs two numbers: the mean free path `4V / S` and the mean
absorption of the walls (Sabine 1922, Eyring 1930; `docs/references/room-acoustics.md`). For
the region the listener can see, both are exact integrals over direction of the first-hit
distance `r` and the hit face's angle: `V = (1/3) * integral of r^3`, `S = integral of
r^2 / cos(theta)`. A fan of 128 directions evaluates them within 4 percent on a 12 m cube from
2 m off its centre (quadrature checked 2026-10-17). The material table's acoustic triple
`[A, T, R]` defines `A` as absorption per metre of path inside the material (Plan 0006), the
only absorption figure a row carries.

## Decision

**One shared tail.** The mixer owns one feedback delay network of sixteen delay lines with a
Householder feedback matrix and a first-order low-pass per line (Jot and Chaigne 1991; Smith,
"FDN Reverberation"). Every voice sends its own-gained, occlusion-scaled mono signal into it;
the network's two decorrelated outputs add to the stereo output without an HRIR, since a
diffuse field has no direction. The delay lengths follow the mean free path (spread from half
to one and a half of it, capped at 100 ms) and glide to a new target at a bounded rate; the
per-line gains are recomputed every block from the current lengths and the decay time, so a
change of room is a glide, never a swap.

**Parameters from a fan at the listener.** `Acoustics` traces a fan of 128 fixed directions
from the listener, sliced at sixteen rays per `update` call, and restarts it when the listener
has moved a metre or half a second of audio has passed. From the first hits it forms the
visible volume and surface, the area-weighted absorption, Eyring's decay time
`T60 = 0.161 V / (-S ln(1 - alpha))`, and the tail's energy relative to a unit-energy direct
impulse, `16 pi (1 - alpha) / (S alpha)` (Kuttruff's room constant). A ray that reaches the
range limit counts as a fully absorbing surface there, so open air gets a tail more than 40 dB
down rather than a long one.

**A face's absorption is the per-metre value over a fixed skin.** A reflection keeps the
amplitude `(1 - A) ^ (2 * REFLECTION_SKIN_METRES / 1 m)` with `REFLECTION_SKIN_METRES = 0.1`:
the wave enters one skin's depth of the material and returns. The same keep factor scales the
early taps and gives the tail's energy absorption `alpha = 1 - keep^2`. With the fly-through's
cave row (`A = 0.3`) a face keeps 0.931 of the amplitude and absorbs 13 percent of the energy,
which is rock; with Plan 0006's `keep = 1 - A` the same row absorbed 51 percent, which is a
padded studio.

**Early echoes from their walls.** Each early tap sums into one of sixteen shared buses, one
per reflection direction, and each bus runs through the HRIR nearest to its direction in the
listener's frame, with the same one-block crossfade a voice uses when its direction changes.
The bus cost is fixed at 1.05 million multiply-adds per block whatever the voice count. Taps
are keyed by direction, and each one's delay glides to its new target through a fractionally
interpolated read while its gain moves through the 50 ms smoother; this replaces the one-block
tap swap that clicked while the listener walked.

## Consequences

### Positive

- "Bigger cavern, bigger echo" follows from the physics with no per-level tuning: the decay
  time and the onset spacing both scale with the mean free path, and the level with the room
  constant. A dug-out chamber grows its tail within a second of the fan completing.
- No dependency, no FFT, no bake. About 30 thousand flops per block for the tail and 324 KB
  for its lines; the fan adds at most 16 rays to an `update` call.
- The early echoes and the tail share one keep factor per material, so a change to a row moves
  both consistently.
- The tail, the buses and the gliding taps are all headless-testable: Schroeder integration
  reads the decay time, the pack's own interaural levels check the bus directions, and the
  block-to-block level step bounds the clicks.

### Negative

- **The room is what the listener sees.** A source in a side chamber gets the listener's tail,
  not its own, and a tunnel that opens into a hall reads as the hall along that line of sight
  (the `r^3` weight favours long views). Steam Audio's source-space reverb and Project
  Acoustics' per-pair parameters do better here, at costs this engine cannot pay.
- **One broadband absorption per material.** Frequency-dependent decay comes from a fixed
  high-frequency ratio in the reverb settings, not from the table. Four-band textures (Wwise
  Reflect) would need a wider row.
- **The skin-depth rule is a convention.** It makes one number serve two physical quantities;
  a material whose face and bulk disagree (thin metal sheet over air) cannot be authored.
- **Gliding taps carry a Doppler glide** of up to 1.7 percent in pitch on the echoes while the
  listener walks at 3 m/s, while the direct path has none; the owner's ear decides whether it
  reads as motion or as an artefact.
- **Sixteen buses add FIR work while turning**, when several buses change direction in one
  block; the mixer's turning figure may approach the 1.0 ms proposal on the dev box.

## Alternatives considered

### Alternative A: Convolution with a synthesized or traced impulse response

Build an impulse response per room change (noise shaped by the estimated per-band decay, or
traced with thousands of bounced rays as Steam Audio's real-time mode does: 4096 rays and 16
bounces in its example) and convolve the shared send by partitioned FFT. A 4 s tail is 192
thousand taps; the response must be rebuilt and crossfaded on every room change, doubling the
work during the fade, and tracing it through our DDA costs about 260 ms per update. It lost on
cost and a dependency for a tail the ear cannot tell from a feedback network's in a cave.

### Alternative B: A baked parametric field (Project Acoustics)

Simulate the wave equation per level offline and decode loudness, decay and direction per
source-listener pair at runtime (Raghuvanshi and Snyder 2014). The decode is cheap and the
result is the best available, but the bake assumes static geometry; its 2021 extension handles
doors only. The engine's premise is that every voxel can go, so it lost outright.

### Alternative C: More image-source taps and no tail

Second-order bounces and a higher tap limit per voice. Cheapest to build, but echo density
never grows, the 200 ms delay line caps the reach at 34 m of excess path, and nothing scales
with the room. It lost on the requirement itself.

### Alternative D: Room volume from a flood fill on the coarse grid

Count air cells reachable from the listener on the 0.8 m coarse grid up to a cap. The
fly-through's caves are one connected system, so the fill always runs to its cap and the cap
becomes the volume; a line-of-sight fan measures the chamber the listener is in. Multi-bounce
ray statistics (the mean segment length is the mean free path without a star-shaped
assumption) were set aside for the same plan's second pass: they need a seeded direction
stream and about four times the rays for the same precision.

### Alternative E: A fourth field in the acoustic triple for surface absorption

Honest about the two physical quantities, but it widens a row the consumer's requirement names
as a triple and leaves every existing row to be re-authored. The skin-depth rule keeps the
triple and can be superseded by a wider row later without changing the mixer.

## Outcome

_(Added at acceptance if implementation falsifies something above.)_
