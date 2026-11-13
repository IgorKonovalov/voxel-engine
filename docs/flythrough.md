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
| `F5` | Toggle what kills history: edit boxes (default), or edit boxes and A-SVGF's gradient |
| `F6` | Toggle the lighting resolution: half (default) or full |
| `F7` | Toggle the headlamp |
| `F8` | Toggle the lamp's bounce from the probe volume (ADR-0043) |
| `Q` | Cycle the exposure: auto (the preset), auto symmetric, manual (holds the exposure of the moment) |
| `,` | Toggle how a cube face reads the light store: normal (default) or face (ADR-0049) |
| `;` | Toggle the normal the light's filters compare: face (default) or normal |
| `M` | Pulse the ore's emission (a render-side scale) |

### The light queue

| Key | Does |
|-----|------|
| `1` | Cycle the queue's order |
| `2` / `3` | Halve / double the queue's angle weight |
| `4` | Toggle withdrawing the jobs of chunks the camera passed |
| `5` | Cycle the bake budget: the default slice a frame, 150 ms a second, 300 ms a second (each capped at a quarter of the frame) |
| `6` | Toggle the full-first radius: off or `--near`'s |
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
| `--workers N` | The streamer's generation workers, at least 1 [one fewer than the logical cores] |
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
| `--source cave\|level\|showcase` | The world: the streamed noise cave [default], the bounded level, or the showcase cavern (a bounded 72 m hall lit by four ores; the lamp off, light radius 19 and its `hall` shot unless told otherwise; no `--save`, `--replay`, `--vault`, `--autopilot`, `--room` or `--castle`) |
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
| `--upscaler off\|own\|fsr2` | The upscaler [fsr2, or own with `--jitter`, or off at scale 1] |
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
| `--filter on\|off` | The a-trous filter runs [on] |
| `--light-radius N` | Chunks whose emitters light the frame, 0 to 64 [4; 19 on the showcase] |
| `--history edits\|gradient` | What kills the temporal history [edits] |
| `--lighting full\|half` | The lighting passes' resolution [half] |
| `--probe-spacing N` | Voxels between probes, 4 to 64 [16] |
| `--probe-rays N` | Rays each updated probe traces, 1 to 64 [64] |
| `--probe-slices N` | The probes refreshed a frame, one in N: 8, 16 or 32 [8] |
| `--emissive-ore on\|off` | The ore glows and lights the cave [on] |
| `--lamp-toggle N` | Switch the lamp every `N` frames |
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

### Removed flags

Flags from earlier plans (`--steady`, `--sampler`, `--emissive-history` and others from Plan
0057; `--light`, `--preload` and others from Plan 0059) are refused with a message that names
the plan. `--help` lists them.
