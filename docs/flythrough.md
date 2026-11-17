# The fly-through: every key and flag

The fly-through (`apps/flythrough`) is the engine's testbed. This page lists every key it binds
and every flag it reads. The sources of truth are the code: `flythrough --help` prints one row
per flag from the same table the parser reads (`apps/flythrough/src/cli/table.rs`), the toggle
keys are one table in `apps/flythrough/src/toggles.rs`, and the long-form notes are the
`flythrough::cli` module docs. When this page and `--help` disagree, `--help` is right and this
page is stale.

```sh
cargo run --release -p flythrough                       # the streamed cave
cargo run --release -p flythrough -- --level            # the bounded level
cargo run --release -p flythrough -- --source showcase  # the showcase cavern
cargo run --release -p flythrough -- --help             # every flag
```

Every toggle prints its mode to stdout and shows it in the window: the lighting, look,
upscaler and queue toggles raise a banner for 3 s, and the stats overlay (`F2`) carries
the same rows.

## Keys

### Moving and editing

| Key | Does |
|-----|------|
| Mouse | Look |
| `W` `A` `S` `D` | Fly (or walk, in walk mode) |
| `Space` / `C` | Rise / sink; walking, `Space` jumps, or climbs a ledge with a standable top within 1.2 m |
| `Shift` | Faster |
| Left click | Dig at the crosshair; the edit result prints to the terminal |
| Right click | Fill at the crosshair |
| Scroll | Change the dig radius |
| `G` | Toggle fly and walk mode (bounded worlds only) |
| `B` | Run the carve benchmark in place |
| `P` | Stamp a 16-voxel room with an indestructible shell at the crosshair |
| `R` | Toggle what unsupported rock does after a dig: fall (default) or crumble into rubble (bounded worlds only) |
| `K` | Print the coarse grid and walkable map (bounded worlds only) |
| `I` | With `--castle`: move the camera to the next castle shot |
| `T` | On a `--path ... --path-mode tour` run: resume the path from its nearest point after a key took the camera |
| `Esc` | Quit (on a path run, end it so its report says how) |

### Readings and views

| Key | Does |
|-----|------|
| `F1` | Sample the next frames and print frame-time readings (with `--tester`: the help panel) |
| `F2` | Toggle the stats overlay |
| `F9` | Cycle the view: shaded, normals, hitmask, steps, cracks, albedo |
| `X` | Cycle the cutaway: auto, on, off |
| `L` | Toggle the shadow ray |

### Look and upscaler

| Key | Does |
|-----|------|
| `N` | Cycle the look presets: face, field, trilinear8, gradient27 (the field only in a run started under `--look field`) |
| `V` | Cycle face mode's far normal: `[2, 6]`, `[1, 2]`, off |
| `F10` | Cycle the upscaler: off, own, fsr2 |
| `Z` | Cycle the sharpening: 0.5, 0.25, 0 |
| `.` | Stop and start the `--pan` turn |

### Lighting

| Key | Does |
|-----|------|
| `F3` | Toggle the shadow walk: coarse (default) or full |
| `F4` | Step the a-trous filter's iterations: 4 (default), 2, 1; the lighting row names `filter <n>` |
| `F5` | Toggle what kills history: edit boxes (default), or edit boxes and A-SVGF's gradient |
| `F6` | Toggle the lighting resolution: half (default) or full |
| `F7` | Toggle the headlamp |
| `F8` | Toggle the lamp's bounce from the probe volume (ADR-0043) |
| `Q` | Cycle the exposure: auto (the preset), auto symmetric, manual (holds the exposure of the moment) |
| `,` | Toggle how a cube face reads the light store: normal (default) or face (ADR-0049) |
| `;` | Toggle the normal the light's filters compare: face (default) or normal |
| `Delete` | Toggle the a-trous weights at a pixel younger than 4 frames: luma (all weights, default) or edges (plane and normal only); the lighting row names `young <mode>` |
| `Insert` | Toggle where an entity's emissive light comes from: own (its own sample) or world (the baked light of the world faces around it, default); the lighting row names `entity light <mode>` |
| `M` | Pulse the ore's emission (a render-side scale) |

