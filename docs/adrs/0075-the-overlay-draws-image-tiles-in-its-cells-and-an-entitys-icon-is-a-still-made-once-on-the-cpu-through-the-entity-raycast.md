# ADR-0075: The overlay draws image tiles in its cells, and an entity's icon is a still made once on the CPU through the entity raycast

> **Status:** proposed
> **Date:** 2026-11-18
> **Related plan(s):** [Plan 0077](../plans/0077-an-entity-picker-in-the-fly-through-choose-a-model-from-the-library-and-place-it.md) (Phase 7)

## Context

Plan 0077's chooser lists the library's models as rows of words, by its original decision ("No
thumbnails"). The owner's first window (2026-11-18) asked for "a small preview of the entity"
per entry, and the same day for a Minecraft-style belt of slots along the bottom of the view;
both want a picture per model. The owner answered question 17 with a **still** icon.

The overlay is ADR-0009's: an embedded 8 x 8 bitmap font, a grid of `u32` cell words (bits
0..8 the byte, 8..16 the colour, 16 the backdrop since Plan 0080) drawn by one fullscreen pass
at an integer scale, pixel-exact on lavapipe, with no UI library. Its negative was recorded
then: no widgets, no image. The editors converge on a 64-pixel thumbnail made off the main
thread and delivered when ready (Godot's `EditorResourcePreview`, Unity's `AssetPreview`:
"poll this and repaint when it finishes"), and Minecraft's hotbar draws a block at a fixed GUI
transform, 30 degrees down and 225 degrees round at 5/8 scale
(`docs/references/entity-placement-ux.md`, "The chooser and its pictures").

`vxe-entity` already answers the ray query the icon needs: `EntitySet::raycast` traces a ray
through a set of posed models in integer arithmetic (fixed-point positions, Q14 quaternions,
`1/256` voxel ray steps), headless and deterministic on both CI arms. The frame pipeline, by
contrast, renders a `Frame` through the march, the light passes, the denoiser and the upscaler
against the scene's lighting, with its own set of targets; it is built to draw the world, not
a diagram of one model.

## Decision

Two facilities, one in the engine and one in the app, and the engine's knows nothing of icons.

`vxe-render`'s overlay gains **image tiles**: `HudOverlay::set_tile(queue, index, &rgba8)`
writes a 64 x 64 RGBA8 tile into an atlas of `MAX_TILES` = 32 (512 KiB of VRAM), and a cell
word's free bits 17..31 name a tile and the cell's position in it. A tile covers a 4 x 4 block
of cells at any glyph scale, so a tile pixel is `scale / 2` output pixels, **nearest-sampled**
so a voxel model's pixels stay crisp and the overlay stays pixel-exact for the goldens.
`hud.wgsl` draws the tile's pixel where the cell has the tile bit and no glyph byte, above the
backdrop; a glyph on a tile cell draws over the tile, so a digit or a frame sits on a picture.

The fly-through makes an **entity's icon once, on the CPU**, through
`vxe_entity::EntitySet::raycast` over the entry's model in its rest pose in a scratch set: an
orthographic view 30 degrees down and 225 degrees round, the model fitted to 5/8 of the tile,
each pixel the hit voxel's material colour from the table shaded by a fixed Lambert from the
entered face mapped through the part's rest rotation, in integer arithmetic, so an icon is a
**fixed vector** bit-identical on both CI arms. It is made on a worker thread at the library's
build and again when an entry reloads; a row or a slot shows its words until the tile arrives.
A still, not a live view: the ghost standing in the world (ADR-0074) is the live view of the
selected one.

## Consequences

### Positive

- The overlay stays ADR-0009's one pass with no library: the tile is one more kind of cell,
  the goldens stay bit-exact, and the atlas is a fixed 512 KiB reservation.
- An icon costs its rays once (the guess, Plan 0077 "Budgets added 2026-11-18": 5 to 50 ms per
  entry for 4096 rays through a fourteen-part model, off the frame thread), so the start time
  of `docs/nfr.md` section 7 does not move and the frame never waits for a picture.
- Deterministic: a CPU test hashes the walker's icon to a fixed vector on both arms, and a
  wrong pixel is a diff, not a judgement.
- General on both sides: a consumer draws any 64 x 64 picture (a map tile, a portrait, a
  swatch) in its overlay, and any consumer with `vxe-entity` can make icons the same way.

### Negative

- Icons are not lit as the scene is: a model under a red lamp has a neutral icon. That is the
  editors' convention (a diagram, not a render) and the owner's pick; the ghost shows the lit
  model.
- One fixed view for all models: a model whose readable side is not its front-left
  three-quarter shows its worse side. A per-model icon view (Unreal's thumbnail edit mode) is
  a later addition to the rig grammar (ADR-0051), not taken now.
- 32 tiles of 64 x 64 is a cap: a library past it reuses indices, and a larger atlas or a
  second tile size is a settings change.
- The overlay's cell word is now full (bits 0..16 the glyph, colour and backdrop, 17..31 the
  tile); the next cell attribute needs a second word per cell.
- The icon renderer is the app's code to keep right (fit, shading, the face-to-normal map),
  with the fixed vector as its one guard.

## Alternatives considered

### Alternative A: a live icon, a `Frame` per entry through the frame pipeline

Each slot's model rendered every frame into its tile by the engine's own pipeline, so the
icon is lit and shaded exactly as the model will be. Rejected because it couples an icon to
the scene's lighting and the camera's exposure (the icon changes as the player turns), costs
the pipeline's targets per entry per frame on the headroom line, and the ghost in the world
already gives the live view of the one that matters; Unreal ships live thumbnails as an
opt-in behind a still default.

### Alternative B: a UI library for the picture panel

`egui` or a widget crate drawing the chooser and the belt with image widgets. Rejected for
ADR-0009's reasons, which stand: the dependency tree and its `wgpu` version coupling, a second
overlay pass that disturbs the readings the HUD displays, and the loss of pixel-exact goldens
on lavapipe; a tile in a cell is all a picture panel needs here.

### Alternative C: icons as glyph art

The model projected into the font's 8 x 8 cells as coloured block glyphs, needing no atlas.
Rejected because a 4 x 4 block of cells gives 4 x 4 "pixels" per icon at any scale, which
tells a spider from a walker and nothing more; the atlas's 64 x 64 is what the editors
converge on and what a fourteen-part model needs to read.

## Outcome

_(Added at acceptance if Plan 0077's Phase 7 reading or Phase 8 window falsifies something
above.)_
