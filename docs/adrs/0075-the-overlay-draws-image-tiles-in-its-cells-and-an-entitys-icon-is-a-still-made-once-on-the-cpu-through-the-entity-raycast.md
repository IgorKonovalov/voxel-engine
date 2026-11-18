# ADR-0075: The overlay draws image tiles in its cells, and an entity's icon is a still made once on the CPU through the entity raycast

> **Status:** accepted (Plan 0077, 2026-11-18, with an Outcome)
> **Amended 2026-11-18 (architect, at Plan 0077's Phase 5 checkpoint):** the cell word is not
> full. A tile needs 9 bits (5 for `MAX_TILES` = 32, 4 for the cell's place in its 4 x 4
> block), so the tile takes bits 17..26 and bits 27..31 stay free; the Decision and the
> Consequences say so below. The colour was always per cell (bits 8..16, read per cell by
> `hud.wgsl`); only `HudOverlay`'s CPU API colours whole rows, and Plan 0077 Phase 7 adds a
> span colour over the written rows beside the tiles, with no new bit.
> **Date:** 2026-11-18
> **Related plan(s):** [Plan 0077](../plans/done/0077-an-entity-picker-in-the-fly-through-choose-a-model-from-the-library-and-place-it.md) (Phase 7)

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
word's free bits from 17 name a tile and the cell's position in it: a tile bit, 5 bits of tile
index and 4 of position, bits 17..26, with 27..31 still free. A tile covers a 4 x 4 block
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
- The overlay's cell word is nearly full (bits 0..16 the glyph, colour and backdrop, 17..26
  the tile): five bits remain for the next cell attribute, and the one after needs a second
  word per cell. A per-cell colour is not a new attribute: the colour field is per cell
  already, and the CPU API gains a span colour (Plan 0077 Phase 7).
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

Added 2026-11-18 at Plan 0077's close (Phase 7 landed at 0011560f, the icons re-cut at
87e2aff0 after the owner's Phase 8 window; "all fine" at Phase 10).

- **The fit is 7/8 of the tile, not Minecraft's 5/8, and the fit is the silhouette's.** At 5/8
  the owner read the icons as "small and dim" beside the words; the fit rose to 56 of 64 pixels
  on the longer axis (the walker's solid pixels from 10.5 % to 21 % of the tile), and it is
  taken over the model's projected solid voxels, not its box (the box fit gave the walker 16 x
  38 pixels). The face shade rose from `96 + 160 cos` to `176 + 80 cos` 256ths (the walker's
  three visible faces 198, 210, 244 bytes; the darkest at 0.77 of the albedo). The block stays
  4 x 4 cells and the tile 64 pixels: the cell word above is unchanged. `tests/icon.rs` pins
  the fit, the shades and the hashes.
- **The cost is under the guess.** The icon takes a median 1.89 ms for the six-part walker and
  14.68 ms for the fourteen-part spider (release, the dev box's Ryzen 9 5900HS, load 8.1), on
  a worker thread (`std::sync::mpsc`, polled with `try_recv` each frame); the atlas is
  524 288 bytes as a storage buffer at group 0 binding 4.
- **The overlay's paint is one struct**: `HudOverlay::set_lines_painted(queue, text, colours,
  HudPaint { backdrops, spans, tiles })`, the span colour of the amendment above and several
  backdrop boxes (the panel's and the belt's) beside the tiles; a tile index past `MAX_TILES`
  keeps its cell's words.