### Dynamic lights

| Key | Does |
|-----|------|
| `1` | Drop a point light of the picked colour (intensity 600) 2 voxels off the looked-at surface (20 voxels ahead on a miss); the banner names `lights <n>/16`. Sixteen slots, the headlamp's among them while it is on; a drop into a full list prints a line and adds nothing |
| `2` | Toggle carry: the newest light rides 2 m ahead and 0.3 m below the eye until `2` again sets it down |
| `3` | Pick the next colour (white, warm, red, green, blue) and recolour the newest light, in flight or still; later drops and shots take it |
| `4` | Remove the newest light or walker, whichever was added last |
| `5` | Flip the newest light's bounce flag: with `--lamp-bounce on` a flagged light feeds the probe channel; the `lights` row marks each flagged light `*` |
| `6` | Fire a point light of the picked colour (intensity 600) along the view ray at 20 m/s; it stops 2 voxels short of the first surface (or after 40 m) and stays there as a dropped light |

### Walkers

| Key | Does |
|-----|------|
| `` ` `` | Place a walker (a voxel-model entity) on the looked-at surface; the banner names the walker count. A world whose table has no walker rows prints a line and places nothing |
| `F11` | Toggle whether the walkers walk or hold still |
| `F12` | Toggle the walkers' eye lights |
| `PageDown` | The next render profile: floor, portable, performance, high (`--profile`) |
| `Home` | The next world source, rebuilt in process (`--source`'s order: cave, level, showcase, outdoor, and back), with that source's start settings; the window, the frame and the clock stay. A source that refuses the run's other flags is passed over |
| `PageUp` | Under a sky: the time of day an hour on; with `Shift`, an hour back |
| `End` | Under a sky: run or pause the day cycle (`--day-length`) |
| `CapsLock` | On the outdoor hills: the next `--trees` (none, sparse, forest, and back), rebuilt in process as `Home` rebuilds a source; the window, the frame and the clock stay |
| `Left`, `Right` | With `--model`: the clip before and after |
| `Down` | With `--model`: pause or play the clip |

The HUD's entities row reads `entities <n>/64 parts <m>/512 over <k> pool <used>/<total> KB`
(`walk` after it while they walk): the entity pool's bytes in use, rounded up to whole KB,
against its size. A `--model` reload frees the replaced model, so the figure holds still over
saves that keep the model's size. When an upload is refused although the pool's free bytes
would hold it (no hole is large enough), the pool is cleared and every placed model uploaded
again, and the banner reads `entity pool reset`; a `--model` model refused after that keeps
its error on screen in red until a reload builds.

### The light queue

| Key | Does |
|-----|------|
| `F` | Cycle the queue's order |
| `Tab` / `Slash` | Halve / double the queue's angle weight |
| `Quote` | Toggle withdrawing the jobs of chunks the camera passed |
| `Backspace` | Cycle the bake budget: the default slice a frame, 150 ms a second, 300 ms a second (each capped at a quarter of the frame) |
| `Backslash` | Toggle the full-first radius: off or `--near`'s |
| `7` | Cycle what a face with no baked value reads: black, neighbour, parent, chain (ADR-0053) |
| `8` | Toggle the light reach: full or partial |

### Audio (with `--audio`)

| Key | Does |
|-----|------|
| `H` | Play a knock at the point the last left click dug |
| `J` | Play a looping hum there |
| `T` | Play a looping pulse there (a knock each second) |
| `E` | Toggle the early reflections |
| `Y` | Toggle the late reverb tail |
| `[` / `]` | Trim the echoes down / up 3 dB |
| `-` / `=` | Trim the tail down / up 3 dB |
| `9` / `0` | Shorten / lengthen the tail's pre-delay by 10 ms |
| `O` | Toggle the tail's glide |
| `U` | Toggle the direct path's propagation delay |

### Gamepad (`--pad on`, the default in a window)

| Input | Does |
|-------|------|
| Left stick | Move, at a speed set by its deflection |
| Right stick | Look |
| `L1` / `R1` | Sink / rise |
| `R3` | Latch boost |
| `South` | Walking: jump |
| `R2` / `L2` | Dig / fill |
| D-pad up / down | Step the dig radius |
| `North` | Toggle walk mode |
| `West` | Cycle the cutaway |
| `East` | Knock (with `--audio`) |
| `Start` | Toggle the stats overlay |

### Tester mode (`--tester`)

Only movement, look, the left click's dig, `F1` (a help panel), `T` and `Esc` act. Every other
key is off, the stats overlay among them.

## Flags

Grouped as `--help` groups them. A default in brackets is the code's.

### Run

| Flag | Does |
|------|------|
| `--help`, `-h` | Print every flag and exit |
| `--check-args` | Parse the arguments, print `arguments ok` and exit, opening nothing |
| `--seed N` | The world's seed [1] |
| `--render WxH` | Fix the frame's output resolution; the window scales it |
| `--frames N` | Exit after `N` frames, printing the `F1` readings over the last ones |
| `--pace free\|compositor` | Wait for the compositor's frame callback before each present or not [free] |
| `--bench N` | Send `N` `B` benchmark inputs on the first frame |
| `--dig-at F,..` | Dig at each of these frames |
| `--bench-at F,..` | A bench input at each of these frames |
| `--sync-budget MIB` | The mirror's upload bytes a frame |
| `--sync-ms MS` | The mirror's upload time a frame [none; 0 is none] |
| `--workers N` | The streamer's generation workers, at least 1 [half the logical cores] |
| `--residency R_FULL,R_LOD` | The streamed full and LOD radii in chunks, hysteresis 1 [16,24] |
| `--mirror-budget MIB` | The VRAM the mirror reserves at startup [3072 streamed; on the level what it needs plus dig headroom]. While streaming, the full radius drops a chunk after 30 frames with the mirror's pressure over 0.9 and returns after 300 frames under 0.7, down to 6 |

### Session

| Flag | Does |
|------|------|
| `--autopilot` | Dig, fly 300 m out and back through the streamed cave, print the streaming readings |
| `--headless` | Render the same frames offscreen, with no window |
| `--adapter SUBSTRING` | Use the first adapter whose name contains it |
| `--hud` | Start with the stats overlay shown |
| `--census` | Print the brick census on exit |
| `--save PATH` | Write the session's log file on exit and print the world's hash (with `--level`) |
| `--replay PATH` | Re-simulate a saved session before the first frame |
| `--logic native\|wasm[=PATH]` | The server's logic: native [default], or a component (needs `--features wasm` and `scripts/build-guests.sh`) |
| `--path cave\|showcase\|castle` | Fly that scene's camera path (the scene's own flags and seed 1 required) |
| `--path-mode bench\|tour` | Fly the path once by wall time with the lamp off, then exit [bench]; or loop it |
| `--report DIR` | Write `DIR/run.json` at exit (with `--path`) |
| `--tester` | Only movement, look, the dig, `F1`, `T` and `Esc` act |
| `--pad on\|off` | Read a gamepad in a window [on; never headless or under the autopilot] |

### World

| Flag | Does |
|------|------|
| `--source cave\|level\|showcase\|outdoor` | The world: the streamed noise cave [default], the bounded level, the showcase cavern (a bounded 72 m hall lit by four ores; the lamp off, light radius 19 and its `hall` shot unless told otherwise; no `--save`, `--replay`, `--vault`, `--autopilot`, `--room` or `--castle`), or the streamed outdoor hills with a cave mouth under a sky (no `--level`, `--castle`, `--vault`, `--room`, a rig, `--save` or `--replay`). The banner's line starts `source NAME` |
| `--spawn outside\|inside\|forest` | Where `--source outdoor` starts: 30 m before the cave mouth facing it, 2 m up [default], 3 m inside the tunnel facing out, or 64 m north of the mouth facing north, 2 m up, 10 degrees down |
| `--trees none\|sparse\|forest` | Trees on the outdoor hills: about 9 % or 94 % of the 48-voxel cells of a 3072-voxel box between the mouth and the forest spawn; the banner's source reads `outdoor, trees NAME` [sparse] |
| `--leaf-fill N` | The canopy voxels kept, of 256, 1 to 256 [64] |
| `--leaf-shadow brick\|voxel` | How a canopy shadows the sun and the sky: as its 8^3 bricks (a light's shadow ray stops where it enters a brick holding leaves, so no fleck shows through) or voxel by voxel; the banner reads `leaf voxel` with a sky set and names nothing for `brick` [brick] |
| `--sun EL,AZ` | Fix the sun at this elevation and azimuth in degrees (clockwise from north, 180 south), the moon opposite; `PageUp` and `End` leave it, and the banner reads `sun EL/AZ fixed` in place of the time |
| `--time HH:MM` | The time of day the sky starts at, latitude 45 at an equinox [12:00]; the banner names it while a sky is set |
| `--day-length S` | Seconds for a whole day while the cycle runs (`End`) [1200] |
| `--sun-radius DEG` | The sun's angular radius, 0.05 to 10 degrees: a wider disc softens every sun shadow (the real sun's is 0.2665) [3] |
| `--moon-radius DEG` | The moon's angular radius, 0.05 to 10 degrees: its disc in the sky and its shadows' softness [3] |
| `--sky-rays N` | Cosine-weighted sky rays a lighting pixel casts where no baked sky visibility is fresh, 0 (bake only) to 4; the banner names the count [1] |
| `--sky-history N` | Frames the sky rays' running mean (the visibility pair) holds at most, 1 to 1024 [64] |
| `--sky on\|off` | A sky over the world [on for `--source outdoor`, else off] |
| `--switch-at F,..` | Switch the source at these frames, as `Home` does, and print the spawn's hash at the end |
| `--spawn-hash` | Print the voxel hash of the chunks within 2 of the spawn's chunk at the end |
| `--level` | The bounded level (as `--source level`) |
| `--thin-rig` | A bounded scene of thin features for the field look |
| `--seam-rig` | A bounded scene of seams for the field look |
| `--room METRES` | A closed cube of air of that edge, 1 to 100 m |
| `--shot hall\|chasm\|crystals` | With the showcase: the shot the camera starts at [hall] |
| `--pose vein\|patch` | With the level: the settle rig's still camera |
| `--walk` | Start in walk mode (on a bounded world) |
| `--glide V` | Move along the heading at `V` voxels a second |
| `--vault` | A 16-voxel room with an indestructible shell 10 m along +x from the spawn |
| `--castle` | A generated castle in the streamed cave's hall |
| `--castle-out PATH` | Also write the castle's `.vox` (implies `--castle`) |
| `--castle-shot gate\|nave\|window\|dungeon` | The castle pose to start at (implies `--castle`) |
| `--crisp none\|rock\|all` | The table rows marked crisp, which the field look draws as cubes [none] |

### Look

| Flag | Does |
|------|------|
| `--look PRESET` | The starting preset: `face` (honest cubes with the far normal; named here, with it off), `field` (the smooth surface, opt-in), `trilinear8`, `gradient27` [face] |
| `--face-mix K` | The preset's face mix, 0 to 0.5 |
| `--far-normal LO,HI` | Face mode's far normal band in pixels a voxel [2,6] |
| `--band-census` | Print the field surface's band census |
| `--view VIEW` | `shaded`, `normals`, `hitmask`, `steps`, `cracks` or `albedo` [shaded] |
| `--translucent-layers N` | Translucent voxels composited before the hit [4; 0 ignores them] |
| `--cutaway auto\|on\|off` | The cutaway view's start state [auto: on while the camera is in rock] |
| `--internal-scale 1\|1.5\|2` | The output size over the internal size [1.5] |
| `--jitter on\|off` | The primary rays' sub-pixel jitter (selects the `own` upscaler) |
| `--sharpen S` | The upscaler's sharpening, 0 to 1 [0.5] |
| `--upscaler off\|own\|fsr2` | The upscaler [fsr2, or own with `--jitter`, or off at scale 1 but under `--profile high`] |
| `--pan DEG_PER_S` | Turn the camera at a constant yaw rate (`.` stops and starts it) |
| `--jitter-phases N` | FSR 2's jitter phase count, 1 to 1024 |
| `--dither on\|off` | FSR 2's static one-level dither [off] |

### Lighting

| Flag | Does |
|------|------|
| `--lamp on\|off` | The headlamp [on; off on the showcase and a `--path` benchmark] |
| `--lamp-bounce on\|off` (alias `--indirect`) | The lamp's bounce from the probe volume, ADR-0043 [on] |
| `--shadow-walk full\|coarse` | The walk shadow rays take [coarse] |
| `--store-read face\|normal` | How a cube face reads the store, ADR-0049 [normal] |
| `--light-weights normal\|face` | The normal the light's filters compare [face] |
| `--young-filter luma\|edges` | The a-trous weights at a pixel younger than 4 frames: all, or plane and normal only (`Delete`) [luma] |
| `--entity-light own\|world` | An entity's emissive light: its own sample, or the baked light of the world faces around it (`Insert`) [world] |
| `--filter on\|off` | The a-trous filter runs [on] |
| `--light-radius N` | Chunks whose emitters light the frame, 0 to 64 [4; 19 on the showcase] |
| `--history edits\|gradient` | What kills the temporal history [edits] |
| `--lighting full\|half` | The lighting passes' resolution [half] |
| `--probe-spacing N` | Voxels between probes, 4 to 64 [16] |
| `--probe-rays N` | Rays each updated probe traces, 1 to 64 [64] |
| `--probe-slices N` | The probes refreshed a frame, one in N: 8, 16 or 32 [8] |
| `--emissive-ore on\|off` | The ore glows and lights the cave [on] |
| `--lamp-toggle N` | Switch the lamp every `N` frames |
| `--entities N` | Start with `N` walkers (voxel-model entities) on a 3 m ring around the spawn [0] |
| `--entities-walk on\|off` | The walkers start walking, as `F11` sets them [off] |
| `--model PATH` | View a `.rig` and the `.vox` it names (the library is `apps/flythrough/assets/entities/<name>/<name>.rig`, [entities.md](entities.md)): `` ` `` and `--entities N` place it in place of the walker; a saved change to either file reloads |
| `--clip NAME` | The `--model` clip to play first [the rig's first] |
| `--fade-reading` | Headless: dig, settle, switch the lamp off and print how the light fades |
| `--exposure auto\|E` | Adapt like an eye, or a fixed exposure [auto] |
| `--adaptation eye\|symmetric\|instant` | The auto exposure's preset [eye] |
| `--adapt LIGHT,CONE,ROD,FLOOR` | The preset's time constants and cone floor |
| `--metering centre\|average` | How the meter weights the frame |

### Light store and queue

| Flag | Does |
|------|------|
| `--bake-slice MS` | The queue's slice a frame, 0.1 to 100 ms [1.5] |
| `--bake-budget frame:MS\|second:MS,CAP` | The queue's budget, ADR-0052 |
| `--coarse N` | The coarse pass's samples a brick direction, 0 to 4096 [256] |
| `--near R` | Chunks baked in full before the first frame, 0 to 8 [2] |
| `--full-first off\|N` | The queue's full-first radius after the pre-roll [`--near`'s] |
| `--missing-light black\|neighbour\|parent\|chain` | What a face with no baked value reads, ADR-0053 [chain] |
| `--reach full\|partial` | Whether a chunk waits for its whole light reach [full] |
| `--queue-order score\|distance` | The live queue's order |
| `--queue-weights A,R\|A,tier` | The score's angle and refine weights |
| `--queue-withdraw on\|off` | Withdraw the jobs of chunks the camera passed |
| `--queue-census` | Read the queue's view census at every opening |
| `--edit-ms MS` | The slice's growth while a dig waits, 0 to 100 ms [3] |
| `--dig-reach on\|off` | A dig re-bakes the chunks within the light radius [on] |
| `--read-missing` | Read the store's no-value count while a dig waits |

