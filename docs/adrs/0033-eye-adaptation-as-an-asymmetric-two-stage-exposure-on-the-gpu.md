# ADR-0033: Exposure adapts like an eye: fast toward bright, slow toward dark in two stages, metered and stepped on the GPU, applied as pre-exposure

> **Status:** accepted (2026-10-28, Plan 0040 closed; see Outcome)
> **Date:** 2026-10-22
> **Related plan(s):** [Plan 0040](../plans/done/0040-eye-adaptation-asymmetric-auto-exposure-and-pre-exposure.md)
> (builds it), [Plan 0032](../plans/0032-sun-and-sky-hillaire-atmosphere-time-of-day-and-an-outdoor-testbed.md)
> (its Phase 4 shrinks to using it), [Plan 0029](../plans/done/0029-indirect-light-probe-volume.md)
> (the live check that asked for it)

## Context

The frame's exposure is a fixed `f32` that the composite multiplies in just before the ACES
tone map. At Plan 0029's Phase 7 check the owner turned the headlamp off in a dark room and
asked for what an eye does: "in real situation I would be blinded at first seconds when high
light is turned off, and gradually my eyes would be accustomed to darkness and I would see
normally again" (2026-10-22). Plan 0029 Phase 8 fixes how fast the bounce light leaves. The
rest of the request is exposure, which the engine does not adapt at all today.

Plan 0032 Phase 4 (approved, not started) already designs an automatic exposure for daylight.
It meters per-workgroup sums of log2 luminance and steps them in a one-workgroup pass. It
adapts symmetrically at `1 - exp(-dt speed)` with `speed` 1.5 per second. It pre-exposes the
HDR targets so that a sun of about 3000 frame units keeps the `rgba16float` second moment
finite (3000^2 is 9 million, and the largest finite half is 65 504). A symmetric rate cannot
give the owner's effect. A rate fast enough for the lamp coming on makes the lamp going off
recover in a fraction of a second. A rate slow enough for the dark leaves the image blown out
for seconds when the lamp comes back.

