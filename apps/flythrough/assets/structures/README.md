# Structures

MagicaVoxel scenes the fly-through places in the cave (Plan 0083). One directory per
structure, `<name>/<name>.vox`:

```sh
flythrough --vox apps/flythrough/assets/structures/chapel/chapel.vox          # ahead of the spawn
flythrough --vox apps/flythrough/assets/structures/chapel/chapel.vox@40,0,10*2 # at a corner, scale 2
```

A `.vox` loads as painted: every palette entry becomes a material row named by its colour and
MagicaVoxel material (`vox #3c6ee6 glass 0.6 0.1`), so glass is translucent, `_emit` glows and
lights the cave through the bake, and `_metal` sets the roughness. Hidden layers and hidden
objects are left out, a mirrored group is placed mirrored, and one `.vox` voxel is one world
voxel (10 cm) unless `*k` scales it (1 to 4). Empty cells leave the world alone; the
structure's box, with 1 m on its sides and above, is carved to air first unless
`--vox-clear off`. The palette's rows are added when the world is generated, so a recoloured
entry needs a restart.

| Structure | What it shows |
|---|---|
| `chapel/` | A nave with blue glass windows and two lamps, a transept placed twice through a reflected frame, a west tower, and a scaffold on a hidden layer: about 48 x 32 x 40 voxels. `chapel.voxtext` draws the models; `chapel.vox` adds the `MATL`, the layers and the mirror (`structures::chapel_vox`; `VXE_BLESS_CHAPEL=1 cargo nextest run -p flythrough --test structures` writes it) |
