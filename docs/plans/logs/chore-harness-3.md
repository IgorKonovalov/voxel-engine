# chore-harness-3: readings and resume notes

### Piece A: the inert mark, with independent renders (2026-11-25)

The runtime table (`apps/flythrough/src/inert.rs`, `INERT_WHEN`), the menu mark and notice, CPU
tests in `src/run/inert_tests.rs`; the walk (`tests/gpu_settings_rows.rs`) reads the table.

**What carried state across the walk's reset** (all readings lavapipe, exclusive perf lock).
Not the key history, the light history's store or a moment target: with the exposure manual,
`on, s, on, on, s, ...` under key far ray repeated to the byte (load 4.5).

1. The auto exposure. `Frame::reset_exposure` (`crates/svet-render/src/frame/mod.rs`) sets the
   reset bit, which jumps the adapted level at the end of the next frame
   (`shaders/exposure.wgsl`, the `EX_RESET` branch); that frame's passes use `state.current`
   (`exposure.wgsl`, `used`), which the render before left, and the frame stays in the
   upscaler's history. Under key far ray the same settings twice in a row differed by 1654
   pixels, and by 1097 to 1173 after a penumbra render, though the exposure state at the end
   of each render was the same (0.00367, adapted 5.6177).
2. The history's light lists and keys (`frame/state.rs`, `History::lists`, `listed`,
   `light_key`, `bounce_key`, `light_window`), which `Frame::reset_history` keeps: the
   upscaler's light-change test compares the list `CHANGE_FRAMES` (24) encodes back
   (`frame/encode.rs`, `old_list`), about three of the walk's 9-encode renders. Read by walking
   the rows forward and reversed with only (1) fixed: 87 rows, many counts moved (`field_split`
   7624 forward and 0 reversed, the 7624 two renders after `light_cap` in both orders;
   `lighting.entity_light` 5059 and 0; the defaults after the reversed walk 3617 pixels off).

Fix, test support only: `Frame::restart_as_new` (`#[cfg(feature = "test-support")]`) sets the
history to a new frame's whole, the frame index to 0, and the exposure state, its uniform copy,
the highlight state and the floor's smoothed sky share to a new frame's, with the reset bit;
the exposure state and highlight buffers gain `COPY_DST`. The walk's `render` calls it in place
of `reset_history`, `reset_exposure` and `set_frame_index`. A fresh `Frame` per image also
repeats to the byte (0 pixels, two fresh frames on the dug world) but costs a bake (0.6 s) and
a first render (1.6 s) each.

**The window.** Neither is stale state there: the exposure adapting after a change and the
upscaler's light-change reactivity are the frame working as designed, so no `Live` row needs a
settle for them. Backlog line (the harness chore's entry): whether `reset_exposure` itself
should set the exposure the next frame's passes use.

**Independence, proven.** The test asserts each `live_under` image repeats to the byte after
its row's render (key far ray, the light history) and the defaults before and after the walk.
By hand: the walk forward and reversed (`FIELDS` and `INERT_WHEN` both reversed) gave the same
count for every row, 87 lines identical (load 2.5 and 2.9).

**Rows under their needs** (independent renders, floor 20 pixels):

| row | ray | hist | baked | ray+hist | baked+hist | defaults |
|---|---|---|---|---|---|---|
| penumbra | 18708 | 0 | 0 | 18769 | 0 | 0 |
| key_cap | 0 | 0 | 0 | 0 | 0 | 0 |
| key_history | 0 | 0 | 0 | 0 | 0 | 0 |
| sky_rays | 0 | 224 | 0 | 2124 | 1971 | 0 |
| sky_history | 0 | 10 | 0 | 60 | 20 | 0 |
| sky_down_skip | 0 | 0 | 0 | 0 | 0 | 0 |
| key_far_mix | 0 | 2297 | 0 | 0 | 6887 | 0 |

`key_history` and `key_cap` also 0 under ray with the disc penumbra (the defaults' already)
and ray+hist+disc. Table after: `penumbra` (key far column, live under ray), `sky_rays`,
`sky_history` (live under ray+hist), `key_far_mix` (light history off; it was "key far not
baked", which the readings refute), `metering`. To the walk's list: `key_cap` (Absent: its
test value's 1536 voxels is past every ray in the 128-voxel world), `key_history` (Absent: its
test value's 64 is past the key mean's count, at most 17 in 8 frames with the seed's 8),
`sky_down_skip` (Open), and `lighting.entity_light` and `field_split` (Absent: 0 pixels with
independent renders, as their docs say).

**Time:** the walk 16.0 s on lavapipe (load 2.5 at the lock), fixture and bake 4.9 s. Checks:
quick tier 554/554, `test-changed --clippy`, fmt, the six check gates.

### Piece B notes (not started)

No flag writes a still today. The headless target
(`svet_app::run_headless`) is RENDER_ATTACHMENT only; the frame's output is readable with
`svet_render::debug::read_rgba8(&device, &queue, gfx.frame.target())` (the HUD overlay is not
in it). `png` is a dev-dependency of the fly-through only: write a binary PPM from the binary
and let the node script encode PNG with `node:zlib` (node 26 has `zlib.crc32`). Text: the
mode rows from `Modes::write_rows` and the banner from `Run::compose_banner`. The exit point
is `frame.rs` where `args.frames` ends the run (`print_exit_readings`). Plan 0098's
rehearsals used GPU test rigs (`dig_line::reading::dig_timeline` with `VXE_LINE_PNG`,
`docs/plans/logs/0098.md` "The rehearsal for Phase 18"), not the binary.
