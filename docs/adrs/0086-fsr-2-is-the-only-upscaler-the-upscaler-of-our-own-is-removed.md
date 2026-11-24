# ADR-0086: FSR 2 is the engine's only upscaler; the upscaler of our own is removed

> **Status:** proposed (2026-11-24, with Plan 0102). At acceptance it supersedes
> [ADR-0024](0024-own-temporal-upscaler-on-the-hit-buffer.md) (what ADR-0034 left of it: the
> upscaler itself) and one clause of
> [ADR-0034](0034-fsr-2-ported-to-wgsl-as-the-default-upscaler-candidate.md): "our own
> upscaler stays selectable beside FSR 2", with its Outcome's "removing our upscaler stays
> with Plan 0045".
> **Date:** 2026-11-24
> **Related plan(s):** [Plan 0102](../plans/0102-remove-own-the-engines-own-upscaler-fsr-2-is-the-only-one.md)

## Context

ADR-0024 (Plan 0030) built a temporal upscaler of our own. ADR-0034 (Plan 0041) ported FSR 2
beside it and kept ours selectable. Since Plan 0055 FSR 2 at 1.5 is the default, the owner
kept it at Plans 0056 and 0061, and since Plan 0069 every profile reads FSR 2. Ours, `own`, is
an opt-in mode no profile uses.

The owner asked for its removal on 2026-11-10 and confirmed it on 2026-11-11, with Portable on
FSR 2. A day later he set a rule that pulls the other way: an unpicked arm that costs nothing
when off stays a render setting. So the question is real: is `own` such an arm?

It is not a branch inside a pass. It is a second output path: two compute passes and their
shaders, a history target, two uniform buffers, a jitter flag and a sharpening setting of its
own, a tuning struct of five fields in the settings surface, a third mode in the testbed, a
test binary and a golden. It is also the path that resizes a frame whenever the internal size
differs from the output and FSR 2 is off. And it is the baseline three of FSR 2's gate tests
compare against, though nobody has judged its picture since Plan 0055.

Against that, `own` is the cheaper upscaler: 0.93 to 1.3 ms against FSR 2's 2.9 ms at the
owner's window on the RTX 3080 Laptop, and 6 to 7 ms less on the Radeon iGPU (the static
review of 2026-11-10). At Portable on the RADV RENOIR FSR 2 reads 4.06 to 4.08 ms against a
3.0 ms line. Plan 0102's Phase 1 reads `own` at Portable's sizes; the figure is added here at
acceptance.

## Decision

**FSR 2 is the engine's only upscaler.** `UpscalerChoice` is `Off` or `Fsr2`. The upscaler of
ADR-0024, its passes, shaders, history, tuning, and the jitter and sharpness settings that
exist only for it are deleted. With `Off` a frame renders at its output size: the scale
applies as 1 and reads back as 1. The jitter sequence and the internal-resolution machinery
that FSR 2 uses stay, as ADR-0034 took them over.

## Consequences

### Positive
- One output path to keep right when the composite, the exposure, the targets or the frame's
  inventory change, and one picture to judge at a window.
- FSR 2's bars stand on absolute figures, not on a baseline nobody looks at.
- Fewer settings: `jitter`, `sharpness` and `own.*` leave the surface a consumer reads.
- Two pipelines fewer per frame object, a test binary and a golden fewer in the gate
  (Plan 0102 measures both).

### Negative
- **The slow adapter loses its cheaper upscaler.** FSR 2 stays over the 3.0 ms line at
  Portable on the RENOIR, and the arm that might have met it is gone. If the Radeon's frame
  needs those milliseconds later, the way back is a new plan, not a flag.
- **No temporal fallback.** If the FSR 2 port misbehaves on some adapter, the only other mode
  is native resolution with no upscaler.
- **No resize without FSR 2.** A consumer who wants a low internal resolution and a cheap
  stretch has to take FSR 2 or ask for a resize pass.
- **A breaking change** to `RenderSettings` and `Frame` for a consumer that named the removed
  fields or setters.
- Code that worked is deleted. It stays in history under the tag Plan 0102 sets.

## Alternatives considered

### Alternative A: keep `own` as a setting, off by default
The standing rule for cheap unpicked arms. It lost because `own` is not cheap when off: it is
a parallel path with its own passes, targets, settings, tests and golden, and every plan on
the output side pays for it. Keeping it would also leave FSR 2's bars tied to it.

### Alternative B: delete the temporal half, keep the pass as a plain resize for `Off`
It would keep "render low, stretch cheaply" available. It lost because no profile uses that
combination, and a pass kept for it keeps a shader, a layout, an inventory line and tests. A
bilinear resize can return as a small pass of its own when a consumer asks.

### Alternative C: keep `own` for the slow adapter (Portable on `own`)
The only place it has a measured advantage. It lost on the owner's answer of 2026-11-11
(Portable takes FSR 2) and on the stable-image priority: an upscaler per adapter is two
pictures to keep free of flicker, and Portable's picture under FSR 2 is the one the owner
accepted at Plan 0098's windows.
