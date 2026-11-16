# ADR-0065: Materials are data: a table file in the `.rig` line grammar, and every `.vox` palette entry becomes a row

> **Status:** proposed
> **Date:** 2026-11-16
> **Related plan(s):** [Plan 0083](../plans/0083-anything-from-a-vox-palette-materials-a-material-file-and-large-scenes-stamped-into-the-world.md)

## Context

A material is a row of `vxe_world::MaterialTable` (`MaterialId = u16`; albedo, roughness,
emission, hardness, flags, the acoustic triple, opacity, an opaque payload;
`crates/vxe-world/src/material.rs`). Today every row is written in Rust: the fly-through's
`apps/flythrough/src/scene.rs` builds about fifteen, and a consumer would do the same. A
`.rig` maps a `.vox` palette index to a row by name (`material 9 glow_amber`, ADR-0051); the
`.vox` file's own colour is ignored, and an index a part uses with no `material` line is a
load error (`VoxMap::to_prefab`, `Unmapped`). So what an artist paints in MagicaVoxel is not
what they see, and a palette of 200 colours needs 200 lines of Rust and 200 lines of rig.

Two facts bound the design. **The table is fixed after `World::generate`:** `World::materials`
hands out a shared reference, the GPU rows are written at the first sync and
`GpuWorld::rewrite_materials` rewrites albedo, roughness and opacity but keeps the row count
and the classes (translucent, emissive, opaque) the first sync read, because the mirror's slot
packing and the bake's emitter lists are built from them. **The engine keeps no file format
of its own** (ADR-0055, ADR-0058): the apps keep settings in TOML; `vxe-assets` reads `.vox`,
`.rig` and `.voxtext` with no parsing dependency, the `.rig` in a line grammar of
`keyword value...` statements (`crates/vxe-assets/src/rig.rs`).

The owner asked (2026-11-16) for both halves: materials from the `.vox` palette, and the
table as a data file hot-reloaded in the viewer like the `.rig`, "so artists and the game
define materials without Rust". The engine stays game-agnostic: the loader is an asset
concern, the table stays `vxe-world`'s.

## Decision

**A material table is a text file in the `.rig` line grammar, parsed by `vxe-assets` into
`vxe_world::Material` rows, and every palette entry of a loaded `.vox` becomes a row of the
table unless a `material` line maps it by hand.**

- **The file.** `material <name>` opens a row; the statements that follow until the next
  `material` set its fields: `color #rrggbb` (sRGB, decoded exactly to linear albedo) or
  `albedo r g b` (linear), `roughness`, `emission r g b`, `opacity`, `hardness`,
  `acoustic A T R`, `payload <u64>`, `flags indestructible|translucent|crisp...`. Rows take
  ids in file order from 1; air is row 0 and is not written. A field left out takes
  `Material::named`'s default. The parser names the line of every error, and `encode` writes
  a table back in canonical form, as `Rig::canonical` does.
- **Palette rows.** When a `.vox` is loaded for the world or for an entity, each of its 255
  palette entries (used or not) resolves to a row: the `material <index> <name>` line of its
  `.rig` or structure file if there is one, else a row named by the entry's colour and
  MagicaVoxel material (`vox #8c8780`, `vox #80b0ff glass 0.5`, `vox #ffaa33 emit 1.0 2`),
  shared by every file whose entry reads the same. The row's albedo is the colour decoded to
  linear; a `MATL` of type `_glass` gives `TRANSLUCENT` with opacity `1 - _trans`; `_emit`
  gives `emission = albedo * _emit * 10^_flux * k`, `k` a loader parameter; `_metal` gives
  `roughness = _rough`; `_media` and `_cloud` map to glass. Explicit beats automatic:
  a `material` line, including `material <index> air` and `material <index> skip`, wins.
- **The rows are known before `World::generate`.** An application resolves the files it may
  load (its structures library, the viewer's `.vox`) at start, then generates the world. A
  reload of the table file rewrites the fields of existing rows (`rewrite_materials`, and a
  re-bake of the chunks whose light the change moves); a reload that adds or removes a row,
  or changes a row's class (translucent, emissive or opaque), is reported and takes effect at
  the next start.

## Consequences

### Positive
- What is painted is what is seen: a MagicaVoxel scene with glass, glow and metal loads with
  no rig lines and no Rust, in the viewer and in the world alike.
- A repaint to another palette index shows on the viewer's next reload, since every index
  already has a row.
- One grammar, one parser style, no dependency; the materials file reads like the rig beside it.
- A consumer's whole material set, with its payloads and acoustics, is a file its designers edit.

### Negative
- Up to 255 rows per distinct palette. The brick palette holds 16 entries (`PALETTE_ENTRIES`);
  a brick with more materials is `Wide` (16 units of raw ids), so a structure painted in many
  colours costs mirror bytes. Plan 0083 reads it on the castle.
- A row's class is fixed for the world's life; turning a stone row into glass in the file
  needs a restart. Lifting this means re-packing every brick that holds the row and re-baking
  its chunks; parked (backlog).
- The emission factor `k` is a guess until the owner's window sets it; MagicaVoxel's `_flux`
  is a renderer-relative scale, not a physical unit.
- Row names from colours are stable but opaque (`vox #8c8780`); a rig that wants a readable
  name still writes a `material` line.

## Alternatives considered

### Alternative A: TOML for the materials file

The apps already parse TOML (ADR-0058), and a table is a list of records. It would add the
`toml` dependency to `vxe-assets`, where nothing parses TOML today, and give the rig and the
table two grammars beside each other. The `.rig`'s grammar already covers keyword-value rows
with named blocks, and its error style (a line number and what was expected) is what the
viewer shows. Rejected for one more dependency and a second grammar with no gain the file needs.

### Alternative B: only the palette entries a model uses become rows

Fewer rows (a model uses tens of entries, not 255). Rejected because the viewer's reload could
not show a repaint to a fresh index without a restart, which is the authoring loop ADR-0051
exists for, and because the rows cost 64 B each on the GPU: 255 of them are 16 KB.

### Alternative C: map an unmapped palette entry to the nearest existing row

No new rows, the table stays the consumer's hand-written set. Rejected: the nearest row's
colour is not the painted one, so the artist sees a different castle than they built, and the
match is silent, which the import's "never a guess" rule (Plan 0008) forbids.
