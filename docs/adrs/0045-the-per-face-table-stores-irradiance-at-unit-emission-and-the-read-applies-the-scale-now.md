# ADR-0045: A per-face table cell stores irradiance at unit emission, and the read multiplies by its kept emitter's emission scale now

> **Status:** proposed
> **Date:** 2026-11-04
> **Related plan(s):** [Plan 0058](../plans/0058-dynamic-lights-a-list-of-analytic-lights-dropped-carried-and-bounced.md)
> (builds it, in its emission-scale phase, on the table
> [Plan 0057](../plans/0057-steady-light-the-per-face-table-as-the-emissive-store-and-a-lamp-bounce-channel.md)
> builds). Refines
> [ADR-0043](0043-emissive-light-from-a-world-space-per-face-table-and-a-lamp-bounce-probe-channel.md)
> (what a cell's sums hold and what the guard compares); leaves its key, its fill, its
> invalidation box and its lamp channel as they are. Rests on the per-material emission scale
> of [ADR-0018](0018-direct-light-per-pixel-one-sample-from-a-hit-buffer-with-svgf-history-killed-by-edits.md)
> and [ADR-0023](0023-indirect-light-from-a-scrolling-probe-volume.md) (`Frame::set_emission_scales`).

## Context

The consumer's P0 asks for "emissive voxels as light sources, with emission changeable at
runtime", with the theme hook "Choir crystal glows when sound reaches it, so sound makes light"
(`project-context.md`, Rendering). The owner's rule of 2026-11-03: "light that never settles
when objects are not moving + not natural flickering light (flame eg) - are super unsettling".
A flame may flicker because a flame flickers. Nothing else may.

The renderer has two ways to change an emitter's light at runtime. A **material swap** through
an edit op changes which voxels emit; it is authoritative, logged, and invalidates the table
through the edit box (ADR-0043). A **render-side per-material emission scale**
(`Frame::set_emission_scales`, an `f32` per material id read by `emission_of`) scales a whole
material's emission on the pixels' light and, separately, on the probe rays'; it is not
authoritative and costs no rebuild (ADR-0025 leaves it out of the chunk powers). The scale is
how a pulse, a flicker or a glow that follows sound is meant to reach the screen.

ADR-0043's table breaks the second way. A cell's three sums hold the mean of samples that were
taken under the scale of their frame, so a scaled-up emitter's faces keep showing the old mean
until something reopens them; and ADR-0043's guard does reopen them, because it re-shades the
kept emitter with "the emitter's radiance now" and the result moves past the dead zone. A flame
whose scale changes every frame therefore reopens every cell it lights every frame, and those
faces fall back to a one-sample mean: the noise the table exists to remove, exactly where the
owner wants a steady flicker. ADR-0043's Negative consequences do not list this; Plan 0057
inherited the emission-change window as a probe-channel detail only.

Two facts make the fix small. The light pass already knows each sample's emitter material
(the candidate carries it, and `emission_of` is where the scale enters). And Plan 0057's
Phase 3 stores a cell's kept emitter voxel and face, from which the mirror gives the material.

## Decision

**A cell's sums hold irradiance at unit emission scale, and a pixel's emissive light is the
cell's mean times the current emission scale of the cell's kept emitter's material.** In
detail:

- **Add at unit scale.** The light pass and the fill pass divide a sample's contribution by the
  emission scale of the sample's emitter material at the frame the sample is taken, before the
  Q17.15 add. The firefly clamp is applied at unit scale too, so a scale change does not move
  the clamp's bias. A material whose scale is 0 at the sample's frame contributes no sample (the
  division is undefined and the sample carries no information).
- **Read times the scale now.** The temporal pass reads the cell's kept emitter, looks up its
  material in the mirror and multiplies the mean by that material's current pixel scale before
  the exposure. A cell with no kept emitter (lit by nothing) reads 0 as before. The lookup is
  one mirror read per cacheable pixel, the kind the hit pass already does.
- **The guard compares at unit scale.** ADR-0043's re-shade of the kept emitter divides its
  result by the emitter material's current scale before the dead-zone test, so a scale change
  reopens nothing, and a reveal, a dug-away emitter or a material swap still does.
- **Exact for one material, approximate for mixed cells.** A face lit by emitters of one
  material follows that material's scale exactly, at the frame the scale changes, with no
  reopen. A face lit by emitters of two materials follows its kept emitter's material and shows
  the other's light at the scale it had when sampled, until the cell is reopened by the box,
  the guard or the sweep. The kept emitter is the one that contributed most, so the error is in
  the smaller share.
- **The emission-change window goes from the pixels' path.** `LIGHTING_EMISSION_CHANGED` and
  the `ALPHA` floor it imposed on the emissive blend are moot: the table has no blend, and the
  fallback (a hit with no face) keeps its own short history as ADR-0043 says. The probe
  channel's window stays for whatever ADR-0043's lamp channel keeps of it.
- **The scale stays per material.** Two crystals of one material glow together. A crystal that
  must glow alone is its own material row, or is swapped to a glowing material through an op;
  a per-emitter scale is a later ADR if the consumer asks.

## Consequences

### Positive
- A flame flickers at full amplitude on every face it lights, the frame its scale changes, and
  not one cell reopens for it. The owner's "a flame may flicker" and "nothing else may" are
  both met by construction.
- The sound-lit crystal, the consumer's theme hook, works through the scale it was designed
  for, with the table steady underneath.
- The table's means become a property of geometry and material, not of when the camera looked:
  two runs that sample a face under different scale histories converge to the same mean.
- One mirror read and one multiply per cacheable pixel, and one divide per sample added: no new
  buffer, no new pass.

### Negative
- **Mixed cells are approximate.** A face lit about equally by two materials whose scales
  diverge shows the smaller share's light at a stale scale until a reopen. A two-material test
  is a reading in Plan 0058, not a bar; if the owner sees it, the cell grows a second kept
  emitter, which is a later ADR.
- **A scale of 0 hides an emitter from the table.** Faces lit only by a material at scale 0
  gather no samples while it is dark and start from nothing when it lights: the fill pass's
  rate, not an instant. A material that switches between dark and lit is better driven between
  a small floor and its full value.
- **The kept emitter's material is read from the mirror.** A streamed-out chunk (the sweep
  clears those cells) or a swapped voxel (the edit box clears those) cannot be reached, so the
  read is safe by ADR-0043's invalidation rules; a cell whose kept voxel is no longer an
  emitter reads scale 1 and is reopened by the guard's next visit.
