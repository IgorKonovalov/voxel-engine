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

### Piece B: the window rehearsal (2026-11-25)

**The fly-through.** `--still PATH` and `--mode-text PATH` (`src/cli/still.rs`, `src/run/still.rs`),
with `--headless` and `--frames` only. The still is `svet_render::debug::read_rgba8` of
`gfx.frame.target()` at the end of `print_exit_readings`, written as a binary PPM (no HUD). The
text: the banner `compose_banner` gives at the first frame (written before that frame's draw),
the mode banner as at its start (`Modes::write_banner(banner_at)`: a headless first frame can
come after the window's 3 s), then at the exit the mode rows and the stats HUD's rows. The dig:
the existing `--dig-at F,..` (the camera ray, the left click's dig) is enough; nothing new. Both
hooks sit outside `draw()`, which is at clippy's 150-line cap. `tests/gpu_still.rs`: 7.1 s on
lavapipe (exclusive perf lock, load 2.8): the closed 12 m room at 64 x 36, 3 frames, 2225 of 2304
pixels differ from the first.

**The script.** `scripts/rehearse-window.mjs` with `node --test scripts/rehearse-window.test.mjs`
(15 tests; the one through the real lib skips where the harness repo is absent, as in CI). A
runner's runs are read by running it with `LIVE_LIST` set: the lib's `run` records label and
command line and `live_end` exits, so shell variables expand as in the window. The lib's
`REHEARSE=<dir>` execs the script (no session lock; the script locks per run, so other lanes'
jobs interleave). The mode-text check asks only flags in `--help`'s mode sections, minus a list
of world and reading flags (`NOT_MODES`); `--named-ok` extends it.

**End to end** (lavapipe, `target/release/flythrough` at d1636aac plus this change, 320 x 180,
20 frames, load 2.5 to 7.8, each run about 9 s held). Through the lib (`live/rehearse-e2e.sh`,
sheet `live/rehearsal-e2e/index.md`), four runs on the level: `field` (`--look field`) against
`base` 31703 pixels (55.0 %); `twin` (base's flags) 0 pixels, **the same-picture line fired**,
exit 1; `dig` (`dig=5`) 18111 pixels (31.4 %), `carved: rock 319, ore 10 (4 chunks, 0.55 ms)`.
A first pass on the showcase at 30 frames: 94.7 % black (the store still baking, lamp off), and
its dig at the spawn carved nothing (the ray reaches no wall), which is why a dig that carves
nothing is now a loud line. A list (`live/rehearse-list.txt`): the cave's dig carved rock 241,
11.4 % of pixels. The level's spawn faces a lamp-lit wall: a bright disc under the near-white
line (240), so 0.0 % near white.