The literature splits the eye's time course into a fast light adaptation and a dark
adaptation that is slow and runs in two stages. Hecht, Haig and Chase (1937, "The influence of
light adaptation on subsequent dark adaptation of the eye") measured the two-branch dark
adaptation curve: the cones settle first, then a break, then the rods carry sensitivity much
further down. Ferwerda et al. (1996, "A model of visual adaptation for realistic image
synthesis") model the threshold, colour and acuity of each system. Pattanaik et al. (2000,
"Time-dependent visual adaptation for fast realistic image display") give each system its own
time course. Durand and Dorsey (2000, "Interactive tone mapping") and Krawczyk et al. (2005,
"Perceptual effects in real-time tone mapping") reduce this to an exponential follower of the
adapting luminance, with time constants that depend on the direction or the level. Real dark
adaptation takes minutes. Every real-time model compresses it to seconds, and so does this one.

The engine defines no photometric units. One frame radiance unit is a convention ("about 30
cd/m^2" in Plan 0032, a guess). The first consumer is a dark cave lit by a headlamp and
glowing ore. Other consumers go outdoors. The engine reads no clock, so the caller supplies
`dt`.

## Decision

We adapt one scalar, the log2 of the adapted luminance `a`, toward the frame's metered log2
luminance `t`. Each step uses an exact exponential over the caller's `dt`. **Toward bright**
(`t > a`) the time constant is `tau_light`. **Toward dark** it is `tau_cone` while `a` is
above a game-set luminance `cone_floor`, and `tau_rod` below it. A step that crosses the floor
splits its `dt` at the crossing, so the trajectory is the closed-form two-branch curve at
every frame. The exposure is `key 2^compensation / 2^a`, clamped to `[min, max]`. All of
these are fields of `AutoExposure` that the game sets, and a symmetric preset reproduces Plan
0032's `speed` model exactly. The meter is the composite's radiance before exposure and before
the tone map, as log2 luminance clamped to `[-16, 16]`. It is summed per 8 x 8 workgroup, and
the pass weights each tile by `1 + strength exp(-r^2 / (2 sigma^2))`. Here `r` is the tile
centre's distance from its view's centre, in half-heights. The default is centre-weighted
(`strength` 3, `sigma` 0.5), and `Metering::Average` is `strength` 0. A one-workgroup pass at
the end of the frame meters and steps the state on the GPU, and the next frame uses the
result. A small ring of read-back buffers gives the CPU the state a few frames late, for
display only. Every value a pass stores into an HDR target is pre-exposed, that is, multiplied
by the frame's exposure. Lighting math stays in absolute units up to the store. A pass that
blends or compares stored history first rescales it by `current / previous`. A stored value
is clamped to 60 000, and the second moment is taken of `min(Y, 255)`. The probe atlases and
the reservoirs stay in absolute units, and the composite multiplies them in where they join.
The glare moment is no extra mechanism. A flash shorter than a few `tau_light` still drops the
exposure most of the way, and the slow return then shows the dark scene near black and fading
in.

## Consequences

### Positive
- The owner's lamp-off look: near black at once, the ore's light filling in over seconds. The
  lamp-on look is a dazzle that clears in about half a second. A lightning strike (Plan 0036)
  and a cave mouth at noon (Plan 0032) get the same behaviour with no code of their own.
- The trajectory is closed form, so a headless test compares the GPU's exposure per frame
  against arithmetic, not against a second implementation.
- One frame of latency, the same on every adapter and in every test: frame `n + 1` uses the
  meter of frame `n`.
- Pre-exposure makes the HDR targets scale-free. A scene and its exposure scaled by `s` and
  `1 / s` store the same halves, so daylight (Plan 0032) and a dark-adapted cave at exposure
  64 both stay finite in `rgba16float`.
- Plan 0032 Phase 4, Plan 0035's cloud buffers and Plan 0037's night tint, which read
  "pre-exposure luminance", find the mechanism already built.

### Negative
- A kink in the rate at the cone floor. The curve's slope falls by `tau_rod / tau_cone` at
  the break, four times at the defaults. That
  is the classic curve's shape, but an eye may read it as a hitch where a smooth model
  (Krawczyk's level-dependent constant) would not. It is a measured change if the owner sees
  it.
- `cone_floor` is a luminance in the game's units. The engine cannot default it sensibly for
  every consumer, and a game that rescales its emission must rescale the floor.
- The adapted state lives on the GPU. The CPU sees it two or three frames late, so a game
  that wants gameplay from "how blinded is the player" reads a lagging value or runs the same
  pure function itself (`AutoExposure::step` is public for that).
- Every pass that stores HDR history must remember the rescale rule. A new pass that forgets
  it shows a pulse of wrong history on every exposure change, and one that forgets the clamp
  overflows. Plan 0040's invariance and overflow tests are the guard.
- Pre-exposure changes the f16 rounding of every stored value when the exposure is not 1.0.
  Images at a manual exposure other than 1.0 move by rounding (within 1/255), not byte for
  byte.
- Clamping the second moment at a luminance of 255 understates the variance of pixels that
  the tone map shows as white anyway.

## Alternatives considered

### Alternative A: one symmetric rate (Plan 0032 Phase 4 as approved)
`1 - exp(-dt speed)` toward any target, with `speed` 1.5 per second. It is the smallest model,
and it is the symmetric preset here. It lost because no single rate gives both halves of the
owner's request: a fast recovery from the dark kills the "blinded at first" moment, and a
slow adaptation to the light leaves the lamp-on frame white for seconds.

### Alternative B: a perceptual model of the eye (Ferwerda 1996, Pattanaik 2000)
Separate cone and rod responses with threshold-versus-intensity functions, pigment bleaching,
loss of colour and acuity, and a time-dependent tone reproduction operator in place of the
ACES curve. It is the faithful option. It lost because it needs photometric units the engine
does not define, it replaces the tone map that every golden pins, and its scotopic look (blue
shift, desaturation) is already Plan 0037's luminance tint (ADR-0032). We take its time
course only.

### Alternative C: adapt on the CPU from a read-back meter
Read the meter back each frame and step the model in Rust, with the exposure in the uniform
block. The math becomes plain and deterministic, and the game holds the state. It lost because
the read-back arrives one to three frames late, depending on frames in flight and the driver.
The exposure would then lag the image by a varying amount, the glare's first frames would
differ between adapters, and the per-frame trajectory test would depend on pacing. It also
needs a buffer map every frame in the frame path.

### Alternative D: exposure at display only, no pre-exposure
Keep the targets in absolute units and multiply by the exposure in the composite, as today. It
needs no rescale rule and keeps every manual-exposure image byte for byte. It lost to the
range. Daylight overflows the moments (Plan 0032's fact), and a target's halves lose
precision at the dark end, where the eye's exposure is highest. Pre-exposure keeps the stored
values near the key either way.

## Outcome

2026-10-28, at Plan 0040's close (v0.34.0). The decision holds as built; these details moved.

- **The defaults.** The owner's windowed sessions chose `tau_light` 0.15 s, `tau_cone` 2 s,
  `tau_rod` 10 s and `max` 16, so the dark-adapted cave stays dim rather than settling at the
  key. The kink at the cone floor is therefore a fivefold fall in slope, not the fourfold of
  Consequences. Phase 5's log records no verdict on the kink; Krawczyk's smooth constant stays
  a followup.
- **The state the frame passes read** is a 32-byte uniform (group 0 binding 13), copied from
  the exposure pass's storage buffer after it runs, not that storage buffer bound read-only.
- **The clamps.** The moments store is clamped at `(255, 255^2)`, and the first moment is also
  of the clamped luminance; the 60 000 clamp of other stores stands.
- **The closed form is a reading, not a bar, on the level.** On the fly-through's level the
  meter falls over about 30 frames when the lamp goes off, not in one step, so the GPU's curve
  matches the stepped model exactly (0.0000 EV) but sits 0.1 to 0.2 EV from the one-step closed
  form. Plan 0040's headless bars assert the stepped model on both test poses and print the
  closed-form distance.
- **FSR 2's seam.** FSR 2 (ADR-0034) receives pre-exposed colour; its tone pass no longer
  multiplies by the exposure.
- **Light patches while adapting.** The patches seen while the exposure moves follow the
  emissive light samples (they step the same under `Manual` at the adapted exposure), so they
  are the emissive estimator's, not this mechanism's. Plan 0047 owns them.
- **Cost (quiet, 2026-10-28, `docs/nfr.md` §2 and §3):** the meter adds 0.05 to 0.07 ms to the
  composite and the exposure pass reads 0.073 to 0.075 ms on the RTX 3080 Laptop at 1080p
  internal; on RADV RENOIR +0.106 ms and 0.060 ms. Both are above the plan's dev-box guesses
  (0.02 to 0.05 and 0.01 to 0.03 ms) and well inside the 1.5 ms post line. `frame_bytes` grows
  by 260 128 B.