### Audio

| Flag | Does |
|------|------|
| `--audio` | Open the default output device |
| `--audio-debug` | `--audio` plus the callback's readings and a recording |
| `--room-line` | Print the room line whenever a fan completes |
| `--absorption A` | The cave row's absorption per metre, 0 to 1 |
| `--mic` | Play the input device at the listener (implies `--audio`) |
| `--echo-db DB` | The echoes' trim |
| `--tail-db DB` | The tail's trim |
| `--predelay-ms MS` | The tail's pre-delay |
| `--tail-glide` | The tail's lines glide instead of handing over |
| `--flat-mix` | Both room curves flat at 0 dB |
| `--direct-delay` | The direct path's propagation delay |

### Loading

| Flag | Does |
|------|------|
| `--loading-screen on\|off` | Step the start-up (the entity pipelines, the level's upload or the streamed near residency, the light pre-roll, the field's fill) from the frame loop behind loading frames: a turning voxel cube, a bar that only moves forward, the scene, the stage and its count; off runs it all before the first frame [on in a window, off headless] |
| `--loading-fade MS` | Fade the loading frame out over the first world frames; 0 cuts [250] |

With the screen on, the window's first frame is a still loading frame (the background, the
scene and an empty bar), drawn as soon as the device exists and before the frame and the
mirror are created; the turning loading frames follow. The castle's prefabs build beside the
window and the device, not before them.

