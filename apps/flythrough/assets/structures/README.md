# Structures

MagicaVoxel scenes the fly-through places in the world (Plan 0083). One directory per
structure, `<name>/<name>.vox` (`--structures-dir DIR` scans another directory). Each is a
belt item after the entities, and the generated castle after them: select its slot (the wheel,
a digit in placement mode, or the menu's `structures` group, `choose a structure`), and its
ghost stands where it will go: a coarse copy of the structure (voxel for voxel up to 64 voxels
a side, else one voxel per cube of `k`) inside a marked frame of its box, its near side at the
crosshair's hit and the box growing away from the camera, its base on the lowest ground under
its footprint (`snap ground`, up to 8 x 8 columns; on a slope the uphill side stands in the
cut), or with `snap free` (`F`, the pad's D-pad up) at the crosshair's hit as it is, or 30 m
out in the air; `R` turns it a quarter turn and the
left click (or `E`) places it as logged stamps (a stamp of written air over the box and 1 m around it
when `clear box` is on, then one stamp per placement). `structure scale` (1 to 4) and `snap`
are in the same group. A scale that puts the box with its 1 m margin over the world's edit limit
(1024 voxels a side) is refused: the belt's name row reads it in grey with the reason in red
(`castle x2: 1200 x 1200 x 1000 voxels, over the edit limit of 1024`) and the click says it over
the view; the castle places at scale 1 only. The castle's icon and ghost arrive with its first build, on a worker the first time
its slot is selected (`castle: building` until then). A structure larger than the cave's rooms
stands in its cleared box with the cut rock around it: judge a large structure on the outdoor
hills (`--source outdoor --time 12:00`). At the start also, from the command line:

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
`--vox-clear off`. Every library file's palette rows are added when the world is generated
(the start-up line `structures: N entries, +R rows`), so a recoloured entry needs a restart; a
second file of the same palette adds none. The castle adds no palette row: it takes the
material file's castle rows, added when the scene has none.

| Structure | What it shows |
|---|---|
| `chapel/` | rows +255. A nave with blue glass windows and two lamps, a transept placed twice through a reflected frame, a west tower, and a scaffold on a hidden layer: about 48 x 32 x 40 voxels. `chapel.voxtext` draws the models; `chapel.vox` adds the `MATL`, the layers and the mirror (`structures::chapel_vox`; `SVET_BLESS_CHAPEL=1 cargo nextest run -p flythrough --test structures` writes it) |
