# ADR-0006: SADIE II HRTFs in a packed f32 format, time-domain FIR, mixer on the device callback

> **Status:** accepted (2026-10-17)
> **Date:** 2026-10-16
> **Related plan(s):** [Plan 0006](../plans/done/0006-spatial-audio-hrtf-mixer-and-occlusion.md)

## Context

The first consumer's audio is its core mechanic: P0 positional playback with HRTF, and
occlusion and transmission through voxels by material (`project-context.md`, Audio). The
roadmap's M4 names two decisions for an ADR: the HRTF source and the audio thread model. The
engine is a public MIT OR Apache-2.0 repository, so the HRTF data's licence matters as much as
its quality. The dependency rule (`CLAUDE.md`) prices every crate and every binary blob.

The candidates found on 2026-10-16 (versions from crates.io):

- **SADIE II** (University of York), HRIRs for two dummy heads and 18 human subjects, in SOFA
  and WAV, licensed Apache 2.0 with attribution required. The SOFA archive for one head is
  38 MB.
- **IRCAM Listen**, the database behind the `hrtf` crate (0.9.0, MIT) through a bespoke
  binary "HRIR sphere" format produced by a C++ tool. The licence notice reproduced by
  downstream packagers says the database is public and available for any use with
  acknowledgement; the original page could not be reached to confirm. The crate's README
  reports audible clicks on fast-moving sources.
- **`sofar`** (0.4.0, MIT OR Apache-2.0), a pure-Rust SOFA reader (its own HDF5 parser) with a
  uniformly partitioned FFT convolution renderer and a real-time mode.
- **Steam Audio** (Apache 2.0) through `audionimbus` (0.16.0, MIT OR Apache-2.0): a prebuilt
  C++ SDK per platform with its own HRTF and its own propagation.

A 128-tap FIR at 48 kHz is 2.7 ms, which covers an HRIR's interaural delay (under 0.7 ms) and
its spectral shape. Thirty-two voices at two ears is 393 million multiply-adds per second,
a few percent of one core, so an FFT is not needed for the voice count the first consumer has.

## Decision

**HRTF source.** We ship one SADIE II head (subject D2, 48 kHz) packed by an engine-owned
example into an engine-owned format: a little-endian header, a direction table in azimuth and
elevation, and 128 `f32` taps per ear per direction. D2 is measured at 8802 directions; the
packer keeps every 3rd azimuth of each ring (2950 directions, 2.7 degree spacing; the full set
would be 9.08 MB in history, rejected). The pack (about 3.0 MB) is committed under
`crates/vxe-audio/assets/` with the Apache 2.0 licence text and the attribution SADIE II
requires. The 38 MB SOFA archive is not committed; the packer reads it through `sofar` as a
dev-dependency of the example and is re-run only to change the head or the tap count.

**Rendering.** Per voice, the mixer picks the nearest measured direction (a precomputed
azimuth-elevation bin table) and convolves the mono signal with the two 128-tap FIRs in the
time domain. A direction change crossfades the old and new filter outputs over one block. No
FFT dependency.

**Thread model.** The `cpal` output callback owns the mixer and renders blocks from the voices'
state without allocating or locking: commands arrive over a bounded channel drained at the top
of each block, and the voices' sample data is shared read-only. Acoustic probes (occlusion
through the material table, reflection taps) run on the caller's thread when the consumer calls
`update(&world, ...)` each frame, which keeps `vxe-world` locks out of the callback and lets the
game choose the probe cadence. Nothing in `vxe-audio` is deterministic by contract: it is the
perceptual layer; the consumer's gameplay noise model lives on M3's coarse grid, not here.

## Consequences

### Positive

- Licence-clean data with a written attribution path, and a reproducible pack from a public
  archive.
- No audio-thread allocation, no lock between the game and the callback, and a mixer that
  renders headless to a buffer for tests and benches.
- One small pure-Rust dependency at runtime (`cpal`) plus one dev-dependency for the packer.
- Time-domain FIR keeps the per-voice cost flat and the code small; the headless bench reads it.

### Negative

- **Nearest-direction switching.** Without interpolation between measurements, a slowly moving
  source steps between filters; the one-block crossfade hides the click, not the step. SADIE II's
  measurement grid is dense enough (a few degrees) that the step is small; the plan's moving
  source test bounds the discontinuity, and interpolation is a follow-up if the owner hears it.
  (2026-10-16, Plan 0006's third amendment: with the pack on one window start, the owner's
  recording while turning shows about 7 interaural level steps over 0.5 dB per second, which
  is the measured case for that follow-up.)
- **One head for everyone.** A generic dummy head localises worse than an individual HRTF for
  some listeners. Swapping the pack is a file change.
- **48 kHz only.** The mixer runs at the pack's rate; a device that cannot open 48 kHz fails
  with a clear error in Plan 0006 and gets a resampler in a follow-up.
- **Probes on the caller's thread** cost the game's frame budget (bounded by the probe count
  and cadence the plan sets) and stall if the world is mid-edit on another thread for the
  microseconds a chunk lock is held.

## Alternatives considered

### Alternative A: The `hrtf` crate with IRCAM spheres

A working MIT crate with MIT-licensed data files already in the Rust ecosystem. Rejected
because the data's licence could not be confirmed at the source, because the sphere format is
produced by an external C++ tool we would have to trust or port, and because the README's
reported clicks on fast sources are the artefact the plan's moving-source test exists to rule
out.

### Alternative B: `sofar` at runtime with the SOFA file

Partitioned FFT convolution and SOFA loading already written, under a compatible licence.
Rejected because it needs the 12 MB-per-rate SOFA file at runtime (committed or downloaded on
first run), an HDF5 parser and an FFT in the shipped binary, for a voice count that a 128-tap
time-domain FIR serves at a few percent of a core. It stays the packer's reader.

### Alternative C: Steam Audio through `audionimbus`

Production HRTF, occlusion, reflections and reverb in one SDK, Apache 2.0. Rejected because it
is a prebuilt C++ binary per platform (a supply-chain and `cargo deny` question the workspace
has avoided so far), because its propagation would duplicate the voxel occlusion the material
table is designed for, and because its scene representation is triangle meshes, which the brick
map would have to export.

### Alternative D: Mixing on the game thread into a ring buffer

The game thread renders audio blocks ahead into a buffer the callback drains. Rejected because
a frame hitch becomes a dropout, and because the callback then has nothing to do but copy,
while a mixer on the callback is the standard shape and is still testable headless.

## Outcome

Accepted 2026-10-17 at Plan 0006's close. The decision stands; four details measured
differently from the body above:

- **The pack's window.** Per-direction onset windows made the one-block crossfade a 5 ms phase
  jump while turning (up to 1.8 rad of 440 Hz phase between ring neighbours); the packer now
  starts every direction's 128 taps at one sample (62 for D2), and the clicks while turning
  went with it. The "nearest-direction switching" consequence remains the stepping the owner
  can still hear slowly; interpolation stays the followup.
- **The crossfade shape.** A linear one-block crossfade kinked the output at every block edge
  where the direction changed (227 times the static kink); it is a raised cosine.
- **The probe cost.** `Acoustics::update` for 32 sources reads 1.03 ms on the dev box against
  the 0.5 ms the plan proposed; the owner accepted about 1.0 ms as the ceiling (D2b).
- **Where the remaining clicks came from.** The clicks heard while the listener walked were
  the early-reflection tap swap, not the HRIR switching; ADR-0015 and Plan 0020 replace the
  swap with taps that glide.
