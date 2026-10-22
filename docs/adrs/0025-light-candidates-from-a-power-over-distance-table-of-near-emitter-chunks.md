# ADR-0025: Light candidates come from a power-over-distance table of the emitter chunks near the camera, and only those chunks hold light samples on the GPU

> **Status:** accepted (2026-10-22, at Plan 0024's close)
> **Date:** 2026-10-21
> **Related plan(s):** [Plan 0024](../plans/done/0024-direct-light-hit-buffer-emissive-samples-and-svgf-denoiser.md)
> (Phase 7, added by the owner's amendment of 2026-10-21); refines the candidate source of
> [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)

## Context

Plan 0024 Phases 2 to 4 built ADR-0018's sampler with its candidates drawn uniformly from the
whole light-sample buffer. The readings (`docs/nfr.md` §2, "The light pass by shadow walk" and
"Light extraction inside `sync`", RTX 3080 Laptop GPU, loaded box) show what that costs. With
the ore emissive the light pass reads 5.3 to 7.3 ms at the level's spawn and 9.2 to 13.2 ms at
the streamed spawn against the 6.0 ms lighting line; with the ore dark the frame's three passes
sum to 2.2 ms. Most of the level's 417 388 samples are far from any given hit, so most
candidates are far, the resampled winner is often far, and its shadow ray is long. The far
candidate also contributes almost nothing: an emitter face at 256 voxels gives `1/65536` of
what it gives at one voxel.

The buffer fills with the whole streamed window. The autopilot's flight ends with 997 740 of
the 1 048 576 reserved samples in use, because every resident full chunk within `r_full` keeps
its range, and an allocation that does not fit drops the chunk's light silently. Extraction
rewrites a chunk's whole range each sync that writes any of its bricks, and its p99 over the
flight reads 13.3 ms (max 48.9 ms) of render-thread time against the 1.0 ms streaming line.

ADR-0017 keeps the mirror's CPU work in the frame loop, bounded per frame; any fix to the
extraction lives inside that rule. A light source the consumer cares about (a vein, a lamp, a
glowing crystal) is within tens of metres of the player in a cave whose views are tens of
metres.

## Decision

**A candidate's light sample comes from a per-frame table of the emitter chunks within a light
radius of the camera, chosen by power over squared distance, and only the chunks inside that
radius (plus one chunk of hysteresis) hold a range of the light-sample buffer.**

- The mirror keeps each full chunk's exposed-emitter lists on the CPU as today, plus the
  chunk's *power* (the sum over its samples of the material's emitted luminance times the
  sample's exposed face count; the render-side emission scale is left out, so a pulse needs no
  rebuild).
- A chunk gets a GPU range when its distance to the focus chunk is at most `light_radius`
  chunks (default 4, 25.6 m at 10 cm voxels: a guess the plan's close reads) and loses it beyond
  `light_radius + 1`. Ranges are written nearest first under a per-sync sample budget, at most
  once per chunk per sync, and carry over. When the buffer has no run for a nearer chunk, the
  farthest range is evicted to make room; a chunk is never dropped silently.
- Each frame the CPU builds a table of the chunks that hold a range and lie within
  `light_radius` of the camera: `(start, len, cumulative weight)`, weight
  `power / max(d^2, d_min^2)` with `d` the camera's distance to the chunk's box and `d_min` one
  chunk, in a fixed coordinate order so a frame is reproducible. A candidate draws a chunk by
  binary search over the cumulative weights and a sample uniformly in its range; the source pdf
  `(w / W) / len` replaces `1 / span` in the resampling weight, so the estimate stays unbiased
  for every emitter inside the radius and ignores the ones outside it.

## Consequences

### Positive
- Candidates land near the camera, where the hits are, so the winner's shadow ray is short and
  the light pass's cost tracks the near emitters, not the world's.
- The buffer's use is bounded by the radius, not the streaming window: about `(4/12)^3` to
  `(4/16)^3` of the window's chunks at `r_full` 12 to 16, and eviction by distance covers a
  denser cave than that.
- Extraction work per sync is bounded by the budget, and a chunk that arrives over several
  syncs is rewritten once, not once per sync.
- The table is one small uniform (1024 entries of 16 bytes, 16 KiB), not another storage
  binding, and needs no rebuild on an edit beyond the edited chunk's own range and power.

### Negative
- **Bias by construction.** An emitter beyond the radius contributes nothing. A vein of 1000
  exposed faces at 26 m adds about `1000 / 65536` of a single face at one voxel; the showcase
  cavern (the plan's followup) is where a bigger radius or a far-field term gets judged.
- **The weights are camera-centred, not hit-centred.** A hit 20 m down a tunnel samples its own
  neighbours' emitters with the weight of their distance to the camera, so a distant pixel lit
  by a distant emitter is noisier than a near one. Reservoir reuse (the plan's next phase) and
  the owner's eye are the check.
- A chunk entering the radius lights its surroundings only once its range is written, a delay
  bounded by the budget's carry-over (the plan sets the bar).
- One more per-frame CPU step on the render thread (a table over at most a few hundred chunks).

## Alternatives considered

### Alternative A: a light tree over the samples (many-lights BVH, stochastic lightcuts)

A bounding-volume hierarchy over every sample, traversed per candidate with a per-hit
importance estimate (Conty Estevez and Kulla 2018). It is the best per-hit sampler, but it is
rebuilt or refitted whenever streaming or an edit changes the samples, which is the extraction
cost this decision is bounding, and its traversal is a divergent walk of about 20 levels per
candidate. It lost because its build cost lands on the line that already misses.

### Alternative B: per-hit spatial bins (each chunk's table of its neighbours)

Each emitter chunk keeps a cumulative table of the emitter chunks in its 3^3 or 5^3
neighbourhood, and a hit draws from its own chunk's table. Weights are hit-centred, which fixes
the tunnel case above. At 3^3 it drops every emitter more than one chunk (6.4 to 12.8 m) away,
which a cavern needs; at 5^3 every change to one chunk's samples rewrites 125 tables. It lost on
that rebuild cost; it is the next step if the camera-centred table's distant pixels read
noisy.

### Alternative C: a global power-weighted table without distance

An alias table over every chunk by power alone keeps the bright emitters and still picks them
wherever they are, so the far-emitter shadow ray, the cost being fixed, stays.

## Outcome

Accepted 2026-10-22 at Plan 0024's close (Phase 7). Two claims above were falsified:

- **The table is not a uniform.** Indexed dynamically per pixel, the 16 KiB uniform read 9.42
  ms against 7.45 ms for the same table in storage (loaded box), so it lives in 1025
  sample-sized slots past the light-sample reservation (`GpuWorld::light_table_base`), written
  each frame; no new binding was needed.
- **Extraction is not bounded enough by the budget alone.** Over the autopilot's flight it
  reads p99 2.21 ms and max 6.12 ms on the quiet box against the plan's 1.0 and 3.0 ms; moving
  extraction to the stream workers goes with Plan 0028.

What held: the table's 64-frame mean is within 2.9 % of the analytic box with a relative RMSE
0.35 of the uniform draw's; the autopilot's light-sample peak is 18 522 with nothing refused
and no wait inside the radius (against 997 740 under the uniform pool). A light sample is 16
bytes (a world voxel and face bits), not the 8 of the plan's data shapes.
