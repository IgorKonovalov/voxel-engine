# ADR-0047: Translucent solids filter what is behind by path length, and reflect a small share of their baked face light

> **Status:** proposed
> **Date:** 2026-11-08
> **Related plan(s):** [Plan 0059](../plans/0059-loaded-light-a-per-chunk-store-baked-at-load-and-at-edit.md), Phase 5a

## Context

A translucent solid (the glass row: opacity 0.6, albedo 0.25 0.35 0.85, no emission) is drawn by `translucent_march` in
`march.wgsl`. The march composites every voxel layer the ray enters as a diffuse wall, at the row's opacity, lit by the
headlamp only. A ray through the fly-through's 2-voxel pane enters 2 to 4 layers, so 84 to 97 % of the pixel is the
glass's own surface. That surface has no baked light (no store cell, no fallback). Plan 0059's Phase 5 window
(2026-11-07/08) showed both failures. With the lamp off, the pane is "very dark and consists of individual visible
voxels". With the lamp on, it "looks like a white non transparent pane" (`live/session5b-glass-lamp-on.png`).

The layer count jumps by one at every voxel edge, so the per-layer composite draws the voxel grid. Any model that
weighs light per layer keeps that grid. The surface term's size sets whether a lamp at 60 cm blows the pane out.

ADR-0029 (proposed) already absorbs light in a translucent liquid by Beer-Lambert over the refracted path.

## Decision

A translucent solid is a filter. The pixel is `T(L) * behind + R * face_light`, where:

- `L` is the ray's path length through the solid's voxels, summed from the march's entry and exit distances.
- `T(L) = tint^(L / d)`, per colour channel, with `d` one voxel. `tint` is the row's per-voxel transmittance,
  `0.8 * (0.5 + 0.5 * albedo / max(albedo))`. For glass that is (0.52, 0.56, 0.80): tinted, but passing a third of the
  light's luminance through a 2-voxel pane. The row's opacity is no longer read as a per-layer weight.
- `R` is a small reflected share: Schlick's Fresnel with `F0` = 0.04, from the view angle at the entry face.
- `face_light` is `E / PI`, untinted, since a dielectric reflects white. `E` is the entry face's baked store cell
  (the bake writes cells for translucent faces too), plus the lamp's analytic term while the lamp is on. The lamp term
  has no shadow ray, because the lamp is at the eye. The lamp's bounce is not added, since `R` is a few percent.
- The hit pass computes `R * face_light`, with the store bound read-only. `T` reaches the composite per channel in a
  second target (rgb9e5 in `r32uint`, 4 B a pixel). An opaque hit behind glass reads its own store cell, like any
  opaque hit.

Absorption depends on distance, so one voxel edge changes `L` by a fraction of a voxel, not a whole layer, and the pane
fades smoothly. `R` is at most a few percent at normal view, so no light on the face can turn the pane opaque.

## Consequences

### Positive
- No voxel grid in a pane, and no white-out from a near lamp.
- A pane in a lit room picks up the room's light through its baked face cell, with the lamp off too.
- The model matches ADR-0029's liquid absorption, so later liquids and solids share one rule.

### Negative
- The bake writes translucent faces' cells too. Glass is rare in a level, so the store grows a little; no reading yet.
- The march tracks entry and exit distances instead of counting layers, which adds a few registers on the translucent
  path only.
- A second screen target for `T`: 4 B a pixel, about 14 MB at 2530 x 1378. The store gains a read-only binding on the
  hit pass.
- Glass reads brighter and bluer than before at every pose. `translucent.png` is re-blessed.
- The march's source hash changes, so every store file is re-baked once.
- No refraction and no scattering inside the glass; a frosted look needs another rule.

## Alternatives considered

### Alternative A: Keep the per-layer composite, add baked face cells

Fixes the darkness with the lamp off, but leaves the voxel grid (a layer per voxel edge) and the lamp's white-out
(the surface term is still 84 to 97 % of the pixel). It lost because two of the owner's three complaints remain.

### Alternative B: Filter only, no reflected share

The simplest. A pane in a lit room then shows no light of its own and reads as a coloured hole. It lost on the owner's
pick (2026-11-08, "Both").

### Alternative C: Treat glass as an opaque face with a store cell

Cheapest, since it is the opaque path, but it is not glass. Rejected outright: translucency is the row's point.