- **Plan 0057's table lands without this.** Its Phase 2 and 3 as drafted store scaled sums and
  reopen on a scale change; the behaviour between 0057's close and this ADR's phase is the one
  the Context describes. No default scene in the fly-through pulses, so no window sees it.

## Alternatives considered

### Alternative A: Reopen on a scale change (ADR-0043 and Plan 0057 as drafted)
The guard re-shades at the current radiance and reopens cells whose light moved. Correct for
every one-off change, and it needs no divide. It lost because a periodic change (a flicker, a
pulse) reopens its cells every frame and the faces show a one-sample mean: the flame becomes
the one place in the image that boils.

### Alternative B: Sums per material per cell
Each cell keeps one set of sums per emitter material that lit it, each read at its own scale,
exact for mixed cells. It lost on size and bound: a cell of ten words becomes a cell of ten
plus three per material, with no bound on the materials that may light a face but the material
table's, and the common case (one material per face) pays for the rare one.

### Alternative C: Smooth the scale over time
Feed the table the scale's running average (as Plan 0029 fed the probes "a pulsing one at its
average") and let the pixels show a steady mean. It lost because it is the owner's rule
inverted: a flame that does not flicker is the "not natural" case the rule names, and the
pixels' scale was always meant to follow the change at full amplitude.

## Outcome

_(Added at acceptance, at Plan 0058's close, if implementation falsified something above.)_
