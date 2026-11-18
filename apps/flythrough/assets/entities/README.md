# The entities library

The fly-through's predefined voxel-model entities, one directory per entry:

```
<name>/<name>.voxtext   the source, voxels drawn as text
<name>/<name>.vox       generated from it: cargo run -p vxe-assets --example voxtext -- to-vox <name>.voxtext <name>.vox
<name>/<name>.rig       the parts, their pivots, the palette's materials and the clips
```

| Entry | What it is | Body | Walk stride |
|-------|------------|------|-------------|
| `walker` | A small biped: torso, head, two legs, two arms; clips `idle`, `walk` and `turn` (the head looks left and right) | ground: radius 2, 13 tall, steps 2 (world voxels) | 8 world voxels a cycle |
| `spider` | A large spider: body, abdomen, eight legs each on its own phase, palps and fangs; clips `walk`, `idle` and `rear`; drawn in the showcase's glowing rows | surface: radius 6, reach 1.5 (world voxels) | 12 world voxels a cycle |

View one with `flythrough --source showcase --model apps/flythrough/assets/entities/<name>/<name>.rig --clip <clip>`,
record one with `scripts/record-clip.sh`. `apps/flythrough/tests/entities.rs` checks every
directory here: its voxtext converts to its committed `.vox`, its rig builds against the
showcase's table, and this table names it. How entities work and how to make a new one:
[docs/entities.md](../../../../docs/entities.md).