`Esc` or the window's close during loading ends the run through the ordinary teardown; a
benchmark so ended reports `ended: "aborted"` with 0 frames. Under `--tester` the loading
frame's title is the path's scene and mode (`castle - benchmark`). `run.json` is schema 2:
`load_ms` (device ready to the first world frame) and `load_stages` (`[{name, ms}]`, each
stage's step time).

### Render settings

| Flag | Does |
|------|------|
| `--settings` | Print every render setting the flags give (`vxe_render::RenderSettings`, ADR-0055), one line per field: its name, its apply class (`live`, `settle`, `rebuild`) and its value; then exit |
| `--profile floor\|portable\|performance\|high` | The render profile (`vxe_render::Profile`) the other flags edit, whatever their order: floor (1080p, FSR 2 at 1.5), portable (720p, FSR 2 at 1.5, fewer layers and lights, no lamp bounce, a smaller residency), performance (the window's size, FSR 2 at 2) or high (1080p, FSR 2 at 1; not for the slow adapter); its output hint sizes a headless run with no `--render`. The HUD's first mode row reads `profile: portable +1` with one field edited [floor] |
| `--light-cap N` | Slots of the list of lights shaded, the headlamp's included, 1 to 16 [the profile's] |

`PageDown` cycles the profiles in the window: the next one's translucent layers, lamp bounce,
light cap, internal scale and bake budget apply from the next frame, and the profile row names
the fields that wait for a restart with `--profile` (`restart for: lod.r_full,
build.mirror_budget_bytes, build.residency`). `F4` takes the cycle once the menu frees it
(ADR-0061).

### Diagnostics

| Flag | Does |
|------|------|
| `--load-slice MS` | The start-up step time a loading frame spends before it draws [25] |
| `--quit-at-loading-frame N` | Act as `Esc` at loading frame `N`, a test's way to end a run while it loads |

### Removed flags

Flags from earlier plans (`--steady`, `--sampler`, `--emissive-history` and others from Plan
0057; `--light`, `--preload` and others from Plan 0059) are refused with a message that names
the plan. `--help` lists them.

## Recording clips

Two scripts turn a model's clip into a GIF or MP4 for articles and the README. The models and
how to make one are in [entities.md](entities.md).

`scripts/record-clip.sh` renders headlessly: it builds the fly-through's `clip` example
(release, in a 6G memory-capped scope), renders numbered PNGs under the GPU lock into
`target/clip/frames/<rig>-<clip>/`, and encodes them with ffmpeg. Frame `n` shows the clip at
tick `n * rate / fps`, so a recording is the same on every run.

```sh
scripts/record-clip.sh                                    # the spider's first clip, a GIF
scripts/record-clip.sh --format both --width 800 -- --clip rear
scripts/record-clip.sh -- --rig apps/flythrough/assets/entities/walker/walker.rig \
  --clip walk --ahead 70 --orbit 24,3,-10 --aim 9          # the walker, close up
scripts/record-clip.sh --input target/clip/frames/spider-walk --crop 960:540:160:90
```

| Option | Does |
|--------|------|
| `--out BASE` | The outputs' path without extension [`target/clip/<rig>-<clip>`] |
| `--format gif\|mp4\|both` | A looping GIF (two-pass palette, ordered dither), an H.264 MP4 (yuv420p, faststart), or both [gif] |
| `--width W` | The encoded width, the height in proportion [640] |
| `--fps N` | Frames a second, rendered and encoded [25] |
| `--crop W:H:X:Y` | Crop the frames, in rendered pixels, before scaling |
| `--colors N` | The GIF palette's colours [192] |
| `--crf N` | The MP4's quality, lower is better [20] |
| `--input PATH` | Encode a directory of `frame-*.png` or a video instead of rendering |
| `--no-build` | Run the example as already built |

Everything after `--` goes to the example; `cargo run --release -p flythrough --example clip --
--help` lists its flags. The ones that frame a recording:

| Flag | Does |
|------|------|
| `--rig PATH`, `--clip NAME` | The model and its clip [the spider; the rig's first clip] |
| `--source`, `--shot`, `--seed` | The world, as the hero's [the showcase's `hall`] |
| `--place X,Y,Z` | The model's feet voxel [the floor `--ahead D` voxels (80) along the shot's heading] |
| `--facing DEG` | The model's yaw from facing -y [towards the shot's eye] |
| `--orbit DIST,HEIGHT,ANGLE` | The camera `DIST` voxels from the feet across the floor, `HEIGHT` over them, `ANGLE` degrees round from the model's front [60,12,10] |
| `--aim H` | The orbit looks at the point `H` voxels over the feet [14] |
| `--at X,Y,Z`, `--yaw`, `--pitch` | A fixed camera instead of the orbit |
| `--exposure auto\|E` | The exposure [a manual 10] |
| `--size WxH` | The rendered size [1280x720] |
| `--loops N`, `--seconds S` | One or more passes of the clip, or a time [1 loop] |
| `--settle N`, `--warmup N` | Frames drawn at each pose before it is kept, and before the first [16, 64] |

A loop is seamless when its ticks fill whole frames (`ticks * fps / rate` whole); the example
says so when they do not. The lighting is the hero's: lamp and bounce off, probes at 32.

`scripts/record-screen.sh` is the fallback for a scene that needs hands on the controls: start
the fly-through in a window yourself, then run it. It counts down in desktop notifications that
expire after 1.5 s, the last at `--quiet` seconds (3) before the recording starts so that none
is on screen at its first frame, dismisses any others (`makoctl`, when installed), records a
monitor with `gpu-screen-recorder` for `--seconds` (10), stops it with SIGINT and hands the
video to `record-clip.sh --input`. It never closes, moves or focuses a window. `--monitor NAME`
picks the monitor (the first `gpu-screen-recorder --list-capture-options` names by default),
`--countdown N` the countdown with the quiet gap in it (6), `--keep-raw` keeps the recording,
`--dry-run` prints the schedule and the commands and records nothing, and `--format`,
`--width`, `--fps`, `--crop` (in screen pixels, `W:H:X:Y`), `--colors` and `--crf` go to the
encode.
